package com.example.evanscomputermod.computer.wasi;

/**
 * File descriptor backed by a WasiPipe.
 * Can be either the read end or write end.
 */
public class PipeFd implements WasiFileDescriptor {

    private final WasiPipe pipe;
    private final boolean isReadEnd;

    public PipeFd(WasiPipe pipe, boolean isReadEnd) {
        this.pipe = pipe;
        this.isReadEnd = isReadEnd;
    }

    @Override
    public int read(byte[] buf, int offset, int length) {
        if (!isReadEnd) return -1;
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
