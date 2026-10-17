package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.ui.proto.UiProtoException;

import java.util.ArrayDeque;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * One UI connection (one {@code ui_open}, §3): the guest-facing half of the
 * protocol. Program threads call {@link #send}, {@link #recv},
 * {@link #putPixels} and {@link #upload}; protocol state lives in the
 * {@link Compositor}, which handles each decoded message under its lock and
 * queues replies here. A protocol error queues {@code Error}, closes the
 * connection and drops its surfaces; the program itself keeps running.
 */
public final class UiConnection {
    /** Host-call results (§3, abi/child-abi.toml). */
    public static final int OK = 0, E_INVALID = -1, E_BUSY = -2, E_NO_DEVICE = -4, E_INTERRUPTED = -5, E_NOT_PERMITTED = -6;

    final Compositor comp;
    final int id;
    final int pid;
    final UiProto.Role role;
    String appId;
    boolean hello;
    /** Protocol version the client said hello with (1 or 2). */
    int version = 1;
    volatile boolean closed;
    /** Why it closed ({@code null}: closed by the program or the compositor going away). */
    volatile UiError closeReason;

    final Map<Integer, Surface> surfaces = new LinkedHashMap<>();
    final Map<Integer, UiResource> resources = new LinkedHashMap<>();
    int nodes;
    long resourceBytes;
    /** Tree-op budget (token bucket, bytes) and when it was last refilled. */
    double opTokens;
    long opRefillMs;

    /** Legacy windows: a connection made by the compositor itself for a gfx_init2 program. */
    final LegacySink legacy;

    private final ArrayDeque<byte[]> out = new ArrayDeque<>();
    private long queuedBytes;
    /** When the oldest unread message was queued (0 = nothing queued). */
    private volatile long oldestQueuedMs;
    private final Set<Object> waiters = new CopyOnWriteArraySet<>();

    /** Input for a legacy window goes straight to the program instead of the message queue. */
    public interface LegacySink {
        /** Protocol {@code PointerKind} / {@code Button} codes; x, y in the program's framebuffer pixels. */
        void pointer(int kind, int x, int y, int buttons, int button, int scroll);

        void key(int hid, int mods, boolean down);

        /** The shell (or player) closed the window. */
        default void close() {}
    }

    UiConnection(Compositor comp, int id, int pid, UiProto.Role role, LegacySink legacy) {
        this.comp = comp;
        this.id = id;
        this.pid = pid;
        this.role = role;
        this.legacy = legacy;
    }

    public int pid() {
        return pid;
    }

    public boolean isClosed() {
        return closed;
    }

    public boolean isShell() {
        return role == UiProto.Role.SHELL;
    }

    public UiError closeReason() {
        return closeReason;
    }

    // ------------------------------------------------------------ program side

    /** {@code ui_send}: one or more whole messages. 0, or {@link #E_INVALID} (the connection is then closed). */
    public int send(byte[] data) {
        if (closed) return E_INVALID;
        List<UiProto.Message> msgs;
        try {
            msgs = UiCodec.decodeAll(data, 0, data.length);
        } catch (UiProtoException e) {
            String why = e.kind() == UiProtoException.Kind.INCOMPLETE ? "partial message in ui_send" : e.getMessage();
            comp.fail(this, new UiError(e.kind() == UiProtoException.Kind.INCOMPLETE ? UiProto.ErrorCode.MALFORMED
                    : e.errorCode(), why));
            return E_INVALID;
        }
        return comp.handleAll(this, msgs, data.length) ? OK : E_INVALID;
    }

    /**
     * {@code ui_recv}: whole messages up to {@code cap} bytes, waiting up to
     * {@code timeoutMs} (negative: forever). Returns the bytes (empty on a
     * timeout), null once the connection is closed and drained, or throws
     * {@link IllegalArgumentException} when the next message is larger than
     * {@code cap}. A thread interrupt (the program is killed) throws.
     */
    public byte[] recv(int cap, long timeoutMs) {
        long deadline = timeoutMs < 0 ? Long.MAX_VALUE : System.nanoTime() + timeoutMs * 1_000_000L;
        synchronized (this) {
            while (out.isEmpty() && !closed) {
                long wait = deadline == Long.MAX_VALUE ? 0 : (deadline - System.nanoTime()) / 1_000_000L;
                if (deadline != Long.MAX_VALUE && wait <= 0) return new byte[0];
                try {
                    wait(deadline == Long.MAX_VALUE ? 1000 : Math.max(1, wait));
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    throw new RuntimeException("ui_recv interrupted");
                }
                if (Thread.currentThread().isInterrupted()) throw new RuntimeException("ui_recv interrupted");
            }
            if (out.isEmpty()) return null;
            if (out.peekFirst().length > cap) throw new IllegalArgumentException("buffer too small");
            int n = 0;
            for (byte[] m : out) {
                if (n + m.length > cap) break;
                n += m.length;
            }
            byte[] buf = new byte[n];
            int off = 0;
            while (off < n) {
                byte[] m = out.pollFirst();
                System.arraycopy(m, 0, buf, off, m.length);
                off += m.length;
            }
            queuedBytes -= n;
            oldestQueuedMs = out.isEmpty() ? 0 : comp.host.nowMs();
            return buf;
        }
    }

    /** Bytes waiting to be received (for {@code poll_oneoff} / {@code sock_poll}). */
    public synchronized int readable() {
        return (int) Math.min(Integer.MAX_VALUE, queuedBytes);
    }

    /** Ready to read: messages waiting, or closed (reads then report it). */
    public synchronized boolean readReady() {
        return !out.isEmpty() || closed;
    }

    /** A poll waiter is notified (notifyAll on it) whenever this connection becomes readable or closes. */
    public void addWaiter(Object w) {
        waiters.add(w);
    }

    public void removeWaiter(Object w) {
        waiters.remove(w);
    }

    /** How long the oldest unread message has waited, in ms (0 if none). */
    long unreadAgeMs(long now) {
        long t = oldestQueuedMs;
        return t == 0 ? 0 : now - t;
    }

    /** Reads {@code n} bytes at {@code offset} of the program's buffer into {@code dst}. */
    public interface PixelSource {
        void read(int offset, byte[] dst, int dstOff, int n);
    }

    /**
     * {@code ui_put_pixels}: rows for a pixel surface or canvas. {@code len} is
     * the program's buffer size (a whole target in its format); {@code rects}
     * are {@code x, y, w, h} quadruples, none meaning the whole target. Only the
     * damaged rows are copied out of the program.
     */
    public int putPixels(int target, int len, int[] rects, PixelSource src) {
        if (closed) return E_INVALID;
        return comp.putPixels(this, target, len, rects, src) ? OK : E_INVALID;
    }

    /** {@code ui_upload}: an image's data (kind 0 PNG, 1 raw RGBA). */
    public int upload(int resource, int kind, byte[] data) {
        if (closed) return E_INVALID;
        return comp.upload(this, resource, kind, data) ? OK : E_INVALID;
    }

    /** The program closed its fd (or exited): windows go away without an error. */
    public void close() {
        comp.disconnect(this, null);
    }

    // ------------------------------------------------------------ compositor side

    /** Queue a message for the program (compositor lock held). */
    void queue(UiProto.Message m) {
        if (closed || legacy != null) return;
        byte[] b = UiCodec.encode(m);
        boolean overflow;
        synchronized (this) {
            if (out.isEmpty()) oldestQueuedMs = comp.host.nowMs();
            out.addLast(b);
            queuedBytes += b.length;
            overflow = queuedBytes > comp.limits.maxQueuedBytes();
            notifyAll();
        }
        wakeWaiters();
        if (overflow) comp.failLater(this, UiError.limit("more than " + comp.limits.maxQueuedBytes() + " bytes left unread"));
    }

    /** Mark closed after queueing a final message (compositor lock held). */
    void markClosed(UiError reason) {
        synchronized (this) {
            if (reason != null && legacy == null) {
                byte[] b = UiCodec.encode(new UiProto.Error(reason.code, truncate(reason.getMessage())));
                out.addLast(b);
                queuedBytes += b.length;
            }
            closeReason = reason;
            closed = true;
            notifyAll();
        }
        wakeWaiters();
    }

    private void wakeWaiters() {
        for (Object w : waiters) {
            synchronized (w) {
                w.notifyAll();
            }
        }
    }

    private static String truncate(String s) {
        if (s == null) return "";
        return s.length() > 300 ? s.substring(0, 300) : s;
    }
}
