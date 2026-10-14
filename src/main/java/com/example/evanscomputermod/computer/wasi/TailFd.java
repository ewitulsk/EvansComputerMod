package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;

import java.nio.charset.StandardCharsets;

/**
 * stdout/stderr of a detached program (see {@link ProcessManager#spawnDetached}):
 * nobody reads it, so writes never block; the last few KiB are kept for
 * diagnostics and tests, and whole lines go to the debug log.
 */
public final class TailFd implements WasiFileDescriptor {
    private static final int KEEP = 8192;
    private final StringBuilder tail = new StringBuilder();
    private final StringBuilder line = new StringBuilder();

    @Override
    public int read(byte[] buf, int offset, int length) {
        return -1;
    }

    @Override
    public synchronized int write(byte[] data, int offset, int length) {
        String s = new String(data, offset, length, StandardCharsets.UTF_8);
        tail.append(s);
        if (tail.length() > KEEP) tail.delete(0, tail.length() - KEEP);
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\n') {
                EvansComputerMod.LOGGER.debug("[detached] {}", line);
                line.setLength(0);
            } else if (line.length() < 400) {
                line.append(c);
            }
        }
        return length;
    }

    public synchronized String text() {
        return tail.toString();
    }

    @Override
    public void close() {}

    @Override
    public boolean isReadable() {
        return false;
    }

    @Override
    public boolean isWritable() {
        return true;
    }
}
