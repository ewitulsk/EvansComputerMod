package com.example.evanscomputermod.computer.apps;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The Java app.toml parser on the vectors the Rust one ({@code cargo test -p pkg})
 * runs too: {@code ok_*} parse, {@code bad_<field>_*} fail with that field.
 */
public class AppManifestVectorsTest {

    private static Path vectors() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.isDirectory(p.resolve("abi/app-manifest-vectors"))) p = p.getParent();
        assertNotNull(p, "abi/app-manifest-vectors not found");
        return p.resolve("abi/app-manifest-vectors");
    }

    @Test
    void sharedVectors() throws Exception {
        int n = 0;
        try (var files = Files.list(vectors())) {
            for (Path f : (Iterable<Path>) files::iterator) {
                String name = f.getFileName().toString();
                if (!name.endsWith(".toml")) continue;
                String src = Files.readString(f);
                if (name.startsWith("ok_")) {
                    assertDoesNotThrow(() -> AppManifest.parse(src), name);
                } else if (name.startsWith("bad_")) {
                    String rest = name.substring(4, name.length() - 5);
                    String field = rest.substring(0, rest.lastIndexOf('_'));
                    var e = assertThrows(AppManifest.ManifestException.class, () -> AppManifest.parse(src), name);
                    assertEquals(field, e.field, name + ": " + e.getMessage());
                }
                n++;
            }
        }
        assertTrue(n >= 10, "only " + n + " vectors");
    }

    @Test
    void fullExampleFields() throws Exception {
        AppManifest m = AppManifest.parse(Files.readString(vectors().resolve("ok_full.toml")));
        assertEquals("dev.evan.notes", m.id());
        assertEquals(List.of(".txt", ".md"), m.openWith());
        assertTrue(m.backgroundRefresh());
        assertTrue(m.permissions().containsAll(List.of("background:refresh", "background:audio", "background:transfer", "network")));
        assertEquals(new AppManifest.Hotkey(6, "sync", "Sync"), m.hotkeys().get(1));
        assertFalse(m.isPython());
    }

    @Test
    void versionOrder() {
        assertTrue(AppManifest.compareVersions("1.2.0", "1.10.0") < 0);
        assertTrue(AppManifest.compareVersions("1.0.0-beta", "1.0.0") < 0);
        assertTrue(AppManifest.compareVersions("2.0.0", "1.9.9") > 0);
        assertEquals(0, AppManifest.compareVersions("1.0.0", "1.0.0"));
    }
}
