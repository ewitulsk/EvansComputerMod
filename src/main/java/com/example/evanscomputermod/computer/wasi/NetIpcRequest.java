package com.example.evanscomputermod.computer.wasi;

import java.util.concurrent.CompletableFuture;

/**
 * A single IPC request from a child WASI process to the kernel.
 * The child thread enqueues this and blocks on the response future.
 * The kernel thread (in process_wait) services it and completes the future.
 */
public class NetIpcRequest {
    public final int sessionId;   // child PID — identifies which IpcSocketTable to use
    public final int syscallId;   // SOCK_SOCKET, SOCK_CONNECT, etc.
    public final byte[] args;     // serialized arguments
    public final CompletableFuture<byte[]> response = new CompletableFuture<>();

    public NetIpcRequest(int sessionId, int syscallId, byte[] args) {
        this.sessionId = sessionId;
        this.syscallId = syscallId;
        this.args = args;
    }
}
