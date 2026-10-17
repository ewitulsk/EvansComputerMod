package com.example.evanscomputermod.computer.wasi;

/**
 * File descriptor backed by a WasiPipe.
 * Can be either the read end or write end. The read end can be polled
 * ({@code poll_oneoff}): a program waits for keys with a timeout that way
 * (vim's Escape timeout).
 */
public class PipeFd implements WasiFileDescriptor, Pollable {

    private final WasiPipe pipe;
    private final boolean isReadEnd;
    /** O_NONBLOCK (WASI FDFLAGS_NONBLOCK): reads return {@link #WOULD_BLOCK} instead of waiting. */
    private volatile boolean nonBlocking;

    /** read() result meaning "no data yet" on a non-blocking read end. */
    public static final int WOULD_BLOCK = -2;

    /** The computer's terminal (not a pipe between programs or a redirect). */
    private volatile boolean tty;

    public void setNonBlocking(boolean on) { this.nonBlocking = on; }
    public PipeFd tty() { this.tty = true; return this; }
    public boolean isTty() { return tty; }
    public boolean isNonBlocking() { return nonBlocking; }

    public PipeFd(WasiPipe pipe, boolean isReadEnd) {
        this.pipe = pipe;
        this.isReadEnd = isReadEnd;
    }

    @Override
    public int read(byte[] buf, int offset, int length) {
        if (!isReadEnd) return -1;
        if (nonBlocking) {
            int n = pipe.tryRead(buf, offset, length);
            if (n == 0 && !pipe.isWriteClosed()) return WOULD_BLOCK;
            return n;
        }
        return pipe.read(buf, offset, length);
    }

    @Override
    public int write(byte[] data, int offset, int length) {
        if (isReadEnd) return -1;
        return pipe.write(data, offset, length);
    }

    @Override
    public void close() {
        if (isReadEnd) {
            pipe.closeRead();
        } else {
            pipe.closeWrite();
        }
    }

    @Override
    public boolean readReady() {
        return isReadEnd && (pipe.hasData() || pipe.isWriteClosed());
    }

    @Override
    public int readable() {
        return isReadEnd ? pipe.available() : 0;
    }

    @Override
    public boolean hungUp() {
        return isReadEnd ? pipe.isWriteClosed() && !pipe.hasData() : pipe.isReadClosed();
    }

    @Override
    public void addWaiter(Object waiter) { pipe.addWaiter(waiter); }

    @Override
    public void removeWaiter(Object waiter) { pipe.removeWaiter(waiter); }

    @Override
    public boolean isReadable() { return isReadEnd; }

    @Override
    public boolean isWritable() { return !isReadEnd; }
}
