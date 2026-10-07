package com.example.evanscomputermod.computer.wasi;

import java.io.IOException;

/**
 * A character device a program opened under {@code /dev} (e.g. a speaker's
 * {@code /dev/audio}). Supports {@code O_NONBLOCK} through
 * {@code fd_fdstat_set_flags}; a non-blocking write that can't take anything
 * returns 0 bytes, which the host reports as {@code EAGAIN}.
 */
public interface DeviceFd extends WasiFileDescriptor {

    boolean isNonBlocking();

    void setNonBlocking(boolean nonBlocking);

    /** An error with a specific WASI errno (EINVAL, EIO, ...). */
    final class ErrnoException extends IOException {
        public static final int EIO = 29;
        public static final int EINVAL = 28;

        private final int errno;

        public ErrnoException(int errno, String message) {
            super(message);
            this.errno = errno;
        }

        public int errno() {
            return errno;
        }
    }
}
