package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.EvansComputerMod;
import org.jetbrains.annotations.Nullable;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;

/**
 * Saved app state (UI_PROTOCOL.md §11): the bytes of an app's last
 * {@code SaveState}, kept in {@code /var/apps/<id>/state} on the computer's
 * disk and handed back as {@code Restore} after a cold start. The file stays
 * until the app saves again, so a crash after a restore loses nothing. Also
 * remembers which apps were running at the last shutdown
 * ({@code /var/lib/apps/session}, one id per line).
 */
public final class AppStateStore {

    /** Largest state accepted (the wire limit of SaveState). */
    public static final int MAX_STATE = 262144;

    private final Path root;

    public AppStateStore(Path storageRoot) {
        this.root = storageRoot;
    }

    public Path stateFile(String appId) {
        return root.resolve("var/apps").resolve(appId).resolve("state");
    }

    /** Persist an app's state (atomically: a half-written file never replaces a good one). */
    public boolean save(String appId, byte[] data) {
        if (data == null || data.length > MAX_STATE || !AppManifest.validId(appId)) return false;
        Path f = stateFile(appId);
        try {
            Files.createDirectories(f.getParent());
            Path tmp = f.resolveSibling("state.tmp");
            Files.write(tmp, data);
            Files.move(tmp, f, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            return true;
        } catch (IOException e) {
            try {
                Files.write(f, data);
                return true;
            } catch (IOException e2) {
                EvansComputerMod.LOGGER.warn("[apps] cannot save state of {}: {}", appId, e2.toString());
                return false;
            }
        }
    }

    @Nullable
    public byte[] load(String appId) {
        if (!AppManifest.validId(appId)) return null;
        Path f = stateFile(appId);
        try {
            return Files.isRegularFile(f) ? Files.readAllBytes(f) : null;
        } catch (IOException e) {
            return null;
        }
    }

    public void delete(String appId) {
        try {
            Files.deleteIfExists(stateFile(appId));
        } catch (IOException ignored) {
        }
    }

    private Path sessionFile() {
        return root.resolve("var/lib/apps/session");
    }

    /** Record the apps running at shutdown. */
    public void saveSession(List<String> appIds) {
        try {
            Files.createDirectories(sessionFile().getParent());
            Files.writeString(sessionFile(), String.join("\n", appIds) + (appIds.isEmpty() ? "" : "\n"), StandardCharsets.UTF_8);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[apps] cannot save the session list: {}", e.toString());
        }
    }

    /** Apps that were running at the last shutdown (empty if none recorded). */
    public List<String> lastSession() {
        List<String> out = new ArrayList<>();
        try {
            if (Files.isRegularFile(sessionFile())) {
                for (String l : Files.readAllLines(sessionFile(), StandardCharsets.UTF_8)) {
                    if (AppManifest.validId(l.trim()) && !out.contains(l.trim())) out.add(l.trim());
                }
            }
        } catch (IOException ignored) {
        }
        return out;
    }
}
