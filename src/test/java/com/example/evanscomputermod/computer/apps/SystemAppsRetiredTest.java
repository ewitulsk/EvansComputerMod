package com.example.evanscomputermod.computer.apps;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

/**
 * A system app the mod no longer ships (the Card Designer) is taken off a computer that
 * has the provisioner's copy, and its {@code /var/apps} data stays. Control: a package of
 * the same id that the player installed (its manifest is not a system app's) is left alone.
 */
class SystemAppsRetiredTest {
    private Path root;

    @AfterEach
    void cleanUp() throws Exception {
        if (root != null) {
            try (var walk = Files.walk(root)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    @Test
    void theCardDesignerIsRemovedButItsCardsStay() throws Exception {
        root = Files.createTempDirectory("ecm-sysapps-retired");
        Path disk = Files.createDirectories(root.resolve("disk"));
        PermissionStore store = new PermissionStore(disk);
        Path app = Files.createDirectories(disk.resolve("apps/dev.ecm.carddesigner"));
        Files.writeString(app.resolve("app.toml"),
                "# A system app: installed by the computer itself (SystemApps). Python, run by the python program.\n"
                        + "id          = \"dev.ecm.carddesigner\"\n");
        Files.writeString(app.resolve("main.py"), "print('designer')\n");
        Path cards = Files.createDirectories(disk.resolve("var/apps/dev.ecm.carddesigner"));
        Files.writeString(cards.resolve("acme.card"), "{}");

        SystemApps.provision(disk, name -> null, store);
        assertFalse(Files.exists(app), "the retired system app is removed");
        assertTrue(Files.isRegularFile(cards.resolve("acme.card")), "its saved cards stay");

        // Control: the player's own package under that id is not the provisioner's to remove.
        Files.createDirectories(app);
        Files.writeString(app.resolve("app.toml"), "id = \"dev.ecm.carddesigner\"\nentry = \"main.py\"\n");
        SystemApps.provision(disk, name -> null, store);
        assertTrue(Files.isRegularFile(app.resolve("app.toml")), "a player-installed package stays");
    }
}
