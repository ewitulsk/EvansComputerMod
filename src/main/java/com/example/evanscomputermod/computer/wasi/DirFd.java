package com.example.evanscomputermod.computer.wasi;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;

/**
 * Directory file descriptor supporting fd_readdir.
 */
public class DirFd implements WasiFileDescriptor {

    private final Path dirPath;
    private List<DirEntry> cachedEntries;

    public DirFd(Path dirPath) {
        this.dirPath = dirPath;
    }

    /** A directory entry for WASI readdir. */
    public record DirEntry(String name, byte type, long inode) {}

    /**
     * List directory entries starting from the given cookie (0-based index).
     */
    public List<DirEntry> readDir(long cookie) {
        if (cachedEntries == null) {
            cachedEntries = new ArrayList<>();
            try (Stream<Path> stream = Files.list(dirPath)) {
                stream.sorted().forEach(p -> {
                    String name = p.getFileName().toString();
                    byte type = Files.isDirectory(p) ? (byte) 3 : (byte) 4; // DIR=3, REG=4
                    long inode = name.hashCode() & 0xFFFFFFFFL;
                    cachedEntries.add(new DirEntry(name, type, inode));
                });
            } catch (IOException e) {
                // Return empty on error
            }
        }

        int start = (int) Math.min(cookie, cachedEntries.size());
        return cachedEntries.subList(start, cachedEntries.size());
    }

    public Path getDirPath() { return dirPath; }

    @Override public int read(byte[] buf, int offset, int length) { return 0; }
    @Override public int write(byte[] data, int offset, int length) { return -1; }
    @Override public void close() {}
    @Override public boolean isReadable() { return true; }
    @Override public boolean isWritable() { return false; }
}
