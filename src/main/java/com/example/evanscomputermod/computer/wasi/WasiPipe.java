package com.example.evanscomputermod.computer.wasi;

import java.util.ArrayDeque;

/**
 * Thread-safe byte pipe for inter-process communication.
 * One end writes, the other reads. Supports blocking and non-blocking reads.
 * Both stdout and stderr can share the same pipe (two write ends, one read end).
 */
public class WasiPipe {

    private final ArrayDeque<Byte> buffer;
    private final int capacity;
    private volatile boolean writeClosed = false;
    private volatile boolean readClosed = false;
    private final Object lock = new Object();
    /** Invoked (outside the lock) after data is written or the write end closes. */
    private volatile Runnable onActivity;

    public WasiPipe(int capacity) {
        this.capacity = capacity;
        this.buffer = new ArrayDeque<>(Math.min(capacity, 1024));
    }

    /**
     * Write data to the pipe. Blocks if the buffer is full.
     * @return number of bytes written, or -1 if read end is closed (broken pipe)
     */
    public int write(byte[] data, int offset, int length) {
        synchronized (lock) {
            if (readClosed) return -1;

            int written = 0;
            // (returns inside the loop on close/interrupt; falls through when
            // everything was written)
            while (written < length) {
                // Wait for space. Tell the reader first so it drains the pipe
                // (the listener only touches the reader's own monitor).
                while (buffer.size() >= capacity && !readClosed) {
                    fireActivity();
                    try {
                        lock.wait(100);
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        return written > 0 ? written : -1;
                    }
                }
                if (readClosed) return written > 0 ? written : -1;

                // Write as much as fits
                int space = capacity - buffer.size();
                int toWrite = Math.min(length - written, space);
                for (int i = 0; i < toWrite; i++) {
                    buffer.addLast(data[offset + written + i]);
                }
                written += toWrite;
                lock.notifyAll();
            }
        }
        fireActivity();
        return length;
    }

    /**
     * Write data to the pipe.
     */
    public int write(byte[] data) {
        return write(data, 0, data.length);
    }

    /**
     * Non-blocking write: copies as much as fits right now.
     * @return bytes written (possibly 0), or -1 if the read end is closed
     */
    public int tryWrite(byte[] data, int offset, int length) {
        int n;
        synchronized (lock) {
            if (readClosed) return -1;
            n = Math.min(length, capacity - buffer.size());
            for (int i = 0; i < n; i++) {
                buffer.addLast(data[offset + i]);
            }
            if (n > 0) lock.notifyAll();
        }
        return n;
    }

    /** Register a callback fired after writes / close (e.g. to wake a reader thread). */
    public void setOnActivity(Runnable r) {
        this.onActivity = r;
    }

    private void fireActivity() {
        Runnable r = onActivity;
        if (r != null) r.run();
    }

    /**
     * Read data from the pipe. Blocks until data is available or pipe is closed.
     * @return number of bytes read, or 0 on EOF (write end closed and buffer empty)
     */
    public int read(byte[] buf, int offset, int length) {
        synchronized (lock) {
            // Wait for data or EOF
            while (buffer.isEmpty() && !writeClosed) {
                try {
                    lock.wait(100);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    return 0;
                }
            }

            if (buffer.isEmpty() && writeClosed) {
                return 0; // EOF
            }

            int toRead = Math.min(length, buffer.size());
            for (int i = 0; i < toRead; i++) {
                buf[offset + i] = buffer.pollFirst();
            }
            lock.notifyAll();
            return toRead;
        }
    }

    /**
     * Non-blocking read. Returns immediately with available data.
     * @return number of bytes read (0 if nothing available)
     */
    public int tryRead(byte[] buf, int offset, int length) {
        synchronized (lock) {
            if (buffer.isEmpty()) {
                return 0;
            }
            int toRead = Math.min(length, buffer.size());
            for (int i = 0; i < toRead; i++) {
                buf[offset + i] = buffer.pollFirst();
            }
            lock.notifyAll();
            return toRead;
        }
    }

    public int tryRead(byte[] buf) {
        return tryRead(buf, 0, buf.length);
    }

    /** Close the write end. Readers will get EOF after draining remaining data. */
    public void closeWrite() {
        synchronized (lock) {
            writeClosed = true;
            lock.notifyAll();
        }
        fireActivity();
    }

    /** Close the read end. Writers will get broken pipe errors. */
    public void closeRead() {
        synchronized (lock) {
            readClosed = true;
            lock.notifyAll();
        }
    }

    public boolean isWriteClosed() { return writeClosed; }
    public boolean isReadClosed() { return readClosed; }

    /** Check if there's any data available without blocking. */
    public boolean hasData() {
        synchronized (lock) {
            return !buffer.isEmpty();
        }
    }
}
