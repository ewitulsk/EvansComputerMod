package com.example.evanscomputermod.computer.wasi;

/**
 * /dev/null file descriptor — discards writes, returns EOF on read.
 */
public class NullFd implements WasiFileDescriptor {
    @Override public int read(byte[] buf, int offset, int length) { return 0; } // EOF
    @Override public int write(byte[] data, int offset, int length) { return length; } // discard
    @Override public void close() {}
    @Override public boolean isReadable() { return true; }
    @Override public boolean isWritable() { return true; }
}
