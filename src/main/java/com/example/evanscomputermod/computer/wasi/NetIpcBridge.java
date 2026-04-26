package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.wasm.WasmExport;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.example.evanscomputermod.api.wasm.WasmTrap;

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
     * Called from a child thread. Enqueues an IPC request and blocks until
     * the kernel thread services it (or timeout expires).
     *
     * @return response bytes from the kernel, or empty array on timeout/error
     */
    public byte[] callBlocking(int sessionId, int syscallId, byte[] args, long timeoutMs) {
        NetIpcRequest req = new NetIpcRequest(sessionId, syscallId, args);
        pending.add(req);
        synchronized (pendingSignal) {
            pendingSignal.notifyAll();
        }
        try {
            byte[] result = req.response.get(timeoutMs, TimeUnit.MILLISECONDS);
            return result != null ? result : new byte[0];
        } catch (Exception e) {
            return new byte[0];
        }
    }

    /**
     * Called from the kernel thread (inside process_wait polling loop).
     * Services all pending IPC requests by calling the kernel's
     * handle_sock_ipc export.
     *
     * @param instance       the kernel's WASM instance
     * @param handleSockIpc  pre-resolved handle_sock_ipc export
     * @return number of requests serviced
     */
    public int servicePending(WasmInstance instance, WasmExport handleSockIpc) {
        int serviced = 0;
        NetIpcRequest req;
        while ((req = pending.poll()) != null) {
            try {
                byte[] result = dispatchToKernel(instance, handleSockIpc,
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
    private byte[] dispatchToKernel(WasmInstance instance, WasmExport handleSockIpc,
                                    int sessionId, int syscallId, byte[] args) throws WasmTrap {
        WasmMemory mem = instance.memory();

        // Write args to IPC_ARGS_BUFFER
        int argsLen = Math.min(args.length, IPC_ARGS_BUFFER_SIZE);
        mem.writeBytes(IPC_ARGS_BUFFER, args, 0, argsLen);

        // Call kernel export: handle_sock_ipc(session, syscall_id, args_ptr, args_len, result_ptr, result_len) -> i32
        long[] results = handleSockIpc.call(
                sessionId,
                syscallId,
                IPC_ARGS_BUFFER,
                argsLen,
                IPC_RESULT_BUFFER,
                IPC_RESULT_BUFFER_SIZE);

        int resultLen = (int) results[0];

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
        return mem.readBytes(IPC_RESULT_BUFFER, readLen);
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
