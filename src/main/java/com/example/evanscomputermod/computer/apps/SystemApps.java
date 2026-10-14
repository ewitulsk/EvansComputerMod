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
 * them back in Settings): the App Store ({@code dev.ecm.store}, program
 * {@code store}, permissions {@code packages} and {@code network}) and the
 * Text Editor ({@code dev.ecm.editor}, program {@code editor}, no
 * permissions, {@code open_with} the text types: what Files opens text with).
 *
 * <p>Some system apps belong to a service and are installed only on computers that
 * run it ({@link App#requires}): the Cellular v2 operator apps (docs/gui/CELLULAR_V2.md,
 * "As built: phase 3 network side"), <b>Cell Site</b> ({@code dev.ecm.cellsite}, program
 * {@code cellsite}) once {@code /etc/celld-site.toml} exists and <b>Carrier Console</b>
 * ({@code dev.ecm.carrier}, program {@code carrier}) once {@code /etc/cored.db} or
 * {@code /etc/cored.toml} exists. Both ask for {@code network} (their daemon's admin
 * socket is TCP on 127.0.0.1). {@code celld-site config set} and {@code cored init}
 * ask the app manager to reindex, so the app shows up at once.
 *
 * <p>A system app is copied again when its program changes size or its
 * folder is missing; {@code pkg remove} of one lasts until the next boot.
 */
public final class SystemApps {

    /** One system app: its manifest fields, its program and its icon resource. */
    record App(String id, String name, String program, String summary, List<String> permissions, List<String> openWith,
               String iconResource, List<String> requires) {
        App(String id, String name, String program, String summary, List<String> permissions, List<String> openWith, String iconResource) {
            this(id, name, program, summary, permissions, openWith, iconResource, List.of());
        }

        /** Installed on this computer: always, or when one of the {@code requires} files exists. */
        boolean wanted(Path root) {
            if (requires.isEmpty()) return true;
            for (String r : requires) if (Files.exists(root.resolve(r))) return true;
            return false;
        }

        String manifest() {
            StringBuilder p = new StringBuilder();
            for (String s : permissions) p.append(p.isEmpty() ? "" : ", ").append('"').append(s).append('"');
            StringBuilder o = new StringBuilder();
            for (String s : openWith) o.append(o.isEmpty() ? "" : ", ").append('"').append(s).append('"');
            return "# A system app: installed by the computer itself.\n"
                    + "id          = \"" + id + "\"\n"
                    + "name        = \"" + name + "\"\n"
                    + "version     = \"1.0.0\"\n"
                    + "entry       = \"main.wasm\"\n"
                    + "summary     = \"" + summary + "\"\n"
                    + "publisher   = \"ECM\"\n"
                    + "icon        = \"icon.png\"\n"
                    + "permissions = [" + p + "]\n"
                    + (openWith.isEmpty() ? "" : "open_with   = [" + o + "]\n");
        }
    }

    /** Text types the Text Editor opens (Files' double-click, {@code Open{path}}, {@code pkg open}). */
    static final List<String> TEXT_TYPES = List.of(".txt", ".md", ".py", ".json", ".toml", ".sh", ".rs", ".cfg", ".conf",
            ".ini", ".log", ".csv", ".xml", ".html", ".css", ".js", ".lua", ".yml", ".yaml");

    static final List<App> ALL = List.of(
            new App("dev.ecm.store", "App Store", "store",
                    "Find, install, update and remove apps from repositories on the network.",
                    List.of("packages", "network"), List.of(), "/evanscomputermod/apps/store.png"),
            // The Text Editor as an app: what Files opens text with. It asks for no permissions, so it can
            // read and write only the file it was handed (one-file grant) and its own /var/apps data.
            // The launcher's Text Editor is the built-in program (unconfined, like the Terminal).
            new App("dev.ecm.editor", "Text Editor", "editor",
                    "Edit text files. Opened from Files, it can only read and write the file it was given.",
                    List.of(), TEXT_TYPES, "/evanscomputermod/apps/editor.png"),
            // Cellular v2: on the computers that run the service.
            new App("dev.ecm.cellsite", "Cell Site", "cellsite",
                    "This computer's cell site: radio, core link, phones and their signal, settings.",
                    List.of("network"), List.of(), "/evanscomputermod/apps/cellsite.png", List.of("etc/celld-site.toml")),
            new App("dev.ecm.carrier", "Carrier Console", "carrier",
                    "This computer's carrier core: subscribers and SIMs, sites, phones, handovers.",
                    List.of("network"), List.of(), "/evanscomputermod/apps/carrier.png", List.of("etc/cored.db", "etc/cored.toml")));

    private SystemApps() {}

    /** Install or refresh the system apps under {@code root}. Returns how many were (re)written. */
    static int provision(Path root, Function<String, Path> programs, PermissionStore store) {
        int n = 0;
        for (App a : ALL) {
            if (!a.wanted(root)) continue;
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
