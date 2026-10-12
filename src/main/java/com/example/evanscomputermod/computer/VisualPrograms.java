package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;

import org.jetbrains.annotations.Nullable;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * The visual editor's files in a computer's storage: saved programs
 * ({@code visual/<name>.vpl}) and the generated script it runs
 * ({@code visual_program.py}). Works for any host; server side.
 */
public final class VisualPrograms {

    /** Write the generated Python and run it in the computer's shell. */
    public static void run(IComputerHost host, @Nullable ComputerInstance computer, String pythonCode) {
        if (computer == null) return;
        Path computerDir = ComputerStorage.path(host);
        try {
            Files.createDirectories(computerDir);
            Files.writeString(computerDir.resolve("visual_program.py"), pythonCode);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to write visual_program.py", e);
            return;
        }
        computer.sendInput("python visual_program.py\n");
    }

    public static void save(IComputerHost host, String fileName, String jsonContent) {
        String sanitized = sanitize(fileName);
        if (sanitized.isEmpty()) return;
        Path visualDir = ComputerStorage.path(host).resolve("visual");
        try {
            Files.createDirectories(visualDir);
            Files.writeString(visualDir.resolve(sanitized + ".vpl"), jsonContent);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to save visual program '{}'", sanitized, e);
        }
    }

    public static List<String> list(IComputerHost host) {
        Path visualDir = ComputerStorage.path(host).resolve("visual");
        List<String> result = new ArrayList<>();
        if (!Files.isDirectory(visualDir)) return result;
        try (var stream = Files.list(visualDir)) {
            stream.filter(p -> p.toString().endsWith(".vpl"))
                  .map(p -> p.getFileName().toString().replace(".vpl", ""))
                  .sorted()
                  .forEach(result::add);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to list visual programs", e);
        }
        return result;
    }

    @Nullable
    public static String load(IComputerHost host, String fileName) {
        String sanitized = sanitize(fileName);
        if (sanitized.isEmpty()) return null;
        Path file = ComputerStorage.path(host).resolve("visual").resolve(sanitized + ".vpl");
        if (!Files.exists(file)) return null;
        try {
            return Files.readString(file);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to load visual program '{}'", sanitized, e);
            return null;
        }
    }

    private static String sanitize(String name) {
        String sanitized = name.replaceAll("[^a-zA-Z0-9_\\-]", "");
        if (sanitized.length() > 64) sanitized = sanitized.substring(0, 64);
        return sanitized;
    }

    private VisualPrograms() {}
}
