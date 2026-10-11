package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;

import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.storage.LevelResource;

import java.io.IOException;
import java.nio.file.*;
import java.util.UUID;

/** World-scoped storage. A legacy directory is copied once, never overwritten. */
public final class ComputerStorage {
    private ComputerStorage() {}

    public static Path path(IComputerHost host) {
        return path(host.getServer(), host.getComputerId());
    }

    public static Path path(MinecraftServer server, UUID id) {
        Path legacy = Path.of("computer-data", id.toString()).toAbsolutePath().normalize();
        if (server == null) return legacy; // Embedded hosts and isolated kernel tests.
        Path destination =
                server.getWorldPath(LevelResource.ROOT)
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
