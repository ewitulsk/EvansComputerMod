package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import io.github.kawamuray.wasmtime.*;

import java.nio.ByteBuffer;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.TimeUnit;

/**
 * Thread-safe bridge for IPC between child WASI processes and the kernel's
 * networking stack. Child threads enqueue requests and block; the kernel
 * thread (inside process_wait) services them by calling the kernel's
 * handle_sock_ipc export.
 */
public class NetIpcBridge {

    /** Scratch region in kernel WASM memory for IPC arguments. */
    private static final int IPC_ARGS_BUFFER = 0x13000;
    private static final int IPC_ARGS_BUFFER_SIZE = 4096;

    /** Scratch region in kernel WASM memory for IPC results. */
    private static final int IPC_RESULT_BUFFER = 0x14000;
    private static final int IPC_RESULT_BUFFER_SIZE = 8192;

    private final ConcurrentLinkedQueue<NetIpcRequest> pending = new ConcurrentLinkedQueue<>();
    private final Object pendingSignal = new Object();

    /**
     * Maximum requests to dispatch in a single servicePending call. Each
     * dispatch is a separate handleSockIpc.call(store, ...) re-entry into
     * the kernel WASM and may take ~75 ms (handle_sendto ARP retry
     * exhaustion). With this cap the worker thread is held inside
     * servicePending for at most ~300 ms before returning to the outer
     * process_wait loop, which then re-runs drainAndDeliverInterrupts(),
     * checkFramebufferDirty(), Ctrl+T checks, and stdout drain. Without
     * the cap, a child making rapid sock_sendto calls (e.g. ping in its
     * tight failure loop) can pin the worker for arbitrarily long.
     */
    private static final int MAX_PER_CALL = 4;

    /**
     * Called from a child thread. Enqueues an IPC request and blocks until
     * the kernel thread services it (or timeout expires).
     *
     * <p>If the calling thread is interrupted (e.g. parent fired
     * processManager.killAll() on Ctrl+T), this method THROWS rather than
     * returns empty. Wasmtime catches the exception thrown from the host
     * function lambda and traps the WASM call; the trap propagates up
     * through {@code _start} into ProcessManager.runWasiProcess, which
     * exits the child cleanly. Returning an empty byte[] here (the
     * previous behavior) was wrong — the child's Rust code typically
     * just sees the resulting -1 and loops, leaving the interrupted
     * thread in a tight CPU spin that dominates JVM allocation rate
     * and lags the entire game.
     *
     * @return response bytes from the kernel, or empty array on timeout
     * @throws RuntimeException if the calling thread is interrupted
     */
    public byte[] callBlocking(int sessionId, int syscallId, byte[] args, long timeoutMs) {
        if (Thread.currentThread().isInterrupted()) {
            throw new RuntimeException("WASI child interrupted");
        }
        NetIpcRequest req = new NetIpcRequest(sessionId, syscallId, args);
        pending.add(req);
        synchronized (pendingSignal) {
            pendingSignal.notifyAll();
        }
        try {
            byte[] result = req.response.get(timeoutMs, TimeUnit.MILLISECONDS);
            return result != null ? result : new byte[0];
        } catch (InterruptedException e) {
            // Re-assert the flag (so any subsequent host call also
            // fast-paths) and throw to trap the host function. The outer
            // ProcessManager.runWasiProcess catch block recognizes the
            // resulting trap and treats it as a normal interrupted-child
            // exit, not a crash.
            Thread.currentThread().interrupt();
            throw new RuntimeException("WASI child interrupted", e);
        } catch (Exception e) {
            return new byte[0];
        }
    }

    /**
     * Discard any pending requests for the given session. Called by the
     * worker thread immediately before SOCK_DESTROY_SESSION runs, so the
     * dead session's leftover requests don't get serviced (and waste
     * 75 ms each on ARP retries) after the session is gone.
     */
    public void cancelPending(int sessionId) {
        java.util.Iterator<NetIpcRequest> it = pending.iterator();
        while (it.hasNext()) {
            NetIpcRequest req = it.next();
            if (req.sessionId == sessionId) {
                req.response.complete(new byte[0]);
                it.remove();
            }
        }
    }

    /**
     * Called from the kernel thread (inside process_wait polling loop).
     * Services all pending IPC requests by calling the kernel's handle_sock_ipc
     * export function.
     *
     * @param store  the kernel's Wasmtime Store
     * @param memory the kernel's WASM linear memory
     * @param handleSockIpc the kernel's handle_sock_ipc exported function
     * @return number of requests serviced
     */
    public int servicePending(Store<Void> store, Memory memory, Func handleSockIpc) {
        int serviced = 0;
        NetIpcRequest req;
        while (serviced < MAX_PER_CALL && (req = pending.poll()) != null) {
            try {
                byte[] result = dispatchToKernel(store, memory, handleSockIpc,
                        req.sessionId, req.syscallId, req.args);
                req.response.complete(result);
            } catch (Exception e) {
                EvansComputerMod.LOGGER.debug("IPC dispatch error for syscall {}: {}",
                        req.syscallId, e.getMessage());
                req.response.complete(new byte[0]);
            }
            serviced++;
        }
        return serviced;
    }

    /**
     * Write args to kernel WASM memory, call the kernel export, read result back.
     */
    private byte[] dispatchToKernel(Store<Void> store, Memory memory, Func handleSockIpc,
                                     int sessionId, int syscallId, byte[] args) {
        ByteBuffer buf = memory.buffer(store);

        // Write args to IPC_ARGS_BUFFER
        int argsLen = Math.min(args.length, IPC_ARGS_BUFFER_SIZE);
        buf.position(IPC_ARGS_BUFFER);
        buf.put(args, 0, argsLen);

        // Call kernel export: handle_sock_ipc(session, syscall_id, args_ptr, args_len, result_ptr, result_len) -> i32
        Val[] results = handleSockIpc.call(store,
                Val.fromI32(sessionId),
                Val.fromI32(syscallId),
                Val.fromI32(IPC_ARGS_BUFFER),
                Val.fromI32(argsLen),
                Val.fromI32(IPC_RESULT_BUFFER),
                Val.fromI32(IPC_RESULT_BUFFER_SIZE));

        int resultLen = results[0].i32();

        if (resultLen < 0) {
            // Negative = error code, encode as 4-byte LE i32
            byte[] err = new byte[4];
            err[0] = (byte) (resultLen & 0xFF);
            err[1] = (byte) ((resultLen >> 8) & 0xFF);
            err[2] = (byte) ((resultLen >> 16) & 0xFF);
            err[3] = (byte) ((resultLen >> 24) & 0xFF);
            return err;
        }

        if (resultLen == 0) {
            return new byte[0];
        }

        // Read result from IPC_RESULT_BUFFER
        int readLen = Math.min(resultLen, IPC_RESULT_BUFFER_SIZE);
        byte[] result = new byte[readLen];
        buf = memory.buffer(store); // re-fetch in case it grew
        buf.position(IPC_RESULT_BUFFER);
        buf.get(result, 0, readLen);
        return result;
    }

    /** Check if there are pending requests (for optimizing poll sleep time). */
    public boolean hasPending() {
        return !pending.isEmpty();
    }

    /**
     * Wait until new IPC requests are enqueued, or timeout expires.
     *
     * Returns immediately if a request is already pending.
     */
    public void waitForPending(long timeoutMs) throws InterruptedException {
        if (timeoutMs <= 0 || !pending.isEmpty()) {
            return;
        }

        synchronized (pendingSignal) {
            if (!pending.isEmpty()) {
                return;
            }
            pendingSignal.wait(timeoutMs);
        }
    }
}
