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
 * permissions, {@code open_with} the text types: what Files opens text with) and the
 * Video Player ({@code dev.ecm.video}, program {@code videoplayer}, permissions
 * {@code files:documents} and {@code peripherals:speaker}, {@code open_with} the
 * video types; only where a video backend, the server-side {@code evanscomputermod-ffmpeg}
 * sidecar, is installed, and removed again when it no longer is) and the Browser
 * ({@code dev.ecm.browser}, program {@code browser}, permissions {@code network} and
 * {@code files:documents}).
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
 * <p>The <b>Card Designer</b> ({@code dev.ecm.carddesigner}, docs/cards/CARDS.md) is a Python app:
 * its {@code app.toml}, {@code main.py} and icon ship in the mod's resources
 * ({@code assets/evanscomputermod/ecm_apps/dev.ecm.carddesigner}) and the app manager runs it
 * with the {@code python} program, like any {@code .py} package.
 *
 * <p>A system app is copied again when its program changes size (a Python entry: when its
 * text changes) or its folder is missing; {@code pkg remove} of one lasts until the next boot.
 */
public final class SystemApps {

    /**
     * One system app: its manifest fields, its program and its icon resource. A Python app
     * ({@link #python}) ships whole in the mod's resources instead: {@code resourceDir} holds
     * its {@code app.toml} (the manifest as written), {@code main.py} (the entry, run by the
     * {@code python} program) and {@code icon.png}.
     */
    record App(String id, String name, String program, String summary, List<String> permissions, List<String> openWith,
               String iconResource, List<String> requires, String resourceDir) {
        App(String id, String name, String program, String summary, List<String> permissions, List<String> openWith,
            String iconResource, List<String> requires) {
            this(id, name, program, summary, permissions, openWith, iconResource, requires, null);
        }

        App(String id, String name, String program, String summary, List<String> permissions, List<String> openWith, String iconResource) {
            this(id, name, program, summary, permissions, openWith, iconResource, List.of());
        }

        /**
         * A Python system app from {@code /assets/evanscomputermod/ecm_apps/<id>/} ({@code app.toml},
         * {@code main.py}, {@code icon.png}); {@code permissions} are the ones its manifest declares,
         * allowed once at install like the others. It needs the {@code python} program.
         */
        static App python(String id, String name, String summary, List<String> permissions) {
            String dir = "/assets/evanscomputermod/ecm_apps/" + id;
            return new App(id, name, "python", summary, permissions, List.of(), dir + "/icon.png", List.of(), dir);
        }

        boolean isPython() {
            return resourceDir != null;
        }

        /** Installed on this computer: always, or when one of the {@code requires} files exists. */
        boolean wanted(Path root) {
            if (requires.isEmpty()) return true;
            for (String r : requires) if (Files.exists(root.resolve(r))) return true;
            return false;
        }

        /** A resource of this app's folder (Python apps), or null when the mod doesn't ship it. */
        byte[] resource(String name) {
            if (resourceDir == null) return null;
            try (InputStream in = SystemApps.class.getResourceAsStream(resourceDir + "/" + name)) {
                return in == null ? null : in.readAllBytes();
            } catch (IOException e) {
                return null;
            }
        }

        String manifest() {
            if (isPython()) {
                byte[] toml = resource("app.toml");
                return toml == null ? "" : new String(toml, StandardCharsets.UTF_8);
            }
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

    /** The Video Player's id: offered only where a video backend is installed. */
    static final String VIDEO_PLAYER = "dev.ecm.video";

    /** Video types the Video Player opens (the host decodes them with FFmpeg). */
    static final List<String> VIDEO_TYPES = List.of(".mp4", ".m4v", ".mkv", ".webm", ".mov", ".avi");

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
            // The Video Player (docs/gui/APPS.md "Video Player"): what Files opens videos with, on the
            // desktop and the ePhone alike. It reads the documents folder for its library and plays sound.
            new App(VIDEO_PLAYER, "Video Player", "videoplayer",
                    "Play videos from your documents with sound: seek, skip, loop. Opens .mp4, .mkv, .webm and more.",
                    List.of("files:documents", "peripherals:speaker"), VIDEO_TYPES, "/evanscomputermod/apps/video.png"),
            // The Browser (docs/gui/BROWSER.md): web pages over the network (http://) and from the
            // documents folder (file://), on the desktop and the ePhone alike.
            new App("dev.ecm.browser", "Browser", "browser",
                    "Browse web pages on the network: the Tech Village sites, pages you write and serve with httpd.",
                    List.of("network", "files:documents"), List.of(), "/evanscomputermod/apps/browser.png"),
            // Cellular v2: on the computers that run the service.
            new App("dev.ecm.cellsite", "Cell Site", "cellsite",
                    "This computer's cell site: radio, core link, phones and their signal, settings.",
                    List.of("network"), List.of(), "/evanscomputermod/apps/cellsite.png", List.of("etc/celld-site.toml")),
            new App("dev.ecm.carrier", "Carrier Console", "carrier",
                    "This computer's carrier core: subscribers and SIMs, sites, phones, handovers.",
                    List.of("network"), List.of(), "/evanscomputermod/apps/carrier.png", List.of("etc/cored.db", "etc/cored.toml")),
            // Cards (docs/cards/CARDS.md "Programs and apps"): the Card Designer, a Python app on ecm-ui
            // shipped in the mod's resources (ecm_apps/dev.ecm.carddesigner: app.toml, main.py, icon.png).
            // It is a printer client only; its .card files live in its own /var/apps folder.
            App.python("dev.ecm.carddesigner", "Card Designer",
                    "Draw a card's faces, name it, write its stripe and print it on a Card Printer.",
                    List.of("peripherals:card_printer")));

    private SystemApps() {}

    /** Remove a system app this provisioner installed (its manifest is ours, unchanged). */
    private static void removeSystemApp(Path root, App a) {
        Path dir = root.resolve("apps").resolve(a.id());
        Path toml = dir.resolve("app.toml");
        try {
            if (!Files.isRegularFile(toml) || !Files.readString(toml, StandardCharsets.UTF_8).equals(a.manifest())) return;
            try (var walk = Files.walk(dir)) {
                for (Path p : (Iterable<Path>) walk.sorted(java.util.Comparator.reverseOrder())::iterator) Files.deleteIfExists(p);
            }
            EvansComputerMod.LOGGER.info("[apps] system app {} removed: no video support on this server", a.name());
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[apps] cannot remove system app {}: {}", a.id(), e.toString());
        }
    }

    /** Install or refresh the system apps under {@code root}. Returns how many were (re)written. */
    static int provision(Path root, Function<String, Path> programs, PermissionStore store) {
        int n = 0;
        for (App a : ALL) {
            if (a.id().equals(VIDEO_PLAYER) && !com.example.evanscomputermod.api.video.VideoBackends.available()) {
                // No video support on this server (the evanscomputermod-ffmpeg sidecar): no Video Player,
                // and one installed while the sidecar was there goes away.
                removeSystemApp(root, a);
                continue;
            }
            if (!a.wanted(root)) continue;
            // A Python app needs the python program; a .wasm app is the program itself.
            Path program = programs == null ? null : programs.apply(a.program());
            if (program == null || !Files.isRegularFile(program)) continue;
            byte[] entry = a.isPython() ? a.resource("main.py") : null;
            if (a.isPython() && (entry == null || a.manifest().isEmpty())) continue;
            Path dir = root.resolve("apps").resolve(a.id());
            try {
                Path main = dir.resolve(a.isPython() ? "main.py" : "main.wasm");
                Path toml = dir.resolve("app.toml");
                String manifest = a.manifest();
                boolean fresh = !Files.isRegularFile(toml);
                boolean stale = fresh || !Files.isRegularFile(main)
                        || (a.isPython() ? !java.util.Arrays.equals(Files.readAllBytes(main), entry) : Files.size(main) != Files.size(program))
                        || !Files.readString(toml, StandardCharsets.UTF_8).equals(manifest);
                if (!stale) continue;
                Files.createDirectories(dir);
                if (a.isPython()) Files.write(main, entry);
                else Files.copy(program, main, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
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
