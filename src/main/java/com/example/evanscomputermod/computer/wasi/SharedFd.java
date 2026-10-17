package com.example.evanscomputermod.computer.wasi;

import java.io.IOException;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * One open file or pipe end held by several owners: the kernel's handle for
 * a redirect or pipe ({@code fd_open}, {@code pipe_create}) and the children
 * it hands it to as stdin/stdout/stderr ({@code process_spawn}). Each owner
 * gets its own {@link #open() handle}; the underlying descriptor closes when
 * the last handle does, so a pipe's reader sees end of input only once every
 * writer has exited or closed it.
 */
public final class SharedFd {
    private final WasiFileDescriptor inner;
    private final AtomicInteger refs = new AtomicInteger();

    public SharedFd(WasiFileDescriptor inner) {
        this.inner = inner;
    }

    public WasiFileDescriptor inner() {
        return inner;
    }

    /** A new handle (one more owner). */
    public Handle open() {
        refs.incrementAndGet();
        return new Handle();
    }

    /** One owner's view of the shared descriptor. */
    public final class Handle implements WasiFileDescriptor {
        private boolean closed;

        public SharedFd shared() {
            return SharedFd.this;
        }

        @Override
        public int read(byte[] buf, int offset, int length) throws IOException {
            return closed ? -1 : inner.read(buf, offset, length);
        }

        @Override
        public int write(byte[] data, int offset, int length) throws IOException {
            return closed ? -1 : inner.write(data, offset, length);
        }

        @Override
        public synchronized void close() {
            if (closed) return;
            closed = true;
            if (refs.decrementAndGet() == 0) inner.close();
        }

        @Override
        public boolean isReadable() {
            return inner.isReadable();
        }

        @Override
        public boolean isWritable() {
            return inner.isWritable();
        }
    }
}
