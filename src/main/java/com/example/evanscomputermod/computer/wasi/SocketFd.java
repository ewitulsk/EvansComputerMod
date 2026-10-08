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

    // Remote shell session syscalls (sshd); handled by the kernel's session
    // manager over the same IPC path. Must match kernel.rs SESSION_*.
    public static final int SESSION_SPAWN = 20;
    public static final int SESSION_WRITE = 21;
    public static final int SESSION_READ = 22;
    public static final int SESSION_READ_BLOCKING = 23;
    public static final int SESSION_STATUS = 24;
    public static final int SESSION_CLOSE = 25;
    public static final int SESSION_RESIZE = 26;
    /** Wi-Fi control channel request (kernel net/wifi.rs WifiDev::ctl). */
    public static final int WIFI_CTL = 48;

    private final int kernelSocketId;
    private final int sessionId;
    private final NetIpcBridge bridge;
    private boolean closed = false;

    public SocketFd(int kernelSocketId, int sessionId, NetIpcBridge bridge) {
        this.kernelSocketId = kernelSocketId;
        this.sessionId = sessionId;
        this.bridge = bridge;
    }

    public int getKernelSocketId() {
        return kernelSocketId;
    }

    /** fd_read on a socket = recv. Returns bytes, 0 on EOF, -1 on error/timeout. */
    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        if (closed) return -1;
        NetIpcBridge.Result r = bridge.call(sessionId, SOCK_RECV, recvArgs(kernelSocketId, length, 0));
        if (r.status < 0) return -1;
        int n = Math.min(Math.min(r.status, r.payload.length), length);
        System.arraycopy(r.payload, 0, buf, offset, n);
        return n;
    }

    /** fd_write on a socket = send (may be partial). */
    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        if (closed) return -1;
        int n = Math.min(length, 4096);
        byte[] chunk = new byte[n];
        System.arraycopy(data, offset, chunk, 0, n);
        return bridge.call(sessionId, SOCK_SEND, sendArgs(kernelSocketId, chunk)).status;
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.call(sessionId, SOCK_CLOSE, encodeI32(kernelSocketId));
    }

    /** SOCK_RECV / SOCK_RECVFROM args: [sock_id: i32, max_len: i32, flags: i32]. */
    public static byte[] recvArgs(int sockId, int maxLen, int flags) {
        byte[] a = new byte[12];
        ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
        ab.putInt(0, sockId);
        ab.putInt(4, maxLen);
        ab.putInt(8, flags);
        return a;
    }

    /** SOCK_SEND args: [sock_id: i32, len: u16, bytes]. */
    public static byte[] sendArgs(int sockId, byte[] data) {
        int n = Math.min(data.length, 0xFFFF);
        byte[] a = new byte[6 + n];
        ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
        ab.putInt(0, sockId);
        ab.putShort(4, (short) n);
        System.arraycopy(data, 0, a, 6, n);
        return a;
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
