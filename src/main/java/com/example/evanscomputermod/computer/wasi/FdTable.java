package com.example.evanscomputermod.computer.wasi;

import java.util.HashMap;
import java.util.Map;

/**
 * Per-process file descriptor table.
 * FDs 0-2 are stdio (stdin, stdout, stderr). FD 3+ are allocated dynamically.
 */
public class FdTable {

    private final Map<Integer, WasiFileDescriptor> fds = new HashMap<>();
    private int nextFd = 4; // 0=stdin, 1=stdout, 2=stderr, 3=preopened root dir

    public void insertAt(int fd, WasiFileDescriptor desc) {
        fds.put(fd, desc);
    }

    public WasiFileDescriptor get(int fd) {
        return fds.get(fd);
    }

    public int allocate(WasiFileDescriptor desc) {
        int fd = nextFd++;
        fds.put(fd, desc);
        return fd;
    }

    public void close(int fd) {
        WasiFileDescriptor desc = fds.remove(fd);
        if (desc != null) {
            desc.close();
        }
    }

    public boolean contains(int fd) {
        return fds.containsKey(fd);
    }

    /** Close all open file descriptors. */
    public void closeAll() {
        for (WasiFileDescriptor desc : fds.values()) {
            desc.close();
        }
        fds.clear();
    }
}
