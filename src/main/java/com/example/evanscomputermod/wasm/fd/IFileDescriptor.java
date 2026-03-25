package com.example.evanscomputermod.wasm.fd;

import java.io.IOException;

/**
 * File descriptor interface — mirrors simulator/src/fd.rs FileDescriptor trait.
 * All I/O abstractions (pipes, files, terminals, sockets) implement this.
 */
public interface IFileDescriptor {
    int read(byte[] buf, int offset, int len) throws IOException;
    int write(byte[] buf, int offset, int len) throws IOException;
    void close();
    boolean isReadable();
    boolean isWritable();
}
