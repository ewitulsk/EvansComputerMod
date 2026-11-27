package com.example.evanscomputermod.computer.repo;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.storage.LevelResource;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.UUID;
import java.util.WeakHashMap;

/**
 * The computers that may use the server's link to the package repository when
 * {@code repo.access = datacenter}: the Tech Villages' repository servers and
 * the ones an operator names with {@code /ecm repo grant}. Kept per world in
 * {@code <world>/ecm-repo-servers.txt}, one computer id per line.
 */
public final class RepoServers {
    private static final String FILE = "ecm-repo-servers.txt";
    private static final Map<MinecraftServer, Set<UUID>> LOADED = new WeakHashMap<>();
    /** Without a server (kernel tests). */
    private static final Set<UUID> DETACHED = new TreeSet<>();

    private RepoServers() {}

    private static Path file(MinecraftServer server) {
        return server.getWorldPath(LevelResource.ROOT).resolve(FILE);
    }

    private static Set<UUID> set(MinecraftServer server) {
        if (server == null) return DETACHED;
        return LOADED.computeIfAbsent(server, s -> {
            Set<UUID> ids = new TreeSet<>();
            try {
                Path f = file(s);
                if (Files.isRegularFile(f)) {
                    for (String line : Files.readAllLines(f)) {
                        String l = line.trim();
                        if (l.isEmpty() || l.startsWith("#")) continue;
                        try {
                            ids.add(UUID.fromString(l));
                        } catch (IllegalArgumentException e) {
                            EvansComputerMod.LOGGER.warn("[repo] {}: not a computer id: {}", FILE, l);
                        }
                    }
                }
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[repo] cannot read {}: {}", FILE, e.toString());
            }
            return ids;
        });
    }

    private static void save(MinecraftServer server, Set<UUID> ids) {
        if (server == null) return;
        StringBuilder sb = new StringBuilder("# Computers that may use the server's link to the package repository (repo.access = datacenter).\n");
        for (UUID id : ids) sb.append(id).append('\n');
        try {
            Files.writeString(file(server), sb.toString());
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[repo] cannot write {}: {}", FILE, e.toString());
        }
    }

    public static synchronized boolean isServer(MinecraftServer server, UUID computer) {
        return computer != null && set(server).contains(computer);
    }

    /** True if it was not one already. */
    public static synchronized boolean grant(MinecraftServer server, UUID computer) {
        Set<UUID> ids = set(server);
        boolean added = ids.add(computer);
        if (added) save(server, ids);
        return added;
    }

    public static synchronized boolean revoke(MinecraftServer server, UUID computer) {
        Set<UUID> ids = set(server);
        boolean removed = ids.remove(computer);
        if (removed) save(server, ids);
        return removed;
    }

    public static synchronized Set<UUID> list(MinecraftServer server) {
        return new TreeSet<>(set(server));
    }
}
