package com.example.evanscomputermod.computer.repo;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.storage.LevelResource;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.SecureRandom;
import java.util.Base64;
import java.util.Properties;
import java.util.UUID;

/**
 * The secret store of the computers (abi/child-abi.toml {@code [imports.keystore]},
 * docs/REPO.md "Secret store"): a small safe per computer and player, kept by
 * the server <em>outside</em> the computer's files, in
 * {@code <world>/computer-secrets/<computer>/<player>.properties}. Nothing a
 * program can read or write on the computer's disk reaches it.
 *
 * <p>This class only stores; {@code ComputerInstance} decides who may ask
 * (a system program, with the secret's player at the computer).
 */
public final class KeyStore {
    public static final int OK = 0;
    public static final int ERR_INVALID = -1;
    public static final int ERR_NOT_FOUND = -2;
    public static final int ERR_FULL = -12;
    public static final int ERR_NOT_PERMITTED = -13;

    public static final int MAX_VALUE = 4096;
    public static final int MAX_NAMES = 64;
    /** Signing keys are kept under this prefix, so a value can never be read back as a key or the reverse. */
    private static final String KEY_PREFIX = "key:";
    private static final String VALUE_PREFIX = "val:";

    private static final SecureRandom RANDOM = new SecureRandom();

    private KeyStore() {}

    static Path file(MinecraftServer server, UUID computer, UUID player) {
        Path root = server == null
                ? Path.of("computer-secrets").toAbsolutePath()
                : server.getWorldPath(LevelResource.ROOT).resolve("computer-secrets").toAbsolutePath();
        return root.resolve(computer.toString()).resolve(player + ".properties").normalize();
    }

    private static Properties load(Path f) {
        Properties p = new Properties();
        if (Files.isRegularFile(f)) {
            try (InputStream in = Files.newInputStream(f)) {
                p.load(in);
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[keystore] cannot read {}: {}", f, e.toString());
            }
        }
        return p;
    }

    private static boolean save(Path f, Properties p) {
        try {
            Files.createDirectories(f.getParent());
            Path tmp = f.resolveSibling(f.getFileName() + ".tmp");
            try (OutputStream out = Files.newOutputStream(tmp)) {
                p.store(out, "Secrets of one player on one computer. Not readable from the game.");
            }
            Files.move(tmp, f, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            return true;
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[keystore] cannot write {}: {}", f, e.toString());
            return false;
        }
    }

    /** The value under {@code name}, or null. */
    public static synchronized byte[] get(MinecraftServer server, UUID computer, UUID player, String name) {
        String v = load(file(server, computer, player)).getProperty(VALUE_PREFIX + name);
        return v == null ? null : Base64.getDecoder().decode(v);
    }

    public static synchronized int put(MinecraftServer server, UUID computer, UUID player, String name, byte[] value) {
        if (value.length > MAX_VALUE) return ERR_INVALID;
        Path f = file(server, computer, player);
        Properties p = load(f);
        if (!p.containsKey(VALUE_PREFIX + name) && p.size() >= MAX_NAMES) return ERR_FULL;
        p.setProperty(VALUE_PREFIX + name, Base64.getEncoder().encodeToString(value));
        return save(f, p) ? OK : ERR_INVALID;
    }

    public static synchronized int delete(MinecraftServer server, UUID computer, UUID player, String name) {
        Path f = file(server, computer, player);
        Properties p = load(f);
        if (p.remove(VALUE_PREFIX + name) != null) save(f, p);
        return OK;
    }

    private static byte[] seed(Properties p, String name) {
        String v = p.getProperty(KEY_PREFIX + name);
        return v == null ? null : Base64.getDecoder().decode(v);
    }

    /** Make an ed25519 key under {@code name} unless there is one; its public key, or null when the store is full. */
    public static synchronized byte[] keygen(MinecraftServer server, UUID computer, UUID player, String name) {
        Path f = file(server, computer, player);
        Properties p = load(f);
        byte[] s = seed(p, name);
        if (s == null) {
            if (p.size() >= MAX_NAMES) return null;
            s = new byte[32];
            RANDOM.nextBytes(s);
            p.setProperty(KEY_PREFIX + name, Base64.getEncoder().encodeToString(s));
            if (!save(f, p)) return null;
        }
        return Ed25519.publicKey(s);
    }

    /** The public key of the signing key under {@code name}, or null. */
    public static synchronized byte[] publicKey(MinecraftServer server, UUID computer, UUID player, String name) {
        byte[] s = seed(load(file(server, computer, player)), name);
        return s == null ? null : Ed25519.publicKey(s);
    }

    /** A signature with the key under {@code name}, or null when there is none. The key stays here. */
    public static synchronized byte[] sign(MinecraftServer server, UUID computer, UUID player, String name, byte[] message) {
        byte[] s = seed(load(file(server, computer, player)), name);
        return s == null ? null : Ed25519.sign(s, message);
    }
}
