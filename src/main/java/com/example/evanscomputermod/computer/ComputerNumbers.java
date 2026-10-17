package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.server.MinecraftServer;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import java.util.WeakHashMap;

/**
 * Each computer's number: small, unique in its world and kept for the
 * computer's life (its storage UUID never changes). Programs read it with
 * {@code computer_number} ({@code os.getComputerID()}, {@code computer.id()}).
 * Numbers are handed out in order, from 0, the first time a computer asks,
 * and kept in {@code <world>/computer-data/numbers.txt} ("uuid=number" lines).
 */
public final class ComputerNumbers {
    private ComputerNumbers() {}

    private static final class Table {
        final Path file;
        final Map<UUID, Integer> numbers = new HashMap<>();
        int next;

        Table(Path file) {
            this.file = file;
            try {
                if (Files.isRegularFile(file)) {
                    for (String line : Files.readAllLines(file, StandardCharsets.UTF_8)) {
                        int eq = line.indexOf('=');
                        if (eq <= 0) continue;
                        try {
                            int n = Integer.parseInt(line.substring(eq + 1).trim());
                            numbers.put(UUID.fromString(line.substring(0, eq).trim()), n);
                            next = Math.max(next, n + 1);
                        } catch (IllegalArgumentException ignored) {
                            // A damaged line: skip it.
                        }
                    }
                }
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("Cannot read computer numbers {}: {}", file, e.getMessage());
            }
        }

        int of(UUID id) {
            Integer n = numbers.get(id);
            if (n != null) return n;
            int assigned = next++;
            numbers.put(id, assigned);
            try {
                Files.createDirectories(file.getParent());
                Files.writeString(file, id + "=" + assigned + "\n", StandardCharsets.UTF_8,
                        java.nio.file.StandardOpenOption.CREATE, java.nio.file.StandardOpenOption.APPEND);
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("Cannot save computer number {}: {}", file, e.getMessage());
            }
            return assigned;
        }
    }

    private static final Map<Object, Table> TABLES = new WeakHashMap<>();
    /** Key for computers outside a server (embedded hosts, kernel tests). */
    private static final Object NO_SERVER = new Object();

    /** The number of computer {@code id} in {@code server}'s world (null: outside a world). */
    public static int of(MinecraftServer server, UUID id) {
        if (id == null) return -1;
        synchronized (TABLES) {
            Table t = TABLES.computeIfAbsent(server == null ? NO_SERVER : server, s -> {
                Path dir = ComputerStorage.path(server, new UUID(0, 0)).getParent();
                return new Table(dir.resolve("numbers.txt"));
            });
            return t.of(id);
        }
    }
}
