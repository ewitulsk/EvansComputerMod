package com.example.evanscomputermod.computer.wasi;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.file.Path;

/**
 * File descriptor backed by a file in the virtual filesystem.
 */
public class VfsFileFd implements WasiFileDescriptor {

    private final RandomAccessFile raf;
    private final boolean readable;
    private final boolean writable;

    public VfsFileFd(Path path, boolean readable, boolean writable, boolean append) throws IOException {
        String mode = writable ? "rw" : "r";
        this.raf = new RandomAccessFile(path.toFile(), mode);
        this.readable = readable;
        this.writable = writable;
        if (append && writable) {
            raf.seek(raf.length());
        }
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        if (!readable) return -1;
        int n = raf.read(buf, offset, length);
        return n == -1 ? 0 : n; // WASI returns 0 for EOF, not -1
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        if (!writable) return -1;
        raf.write(data, offset, length);
        return length;
    }

    @Override
    public void close() {
        try { raf.close(); } catch (IOException ignored) {}
    }

    @Override
    public boolean isReadable() { return readable; }

    @Override
    public boolean isWritable() { return writable; }

    public void truncate() throws IOException {
        raf.setLength(0);
        raf.seek(0);
    }

    public long seek(long offset, int whence) throws IOException {
        switch (whence) {
            case 0 -> raf.seek(offset);                        // SET
            case 1 -> raf.seek(raf.getFilePointer() + offset); // CUR
            case 2 -> raf.seek(raf.length() + offset);         // END
        }
        return raf.getFilePointer();
    }

    public long size() throws IOException {
        return raf.length();
    }
}
