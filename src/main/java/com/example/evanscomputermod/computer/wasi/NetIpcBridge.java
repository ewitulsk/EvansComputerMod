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

    /**
     * Called from a child thread. Enqueues an IPC request and blocks until
     * the kernel thread services it (or timeout expires).
     *
     * @return response bytes from the kernel, or empty array on timeout/error
     */
    public byte[] callBlocking(int sessionId, int syscallId, byte[] args, long timeoutMs) {
        NetIpcRequest req = new NetIpcRequest(sessionId, syscallId, args);
        pending.add(req);
        try {
            byte[] result = req.response.get(timeoutMs, TimeUnit.MILLISECONDS);
            return result != null ? result : new byte[0];
        } catch (Exception e) {
            return new byte[0];
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
        while ((req = pending.poll()) != null) {
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
}
