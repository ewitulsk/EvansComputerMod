package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.EvansComputerMod;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.function.Function;

/**
 * Apps every computer has (docs/gui/APPS.md "System apps"): installed into
 * {@code /apps/<id>/} from the mod's programs when the app manager first
 * indexes, with their declared permissions allowed once (the player can take
 * them back in Settings). Today that is the App Store ({@code dev.ecm.store},
 * program {@code store}, permissions {@code packages} and {@code network}).
 *
 * <p>A system app is copied again when its program changes size or its
 * folder is missing; {@code pkg remove} of one lasts until the next boot.
 */
public final class SystemApps {

    /** One system app: its manifest fields, its program and its icon resource. */
    record App(String id, String name, String program, String summary, List<String> permissions, String iconResource) {
        String manifest() {
            StringBuilder p = new StringBuilder();
            for (String s : permissions) p.append(p.isEmpty() ? "" : ", ").append('"').append(s).append('"');
            return "# A system app: installed by the computer itself.\n"
                    + "id          = \"" + id + "\"\n"
                    + "name        = \"" + name + "\"\n"
                    + "version     = \"1.0.0\"\n"
                    + "entry       = \"main.wasm\"\n"
                    + "summary     = \"" + summary + "\"\n"
                    + "publisher   = \"ECM\"\n"
                    + "icon        = \"icon.png\"\n"
                    + "permissions = [" + p + "]\n";
        }
    }

    static final List<App> ALL = List.of(
            new App("dev.ecm.store", "App Store", "store",
                    "Find, install, update and remove apps from repositories on the network.",
                    List.of("packages", "network"), "/evanscomputermod/apps/store.png"));

    private SystemApps() {}

    /** Install or refresh the system apps under {@code root}. Returns how many were (re)written. */
    static int provision(Path root, Function<String, Path> programs, PermissionStore store) {
        int n = 0;
        for (App a : ALL) {
            Path program = programs == null ? null : programs.apply(a.program());
            if (program == null || !Files.isRegularFile(program)) continue;
            Path dir = root.resolve("apps").resolve(a.id());
            try {
                Path main = dir.resolve("main.wasm");
                Path toml = dir.resolve("app.toml");
                String manifest = a.manifest();
                boolean fresh = !Files.isRegularFile(toml);
                boolean stale = fresh || !Files.isRegularFile(main) || Files.size(main) != Files.size(program)
                        || !Files.readString(toml, StandardCharsets.UTF_8).equals(manifest);
                if (!stale) continue;
                Files.createDirectories(dir);
                Files.copy(program, main, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                Files.writeString(toml, manifest, StandardCharsets.UTF_8);
                try (InputStream in = SystemApps.class.getResourceAsStream(a.iconResource())) {
                    if (in != null) Files.write(dir.resolve("icon.png"), in.readAllBytes());
                }
                Files.createDirectories(root.resolve("var/apps").resolve(a.id()));
                if (fresh) {
                    // Allowed once at install; Settings / pkg perm can take it back.
                    for (String p : a.permissions()) {
                        if (store.decision(a.id(), p) == null) store.setDecision(a.id(), p, true);
                    }
                }
                n++;
                EvansComputerMod.LOGGER.info("[apps] system app {} installed in /apps/{}", a.name(), a.id());
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[apps] cannot install system app {}: {}", a.id(), e.toString());
            }
        }
        return n;
    }
}
