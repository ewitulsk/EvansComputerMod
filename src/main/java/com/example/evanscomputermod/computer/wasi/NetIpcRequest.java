package com.example.evanscomputermod.computer.wasi;

import java.util.concurrent.CompletableFuture;

/**
 * A single socket syscall from a child WASI process to the kernel.
 * The child thread enqueues this and blocks on {@link #response}; the
 * kernel worker thread services it (possibly over several retries while
 * the kernel answers IPC_PENDING) and completes the future.
 */
public class NetIpcRequest {
    public final int sessionId;   // child PID
    public final int syscallId;   // SocketFd.SOCK_*
    public final byte[] args;     // serialized arguments
    public final CompletableFuture<NetIpcBridge.Result> response = new CompletableFuture<>();

    public NetIpcRequest(int sessionId, int syscallId, byte[] args) {
        this.sessionId = sessionId;
        this.syscallId = syscallId;
        this.args = args;
    }
}
