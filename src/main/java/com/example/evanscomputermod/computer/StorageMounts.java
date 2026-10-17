package com.example.evanscomputermod.computer;

import org.jetbrains.annotations.Nullable;

import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Folders mounted into a computer's filesystem while something provides them:
 * a floppy in a Disk Drive next to the computer appears as {@code disk/}
 * (then {@code disk2/}, {@code disk3/} ...). Programs see a mount like any
 * folder: the kernel's shell ({@code ls}, {@code cd}), WASI programs (Lua's
 * and Python's files, {@code cat}, {@code cp}) and the host-ABI fs calls.
 *
 * <p>Owned by the computer's host, so mounts outlive a reboot of the
 * computer. Safe to change from any thread: readers see a snapshot.
 */
public final class StorageMounts {
    private final List<VirtualMount> mounts = new CopyOnWriteArrayList<>();

    /**
     * Mount {@code root} under the first free name of {@code base},
     * {@code base2}, {@code base3}, ... Returns the name used.
     */
    public synchronized String add(String base, Path root, boolean readOnly) {
        String name = base;
        for (int i = 2; find(name) != null; i++) name = base + i;
        mounts.add(new VirtualMount(name, root, readOnly));
        return name;
    }

    /** Unmount {@code name}; fine if it isn't mounted. */
    public synchronized void remove(String name) {
        mounts.removeIf(m -> m.getPrefix().equals(name));
    }

    /** The mounts now, in the order they were made. */
    public List<VirtualMount> list() {
        return List.copyOf(mounts);
    }

    @Nullable
    public VirtualMount find(String name) {
        for (VirtualMount m : mounts) if (m.getPrefix().equals(name)) return m;
        return null;
    }

    /** The mount a path (relative to the computer's root, no leading "/") falls in, or null. */
    @Nullable
    public VirtualMount of(String path) {
        if (path == null) return null;
        String p = path.replaceAll("^/+", "");
        int slash = p.indexOf('/');
        return find(slash < 0 ? p : p.substring(0, slash));
    }

    /** The real path of {@code path} inside its mount (the mount's root for the mount itself), or null. */
    @Nullable
    public static Path resolve(VirtualMount m, String path) {
        String p = path.replaceAll("^/+", "");
        String rest = p.length() > m.getPrefix().length() ? p.substring(m.getPrefix().length() + 1) : "";
        if (rest.contains("..") || rest.contains("\\") || rest.contains(":")) return null;
        Path real = m.getRealRoot().resolve(rest).normalize();
        return real.startsWith(m.getRealRoot()) ? real : null;
    }
}
