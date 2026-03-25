package com.example.evanscomputermod.wasm.fd;

import java.util.HashMap;
import java.util.Map;

/**
 * Per-process file descriptor table — mirrors simulator/src/fd.rs FdTable.
 * FDs 0-2 are stdin/stdout/stderr.
 */
public class FdTable {
    private final Map<Integer, IFileDescriptor> fds = new HashMap<>();
    private int nextFd = 3;

    public int allocate(IFileDescriptor fd) {
        int num = nextFd++;
        fds.put(num, fd);
        return num;
    }

    public void insertAt(int num, IFileDescriptor fd) {
        fds.put(num, fd);
        if (num >= nextFd) nextFd = num + 1;
    }

    public IFileDescriptor get(int fd) {
        return fds.get(fd);
    }

    public boolean close(int fd) {
        IFileDescriptor desc = fds.remove(fd);
        if (desc != null) {
            desc.close();
            return true;
        }
        return false;
    }

    public boolean contains(int fd) {
        return fds.containsKey(fd);
    }

    public int size() {
        return fds.size();
    }
}
