package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;

import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.storage.LevelResource;

import java.io.IOException;
import java.nio.file.*;
import java.util.Map;
import java.util.UUID;
import java.util.WeakHashMap;

/** World-scoped storage. A legacy directory is copied once, never overwritten. */
public final class ComputerStorage {
    private ComputerStorage() {}

    /**
     * Each server's world directory, resolved once. Computers boot on loader
     * threads (Terminals, ePhones and Tech Village computers at the same time),
     * and {@link MinecraftServer#getWorldPath} is not thread-safe: it memoises
     * into a plain {@code HashMap} ({@code LevelStorageAccess.getLevelPath}),
     * so two boots resolving it at once threw a ConcurrentModificationException.
     * The root is bound on the server thread at server start ({@link #bind});
     * a lookup before that resolves it under this map's lock.
     */
    private static final Map<MinecraftServer, Path> WORLD_ROOTS = new WeakHashMap<>();

    /** Resolve and remember the server's world directory. Call on the server thread (server start). */
    public static void bind(MinecraftServer server) {
        worldRoot(server);
    }

    private static Path worldRoot(MinecraftServer server) {
        synchronized (WORLD_ROOTS) {
            return WORLD_ROOTS.computeIfAbsent(server,
                    s -> s.getWorldPath(LevelResource.ROOT).toAbsolutePath().normalize());
        }
    }

    public static Path path(IComputerHost host) {
        return path(host.getServer(), host.getComputerId());
    }

    public static Path path(MinecraftServer server, UUID id) {
        Path legacy = Path.of("computer-data", id.toString()).toAbsolutePath().normalize();
        if (server == null) return legacy; // Embedded hosts and isolated kernel tests.
        Path destination = worldRoot(server)
                        .resolve("computer-data")
                        .resolve(id.toString()).toAbsolutePath().normalize();
        if (!Files.exists(destination) && Files.isDirectory(legacy)) {
            try (var files = Files.walk(legacy)) {
                for (Path source : files.toList()) {
                    Path target = destination.resolve(legacy.relativize(source));
                    if (Files.isDirectory(source)) Files.createDirectories(target);
                    else if (!Files.exists(target)) Files.copy(source, target);
                }
            } catch (IOException e) {
                throw new IllegalStateException("Cannot migrate computer storage " + id, e);
            }
        }
        return destination;
    }
}
