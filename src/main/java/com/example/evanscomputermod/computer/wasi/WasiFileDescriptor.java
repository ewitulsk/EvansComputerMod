package com.example.evanscomputermod.computer.wasi;

import java.io.IOException;

/**
 * Interface for all file descriptor types in the WASI process model.
 * Implementations include pipes, /dev/null, and virtual filesystem files.
 */
public interface WasiFileDescriptor {
    int read(byte[] buf, int offset, int length) throws IOException;
    int write(byte[] data, int offset, int length) throws IOException;
    void close();
    boolean isReadable();
    boolean isWritable();
}
