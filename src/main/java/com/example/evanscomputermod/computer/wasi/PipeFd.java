package com.example.evanscomputermod.computer.wasi;

/**
 * File descriptor backed by a WasiPipe.
 * Can be either the read end or write end.
 */
public class PipeFd implements WasiFileDescriptor {

    private final WasiPipe pipe;
    private final boolean isReadEnd;
    /** O_NONBLOCK (WASI FDFLAGS_NONBLOCK): reads return {@link #WOULD_BLOCK} instead of waiting. */
    private volatile boolean nonBlocking;

    /** read() result meaning "no data yet" on a non-blocking read end. */
    public static final int WOULD_BLOCK = -2;

    public void setNonBlocking(boolean on) { this.nonBlocking = on; }
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
    public boolean isReadable() { return isReadEnd; }

    @Override
    public boolean isWritable() { return !isReadEnd; }
}
