package com.example.evanscomputermod.computer;

import java.nio.file.Files;
import java.nio.file.Path;

/**
 * A virtual mount point mapping a virtual path prefix to a real filesystem directory.
 *
 * <p>The mount system allows the WASM OS to see a unified filesystem composed of
 * multiple real directories. For example:</p>
 * <ul>
 *   <li>{@code ""} (root) → {@code computer-data/{UUID}/} (read-write, per-computer storage)</li>
 *   <li>{@code "bin"} → {@code wasm-bin/} (read-only, shared server programs)</li>
 * </ul>
 *
 * <p>Security: resolved paths are always validated to stay within the mount's real root.
 * Path traversal attacks (via {@code ..}, {@code \}, etc.) are rejected.</p>
 */
public class VirtualMount {

    private static final int MAX_PATH_LENGTH = 256;
    private static final int MAX_PATH_DEPTH = 10;

    private final String prefix;     // virtual path prefix (e.g., "bin", "" for root)
    private final Path realRoot;     // actual filesystem directory
    private final boolean readOnly;

    public VirtualMount(String prefix, Path realRoot, boolean readOnly) {
        // Normalize prefix: strip leading/trailing slashes
        this.prefix = prefix.replaceAll("^/+|/+$", "");
        this.realRoot = realRoot.toAbsolutePath().normalize();
        this.readOnly = readOnly;
    }

    public String getPrefix() { return prefix; }
    public Path getRealRoot() { return realRoot; }
    public boolean isReadOnly() { return readOnly; }

    /**
     * Check if this mount matches the given virtual path.
     *
     * @param virtualPath the path from the WASM OS (already resolved against CWD by Rust)
     * @return true if this mount's prefix matches the path
     */
    public boolean matches(String virtualPath) {
        if (prefix.isEmpty()) {
            return true; // root mount matches everything
        }
        return virtualPath.equals(prefix)
                || virtualPath.startsWith(prefix + "/");
    }

    /**
     * Resolve a virtual path to a real filesystem path under this mount.
     *
     * @param virtualPath the full virtual path
     * @return the resolved real path, or null if the path is invalid or escapes the mount
     */
    public Path resolve(String virtualPath) {
        // Strip the prefix to get the remainder
        String remainder;
        if (prefix.isEmpty()) {
            remainder = virtualPath;
        } else if (virtualPath.equals(prefix)) {
            remainder = "";
        } else {
            // virtualPath starts with prefix + "/"
            remainder = virtualPath.substring(prefix.length() + 1);
        }

        // Validate the remainder
        if (!isValidPath(remainder)) {
            return null;
        }

        // Resolve against the real root
        Path resolved;
        if (remainder.isEmpty()) {
            resolved = realRoot;
        } else {
            resolved = realRoot.resolve(remainder).normalize();
        }

        // Security: ensure resolved path stays within the mount's real root
        if (!resolved.startsWith(realRoot)) {
            return null;
        }

        return resolved;
    }

    /**
     * Check if a file exists at the resolved path under this mount.
     */
    public boolean fileExists(String virtualPath) {
        Path resolved = resolve(virtualPath);
        return resolved != null && Files.exists(resolved);
    }

    /**
     * Validate a path component for safety.
     */
    private static boolean isValidPath(String path) {
        if (path == null) return false;
        if (path.length() > MAX_PATH_LENGTH) return false;
        if (path.contains("..") || path.contains("\\") ||
            path.contains(":") || path.contains("\0")) {
            return false;
        }
        if (path.startsWith("/")) return false;
        long depth = path.chars().filter(c -> c == '/').count();
        return depth <= MAX_PATH_DEPTH;
    }
}
