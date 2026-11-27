package com.example.evanscomputermod.computer.repo;

import com.example.evanscomputermod.EvansComputerMod;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Which running programs are the system's own: the ones the mod ships in the
 * server's {@code wasm-bin/} folder (a computer's {@code /server-bin}). The
 * secret store only answers those.
 *
 * <p>A program is identified by what it <em>is</em>, not by where it was
 * started from: a module loaded from the server's folder, or one whose bytes
 * are exactly a shipped program's (a system app copied into {@code /apps}).
 * The check is made on the bytes the process was started with, so swapping the
 * file afterwards changes nothing. A copy of {@code pkg} with one byte changed,
 * dropped into {@code /bin}, is nobody.
 */
public final class SystemPrograms {
    private record Shipped(String name, long size, long modified, byte[] sha256) {}

    private static Path indexedDir;
    private static Map<Long, List<Shipped>> bySize = Map.of();

    private SystemPrograms() {}

    private static byte[] sha256(byte[] b) {
        try {
            return MessageDigest.getInstance("SHA-256").digest(b);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    private static String stem(Path p) {
        String n = p.getFileName().toString();
        return n.endsWith(".wasm") ? n.substring(0, n.length() - 5) : n;
    }

    /** Sizes of the shipped programs (hashes are computed only when a size matches). */
    private static synchronized Map<Long, List<Shipped>> index(Path binDir) {
        if (binDir.equals(indexedDir)) return bySize;
        Map<Long, List<Shipped>> m = new HashMap<>();
        try (var files = Files.list(binDir)) {
            for (Path f : (Iterable<Path>) files::iterator) {
                if (!f.getFileName().toString().endsWith(".wasm") || !Files.isRegularFile(f)) continue;
                m.computeIfAbsent(Files.size(f), k -> new ArrayList<>()).add(new Shipped(stem(f), Files.size(f), Files.getLastModifiedTime(f).toMillis(), null));
            }
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[repo] cannot list {}: {}", binDir, e.toString());
        }
        indexedDir = binDir;
        bySize = m;
        return m;
    }

    /**
     * The shipped program a process is ({@code pkg}, {@code store}, ...), or
     * null when it is not one.
     *
     * @param binDir   the server's program folder
     * @param wasmPath where the process's module was loaded from
     * @param bytes    the module's bytes, as loaded
     */
    public static String identify(Path binDir, Path wasmPath, byte[] bytes) {
        if (binDir == null || wasmPath == null || bytes == null) return null;
        Path bin = binDir.toAbsolutePath().normalize();
        Path from = wasmPath.toAbsolutePath().normalize();
        // Loaded from the server's own folder: computers cannot write there.
        if (from.startsWith(bin) && from.getParent() != null && from.getParent().equals(bin)) return stem(from);
        List<Shipped> same = index(bin).get((long) bytes.length);
        if (same == null) return null;
        byte[] mine = sha256(bytes);
        for (Shipped s : same) {
            try {
                if (MessageDigest.isEqual(mine, sha256(Files.readAllBytes(bin.resolve(s.name + ".wasm"))))) return s.name;
            } catch (IOException e) {
                // gone since it was listed: not a match
            }
        }
        return null;
    }

    /**
     * May the program {@code identity} use the secret {@code name}? Secrets are
     * named {@code <owner>.<rest>}: a program owns the names that start with its
     * own name. The package programs ({@code pkg} and the App Store) share
     * {@code pkg.*}; scripts run by the interpreters share {@code script.*}.
     */
    public static boolean mayUse(String identity, String name) {
        if (identity == null || !validName(name)) return false;
        int dot = name.indexOf('.');
        if (dot <= 0) return false;
        String owner = name.substring(0, dot);
        return switch (identity) {
            case "pkg", "store" -> owner.equals("pkg");
            case "python", "lua" -> owner.equals("script");
            default -> owner.equals(identity);
        };
    }

    /** 1..64 of {@code [a-z0-9._-]}. */
    public static boolean validName(String name) {
        return name != null && !name.isEmpty() && name.length() <= 64
                && name.chars().allMatch(c -> (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-');
    }
}
