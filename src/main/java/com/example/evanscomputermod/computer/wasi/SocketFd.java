package com.example.evanscomputermod.computer.wasi;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * A WASI file descriptor backed by a kernel-side socket.
 * Integrates into the FdTable so fd_read/fd_write/fd_close work on sockets.
 *
 * <p>All operations are dispatched through the NetIpcBridge to the kernel
 * thread, which calls into the kernel's NetStack.</p>
 */
public class SocketFd implements WasiFileDescriptor {

    // Syscall IDs matching the kernel's net_ipc_handler.rs
    public static final int SOCK_SOCKET = 0;
    public static final int SOCK_BIND = 1;
    public static final int SOCK_CONNECT = 2;
    public static final int SOCK_LISTEN = 3;
    public static final int SOCK_ACCEPT = 4;
    public static final int SOCK_SEND = 5;
    public static final int SOCK_RECV = 6;
    public static final int SOCK_CLOSE = 7;
    public static final int SOCK_SETSOCKOPT = 8;
    public static final int SOCK_SENDTO = 9;
    public static final int SOCK_RECVFROM = 10;
    public static final int SOCK_GETADDRINFO = 11;
    public static final int SOCK_GETSOCKNAME = 12;
    public static final int SOCK_GETPEERNAME = 13;
    public static final int SOCK_SHUTDOWN = 14;
    public static final int SOCK_DESTROY_SESSION = 99;

    private final int kernelSocketId;
    private final int sessionId;
    private final NetIpcBridge bridge;
    private boolean closed = false;

    /** Default timeout for blocking socket operations (ms). */
    private static final long DEFAULT_TIMEOUT = 30_000;

    public SocketFd(int kernelSocketId, int sessionId, NetIpcBridge bridge) {
        this.kernelSocketId = kernelSocketId;
        this.sessionId = sessionId;
        this.bridge = bridge;
    }

    public int getKernelSocketId() {
        return kernelSocketId;
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        if (closed) return -1;
        // SOCK_RECV args: [sock_id: i32, max_len: i32, flags: i32]
        byte[] args = new byte[12];
        ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
        ab.putInt(0, kernelSocketId);
        ab.putInt(4, length);
        ab.putInt(8, 0); // flags = 0

        byte[] result = bridge.callBlocking(sessionId, SOCK_RECV, args, DEFAULT_TIMEOUT);
        if (result.length == 0) return 0; // timeout
        // Check for error (4-byte negative i32)
        if (result.length == 4) {
            int val = ByteBuffer.wrap(result).order(ByteOrder.LITTLE_ENDIAN).getInt(0);
            if (val < 0) return -1; // error or connection closed
        }
        // Result is raw data bytes
        int copyLen = Math.min(result.length, length);
        System.arraycopy(result, 0, buf, offset, copyLen);
        return copyLen;
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        if (closed) return -1;
        // SOCK_SEND args: [sock_id: i32, data_len: u16, data_bytes...]
        byte[] args = new byte[4 + 2 + length];
        ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
        ab.putInt(0, kernelSocketId);
        ab.putShort(4, (short) length);
        System.arraycopy(data, offset, args, 6, length);

        byte[] result = bridge.callBlocking(sessionId, SOCK_SEND, args, DEFAULT_TIMEOUT);
        if (result.length < 4) return -1;
        return ByteBuffer.wrap(result).order(ByteOrder.LITTLE_ENDIAN).getInt(0);
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        byte[] args = new byte[4];
        ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN).putInt(0, kernelSocketId);
        bridge.callBlocking(sessionId, SOCK_CLOSE, args, 5000);
    }

    @Override
    public boolean isReadable() { return true; }

    @Override
    public boolean isWritable() { return true; }

    // --- Helper: encode i32 as 4-byte LE ---

    public static byte[] encodeI32(int val) {
        byte[] b = new byte[4];
        ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN).putInt(0, val);
        return b;
    }

    public static int decodeI32(byte[] data, int offset) {
        if (data.length < offset + 4) return -1;
        return ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN).getInt(offset);
    }
}
