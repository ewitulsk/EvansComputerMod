package com.example.evanscomputermod.wasm.fd;

import java.io.IOException;
import java.util.concurrent.ArrayBlockingQueue;

/**
 * Pipe file descriptor — mirrors simulator/src/fd.rs PipeFd.
 * Created in pairs (read end, write end) for inter-process communication.
 */
public class PipeFd implements IFileDescriptor {
    private final ArrayBlockingQueue<Byte> buffer;
    private final boolean isReadEnd;
    private volatile boolean closed = false;
    private volatile boolean otherEndClosed = false;
    private PipeFd otherEnd;

    private PipeFd(ArrayBlockingQueue<Byte> buffer, boolean isReadEnd) {
        this.buffer = buffer;
        this.isReadEnd = isReadEnd;
    }

    public static PipeFd[] createPair() {
        return createPair(4096);
    }

    public static PipeFd[] createPair(int capacity) {
        ArrayBlockingQueue<Byte> buf = new ArrayBlockingQueue<>(capacity);
        PipeFd readEnd = new PipeFd(buf, true);
        PipeFd writeEnd = new PipeFd(buf, false);
        readEnd.otherEnd = writeEnd;
        writeEnd.otherEnd = readEnd;
        return new PipeFd[]{readEnd, writeEnd};
    }

    @Override
    public int read(byte[] buf, int offset, int len) throws IOException {
        if (!isReadEnd) throw new IOException("Not the read end");
        if (buffer.isEmpty() && otherEndClosed) return 0; // EOF

        int count = 0;
        while (count < len) {
            Byte b = buffer.poll();
            if (b == null) break;
            buf[offset + count] = b;
            count++;
        }
        return count;
    }

    @Override
    public int write(byte[] buf, int offset, int len) throws IOException {
        if (isReadEnd) throw new IOException("Not the write end");
        if (otherEndClosed) throw new IOException("Broken pipe");

        for (int i = 0; i < len; i++) {
            try {
                buffer.put(buf[offset + i]);
            } catch (InterruptedException e) {
                return i;
            }
        }
        return len;
    }

    @Override
    public void close() {
        closed = true;
        if (otherEnd != null) otherEnd.otherEndClosed = true;
    }

    @Override
    public boolean isReadable() { return isReadEnd; }

    @Override
    public boolean isWritable() { return !isReadEnd; }
}
