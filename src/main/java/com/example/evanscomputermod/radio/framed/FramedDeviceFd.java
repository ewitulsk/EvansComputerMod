package com.example.evanscomputermod.radio.framed;

import com.example.evanscomputermod.computer.wasi.DeviceFd;
import com.example.evanscomputermod.computer.wasi.PipeFd;
import com.example.evanscomputermod.computer.wasi.Pollable;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.function.Supplier;

/**
 * {@code /dev/sdr<N>.frames}, {@code /dev/sdr.frames}, {@code /dev/frames.<attachment>}:
 * a framed radio's frames as {@link FramedRecords}. Write TX / BEACON records (whole
 * records, several per write allowed); read RX and STATUS records (whole records,
 * as many as fit). A blocking read waits up to 2 s for one and then returns 0 bytes;
 * a non-blocking read with nothing waiting fails with EAGAIN. Pollable with
 * {@code poll_oneoff} and {@code sock_poll} next to sockets.
 */
public final class FramedDeviceFd implements DeviceFd, Pollable {

    private final Supplier<FramedRadio> radio;
    private volatile boolean nonBlocking;

    /** {@code radio} returns the radio, or null while the host is not in framed mode. */
    public FramedDeviceFd(Supplier<FramedRadio> radio) {
        this.radio = radio;
    }

    private FramedRadio live() throws ErrnoException {
        FramedRadio r = radio.get();
        if (r == null || !r.active()) throw new ErrnoException(ErrnoException.EIO, "framed mode is off (set_mode('framed') or 'mode framed' in sdrctl)");
        return r;
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        FramedRadio r = live();
        long deadline = System.nanoTime() + 2_000_000_000L;
        Object waiter = new Object();
        r.addWaiter(waiter);
        try {
            while (true) {
                r.pump();
                byte[] got;
                try {
                    got = r.take(length);
                } catch (IllegalArgumentException e) {
                    throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
                }
                if (got != null) {
                    System.arraycopy(got, 0, buf, offset, got.length);
                    return got.length;
                }
                if (nonBlocking) return PipeFd.WOULD_BLOCK;
                if (Thread.currentThread().isInterrupted() || System.nanoTime() > deadline || !r.active()) return 0;
                synchronized (waiter) {
                    try {
                        waiter.wait(10);
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        throw new RuntimeException("frames read interrupted");
                    }
                }
            }
        } finally {
            r.removeWaiter(waiter);
        }
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        FramedRadio r = live();
        try {
            r.write(data, offset, length);
        } catch (IllegalArgumentException | IllegalStateException e) {
            throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
        }
        return length;
    }

    // ---- Pollable

    @Override
    public boolean readReady() {
        FramedRadio r = radio.get();
        return r == null || !r.active() || r.readReady();   // off: a read reports it
    }

    @Override
    public int readable() {
        FramedRadio r = radio.get();
        return r == null ? 0 : r.available();
    }

    @Override
    public boolean hungUp() {
        return false;
    }

    @Override
    public void addWaiter(Object waiter) {
        FramedRadio r = radio.get();
        if (r != null) r.addWaiter(waiter);
    }

    @Override
    public void removeWaiter(Object waiter) {
        FramedRadio r = radio.get();
        if (r != null) r.removeWaiter(waiter);
    }

    @Override public void close() {}
    @Override public boolean isReadable() { return true; }
    @Override public boolean isWritable() { return true; }
    @Override public boolean isNonBlocking() { return nonBlocking; }
    @Override public void setNonBlocking(boolean nb) { nonBlocking = nb; }

    /**
     * {@code /dev/framesctl.<attachment>}: the framed radio's control file for hosts
     * without an {@code sdrctl} (modem modules; SDRs accept the same lines in
     * {@code /dev/sdrctl}). Write {@link FramedRadio#control} lines, read {@code key value} status.
     */
    public static final class Ctl implements DeviceFd {
        private final FramedRadio radio;
        private byte[] snapshot;
        private int pos;
        private final StringBuilder partial = new StringBuilder();
        private volatile boolean nonBlocking;

        public Ctl(FramedRadio radio) {
            this.radio = radio;
        }

        @Override
        public synchronized int read(byte[] buf, int offset, int length) {
            if (snapshot == null) {
                snapshot = radio.describe().getBytes(StandardCharsets.UTF_8);
                pos = 0;
            }
            int n = Math.min(length, snapshot.length - pos);
            if (n <= 0) {
                snapshot = null;
                return 0;
            }
            System.arraycopy(snapshot, pos, buf, offset, n);
            pos += n;
            return n;
        }

        @Override
        public synchronized int write(byte[] data, int offset, int length) throws IOException {
            partial.append(new String(data, offset, length, StandardCharsets.UTF_8));
            int nl;
            while ((nl = partial.indexOf("\n")) >= 0) {
                String line = partial.substring(0, nl).trim();
                partial.delete(0, nl + 1);
                if (line.isEmpty()) continue;
                try {
                    if (line.startsWith("mode ")) {   // a framed-only radio: accept 'mode framed' so tools can send it to any host
                        if (!line.substring(5).trim().equals("framed")) throw new IllegalArgumentException("this radio has framed mode only");
                        continue;
                    }
                    if (!radio.control(line.split("\\s+"))) throw new IllegalArgumentException("unknown framed command: " + line);
                } catch (RuntimeException e) {
                    throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
                }
            }
            return length;
        }

        @Override public void close() {}
        @Override public boolean isReadable() { return true; }
        @Override public boolean isWritable() { return true; }
        @Override public boolean isNonBlocking() { return nonBlocking; }
        @Override public void setNonBlocking(boolean nb) { nonBlocking = nb; }
    }
}
