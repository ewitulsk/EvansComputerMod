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
    /** Folders listed besides the real ones: the mounts at the computer's root (disk/). */
    private final List<String> extraDirs;
    private List<DirEntry> cachedEntries;

    public DirFd(Path dirPath) {
        this(dirPath, List.of());
    }

    public DirFd(Path dirPath, List<String> extraDirs) {
        this.dirPath = dirPath;
        this.extraDirs = extraDirs;
    }

    /** A directory entry for WASI readdir. */
    public record DirEntry(String name, byte type, long inode) {}

    /**
     * List directory entries starting from the given cookie (0-based index).
     */
    public List<DirEntry> readDir(long cookie) {
        if (cachedEntries == null) {
            cachedEntries = new ArrayList<>();
            for (String name : extraDirs) cachedEntries.add(new DirEntry(name, (byte) 3, name.hashCode() & 0xFFFFFFFFL));
            try (Stream<Path> stream = Files.list(dirPath)) {
                stream.sorted().forEach(p -> {
                    String name = p.getFileName().toString();
                    if (extraDirs.contains(name)) return; // a mount hides a folder of the same name
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
