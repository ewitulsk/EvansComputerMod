package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.wasm.WasmExport;
import com.example.evanscomputermod.api.wasm.WasmMemory;

import java.util.Iterator;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.ExecutionException;

/**
 * Bridge between child WASI processes (which make blocking socket calls)
 * and the kernel's non-blocking socket syscalls.
 *
 * <p>A child thread enqueues a request and waits. The kernel worker thread
 * dispatches it through the kernel's {@code handle_sock_ipc} export. If the
 * kernel answers {@link #IPC_PENDING} (e.g. recv with no data yet), the
 * request stays queued and is retried after the next network event, tick
 * or new request -- the kernel never blocks, and the kernel owns every
 * socket timeout (SO_RCVTIMEO, connect, DNS), so the child simply waits
 * until the kernel answers or the child is killed.
 *
 * <p>Result encoding (see docs/refactor/ARCHITECTURE.md §4): the kernel
 * writes {@code [status: i32 LE][payload]} into its IPC result region and
 * returns the total length.
 */
public class NetIpcBridge {
    /** Expected child cancellation, including cancellation during descriptor cleanup. */
    public static final class InterruptedCall extends RuntimeException {
        InterruptedCall() { super("WASI child interrupted"); }
        InterruptedCall(InterruptedException cause) { super("WASI child interrupted", cause); }
    }

    /** Kernel return value meaning "not ready yet, retry later". */
    public static final int IPC_PENDING = -11;

    /** Decoded syscall result. */
    public static final class Result {
        public final int status;
        public final byte[] payload;

        Result(int status, byte[] payload) {
            this.status = status;
            this.payload = payload;
        }

        static final Result ERROR = new Result(-1, new byte[0]);
    }

    private final ConcurrentLinkedQueue<NetIpcRequest> incoming = new ConcurrentLinkedQueue<>();
    /** Requests the kernel answered IPC_PENDING; touched by the worker and by cancelPending. */
    private final ConcurrentLinkedQueue<NetIpcRequest> waiting = new ConcurrentLinkedQueue<>();

    private volatile WasmMemory memory;
    private volatile int argsBuf = -1, argsCap = 0, resultBuf = -1, resultCap = 0;
    private volatile Runnable wakeListener = () -> {};

    /** Called once the kernel's shared-memory layout is known. */
    public void setLayout(WasmMemory memory, int argsBuf, int argsCap, int resultBuf, int resultCap) {
        this.memory = memory;
        this.argsBuf = argsBuf;
        this.argsCap = argsCap;
        this.resultBuf = resultBuf;
        this.resultCap = resultCap;
    }

    /** Invoked when a child enqueues a request (wakes the worker thread). */
    public void setWakeListener(Runnable r) {
        this.wakeListener = r != null ? r : () -> {};
    }

    /**
     * Child thread: perform a socket syscall and wait for the kernel's answer.
     *
     * @throws RuntimeException if the calling thread is interrupted (child
     *         killed); the host function trap then unwinds the child.
     */
    public Result call(int sessionId, int syscallId, byte[] args) {
        if (Thread.currentThread().isInterrupted()) {
            throw new InterruptedCall();
        }
        NetIpcRequest req = new NetIpcRequest(sessionId, syscallId, args);
        incoming.add(req);
        wakeListener.run();
        try {
            return req.response.get();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            req.response.complete(Result.ERROR);
            throw new InterruptedCall(e);
        } catch (ExecutionException e) {
            return Result.ERROR;
        }
    }

    /**
     * Drop every queued request of a session (the child exited). Safe from
     * any thread; completes the futures so nothing is left waiting.
     */
    public void cancelPending(int sessionId) {
        for (ConcurrentLinkedQueue<NetIpcRequest> q : java.util.List.of(incoming, waiting)) {
            Iterator<NetIpcRequest> it = q.iterator();
            while (it.hasNext()) {
                NetIpcRequest req = it.next();
                if (req.sessionId == sessionId) {
                    req.response.complete(Result.ERROR);
                    it.remove();
                }
            }
        }
    }

    public boolean hasPending() {
        return !incoming.isEmpty() || !waiting.isEmpty();
    }

    public boolean hasNewRequests() {
        return !incoming.isEmpty();
    }

    /**
     * Worker thread: dispatch new requests and retry waiting ones.
     * @return number of requests completed
     */
    public int servicePending(WasmExport handleSockIpc) {
        NetIpcRequest req;
        while ((req = incoming.poll()) != null) {
            waiting.add(req);
        }
        int completed = 0;
        Iterator<NetIpcRequest> it = waiting.iterator();
        while (it.hasNext()) {
            req = it.next();
            if (req.response.isDone()) {
                it.remove();
                continue;
            }
            Result r = dispatch(handleSockIpc, req);
            if (r == null) {
                continue; // still pending
            }
            req.response.complete(r);
            it.remove();
            completed++;
        }
        return completed;
    }

    /** One kernel call. Returns null if the kernel answered IPC_PENDING. */
    private Result dispatch(WasmExport handleSockIpc, NetIpcRequest req) {
        WasmMemory mem = memory;
        if (mem == null || argsBuf < 0 || resultBuf < 0) {
            return Result.ERROR;
        }
        if (req.args.length > argsCap) {
            return Result.ERROR; // never truncate args: the kernel would misparse them
        }
        try {
            mem.writeBytes(argsBuf, req.args, 0, req.args.length);
            long[] rv = handleSockIpc.call(req.sessionId, req.syscallId,
                    argsBuf, req.args.length, resultBuf, resultCap);
            int len = (rv == null || rv.length == 0) ? -1 : (int) rv[0];
            if (len == IPC_PENDING) {
                return null;
            }
            if (len < 4) {
                return Result.ERROR;
            }
            len = Math.min(len, resultCap);
            byte[] raw = mem.readBytes(resultBuf, len);
            int status = (raw[0] & 0xFF) | ((raw[1] & 0xFF) << 8) | ((raw[2] & 0xFF) << 16) | ((raw[3] & 0xFF) << 24);
            byte[] payload = new byte[len - 4];
            System.arraycopy(raw, 4, payload, 0, payload.length);
            return new Result(status, payload);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("IPC dispatch error for syscall {}: {}", req.syscallId, e.getMessage());
            return Result.ERROR;
        }
    }
}
