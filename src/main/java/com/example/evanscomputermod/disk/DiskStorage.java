package com.example.evanscomputermod.disk;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.ComputerStorage;
import net.minecraft.server.MinecraftServer;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;

/**
 * Where floppy disks keep their files: {@code <world>/computer-data/disks/<n>/},
 * one folder per disk number. A disk gets its number the first time it is
 * put into a drive (numbers go up from 0, kept in {@code disks/next}), so a
 * copy of the item is the same disk, as in ComputerCraft.
 */
public final class DiskStorage {
    private DiskStorage() {}

    private static Path root(MinecraftServer server) {
        return ComputerStorage.path(server, new UUID(0, 0)).getParent().resolve("disks");
    }

    /** The folder of disk {@code id}, made if it isn't there. */
    public static Path folder(MinecraftServer server, int id) {
        Path p = root(server).resolve(Integer.toString(id));
        try {
            Files.createDirectories(p);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("Cannot make disk folder {}: {}", p, e.getMessage());
        }
        return p;
    }

    /** A new disk number. */
    public static synchronized int newId(MinecraftServer server) {
        Path counter = root(server).resolve("next");
        int next = 0;
        try {
            if (Files.isRegularFile(counter)) next = Integer.parseInt(Files.readString(counter, StandardCharsets.UTF_8).trim());
        } catch (IOException | NumberFormatException e) {
            EvansComputerMod.LOGGER.warn("Cannot read {}: {}", counter, e.getMessage());
        }
        try {
            Files.createDirectories(counter.getParent());
            Files.writeString(counter, Integer.toString(next + 1), StandardCharsets.UTF_8);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("Cannot save {}: {}", counter, e.getMessage());
        }
        return next;
    }

    /** Whether disk {@code id}'s folder holds anything. */
    public static boolean hasFiles(MinecraftServer server, int id) {
        Path p = root(server).resolve(Integer.toString(id));
        try (var s = Files.list(p)) {
            return s.findAny().isPresent();
        } catch (IOException e) {
            return false;
        }
    }
}
//?}
