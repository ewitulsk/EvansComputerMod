package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.upload.UploadBackend;
import com.example.evanscomputermod.api.upload.UploadBackends;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Blocks;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * File upload (abi/child-abi.toml {@code [imports.upload]}): a player copies a
 * file from their real computer into a computer in the game. Spawnable with
 * {@code /ecm scenario spawn <name>} and run as GameTests ({@code ecm_upload}).
 * Upload needs the {@code evanscomputermod-upload} sidecar on the server and in
 * the player's game (the runner's {@code -Upload}); without it on the server both
 * scenarios stop at their first step and say so.
 *
 * <p>Exception (docs/SCENARIO_EXCEPTIONS.md, "upload file picker"): the scenario's
 * player has no game of its own, so nobody can click in a system file picker.
 * Each pick is armed with {@link UploadBackend#simulatePick}: the file the player
 * would choose on their computer (written into {@code <server>/scenario-uploads/}),
 * and the server feeds it through the same handlers a game's packets reach. The
 * player's own steps (typing {@code upload}, clicking Upload in Files) are real.
 * The real game's picker and network path are covered by the hidden-client check
 * ({@code scripts/Test.ps1 -ClientChecks -ClientSuite upload}).
 *
 * <ul>
 *   <li>{@code upload_terminal}: on the console, {@code upload status} first says the
 *       player's game lacks the add-on (control: no pick armed); then {@code upload}
 *       puts postcard.txt in /home and {@code cat} shows it; the same name again
 *       becomes "postcard (1).txt" (control); {@code upload /nope} names the missing
 *       directory (control); a file over the server's 16 MiB limit is refused and
 *       leaves nothing behind (control); Python's and Lua's {@code shell.upload}
 *       each bring in a file.</li>
 *   <li>{@code upload_files_app}: on the desktop, Files shows an Upload button in
 *       /home; clicking it brings photo.png in, and Files lists it.</li>
 *   <li>{@code upload_phone}: an ePhone held by the player opens Files from its home
 *       grid (in /home); its ... menu offers Upload; a tap brings selfie.txt in.</li>
 * </ul>
 */
public final class UploadScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    private static final BlockPos PC = new BlockPos(0, 1, 0);
    static final String POSTCARD = "Greetings from the real world!";

    private UploadScenarios() {}

    /** Null when the server has the upload add-on. */
    static String uploadInstalled(ScenarioRun r) {
        if (UploadBackends.available()) return null;
        r.fail("upload needs the evanscomputermod-upload sidecar in the server's mods folder (scripts/Test.ps1 -Upload)");
        return "no upload backend";
    }

    /** "The player's real computer": a folder outside the world. */
    static Path home(ScenarioRun r) throws IOException {
        Path dir = r.level().getServer().getServerDirectory().resolve("scenario-uploads");
        Files.createDirectories(dir);
        return dir;
    }

    /** The player picks {@code name} (with {@code text}) in their system's file picker [exception, see the class]. */
    static void pick(ScenarioRun r, String name, byte[] content) {
        try {
            Path f = home(r).resolve(name);
            Files.write(f, content);
            if (!UploadBackends.get().simulatePick(r.player().entity(), f)) r.fail("the upload backend can't simulate a pick");
        } catch (IOException e) {
            r.fail(e.toString());
        }
    }

    /** The player picks a file of {@code size} bytes (sparse). */
    static void pickLarge(ScenarioRun r, String name, long size) {
        try {
            Path f = home(r).resolve(name);
            try (RandomAccessFile raf = new RandomAccessFile(f.toFile(), "rw")) {
                raf.setLength(size);
            }
            UploadBackends.get().simulatePick(r.player().entity(), f);
        } catch (IOException e) {
            r.fail(e.toString());
        }
    }

    static Path disk(ScenarioRun r) {
        return GuiScenarios.computer(r).storagePath();
    }

    /** Null when {@code /home/<name>} holds {@code text}. */
    static String hasFile(ScenarioRun r, String name, String text) {
        Path p = disk(r).resolve("home").resolve(name);
        try {
            if (!Files.isRegularFile(p)) return "no /home/" + name;
            String s = Files.readString(p, StandardCharsets.UTF_8);
            return s.equals(text) ? null : "/home/" + name + " holds '" + s + "'";
        } catch (IOException e) {
            return e.toString();
        }
    }

    static Scenario.Builder base(String name, String desc) {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -2, 3, 0, 2);
        return Scenario.builder(name, desc)
                .asPlayer()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(floor, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                }, r -> {
                    var u = GuiScenarios.ui(r);
                    if (u != null) u.stop();
                    UploadBackend b = UploadBackends.get();
                    if (b != null) b.simulatePick(r.player().entity(), null);
                }))
                .timeLimit(60_000);
    }

    // ------------------------------------------------------------ upload_terminal

    static Scenario terminal() {
        var b = base("upload_terminal",
                "upload copies a file from the player's real computer into the Terminal's /home; Python and Lua do the same");
        b.note("Exception: the player's pick in their system's file picker is armed for them (they have no game window);"
                + " the file comes from <server>/scenario-uploads/ (docs/SCENARIO_EXCEPTIONS.md, \"upload file picker\").");
        b.await(UploadScenarios::uploadInstalled, "the server has the upload add-on (evanscomputermod-upload)", 1_000);
        b.send("pc", "mkdir -p /home").expect("pc", PlayerKit.PROMPT, "mkdir -p /home");
        b.note("Control: before the player's game is ready to pick (no add-on in it), upload says so");
        b.send("pc", "upload status")
                .expect("pc", "^upload: your game doesn't have the upload add-on", "upload status: the player's game lacks the add-on",
                        "^upload: ready")
                .expect("pc", PlayerKit.PROMPT, "shell");
        b.note("Upload postcard.txt from the player's computer into /home");
        b.mutate(r -> pick(r, "postcard.txt", POSTCARD.getBytes(StandardCharsets.UTF_8)), "the player will pick postcard.txt")
                .send("pc", "upload")
                .expect("pc", "^upload: saved /home/postcard\\.txt$", "upload saves /home/postcard.txt", "^upload: (?!pick|saved)")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .await(r -> hasFile(r, "postcard.txt", POSTCARD), "the file on the computer has the same bytes", 2_000)
                .send("pc", "cat /home/postcard.txt")
                .expect("pc", "^Greetings from the real world!", "cat shows it")
                .expect("pc", PlayerKit.PROMPT, "shell");
        b.note("Control: the same name again doesn't overwrite it");
        b.mutate(r -> pick(r, "postcard.txt", "second".getBytes(StandardCharsets.UTF_8)), "the player picks postcard.txt again")
                .send("pc", "upload /home")
                .expect("pc", "^upload: saved /home/postcard \\(1\\)\\.txt$", "it becomes 'postcard (1).txt'", "^upload: (?!pick|saved)")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .await(r -> hasFile(r, "postcard.txt", POSTCARD), "the first postcard is unchanged", 2_000);
        b.note("Control: a directory that isn't there");
        b.mutate(r -> pick(r, "postcard.txt", "x".getBytes(StandardCharsets.UTF_8)), "a pick is ready")
                .send("pc", "upload /nope")
                .expect("pc", "^upload: /nope: no such directory", "upload names the missing directory", "^upload: saved")
                .expect("pc", PlayerKit.PROMPT, "shell");
        b.note("Control: a file over the server's limit (16 MiB by default) is refused");
        b.mutate(r -> pickLarge(r, "huge.bin", 17L * 1024 * 1024), "the player picks a 17 MiB file")
                .send("pc", "upload")
                .expect("pc", "^upload: the file is larger than this server allows", "refused: too large", "^upload: saved")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .await(r -> {
                    try (var s = Files.list(disk(r).resolve("home"))) {
                        List<String> names = s.map(p -> p.getFileName().toString()).toList();
                        return names.stream().anyMatch(n -> n.startsWith("huge") || n.endsWith(".part")) ? "left behind: " + names : null;
                    } catch (IOException e) {
                        return e.toString();
                    }
                }, "nothing is left behind (no huge.bin, no .part file)", 2_000);
        b.note("Python and Lua: shell.upload_supported() and shell.upload()");
        PlayerKit.typeFiles(b, Map.of("pc", Map.of(
                "/home/up.py", "import shell\nprint(shell.upload_supported())\nprint(\"py got \" + shell.upload(\"/home\"))",
                "/home/up.lua", "local shell = require(\"shell\")\nprint(shell.upload_supported())\nlocal p, err = shell.upload()\nprint(\"lua got \" .. tostring(p or err))")));
        b.mutate(r -> pick(r, "from_python.txt", "py".getBytes(StandardCharsets.UTF_8)), "the player will pick from_python.txt")
                .send("pc", "python /home/up.py")
                .expect("pc", "^ready$", "Python: upload_supported() is ready")
                .expect("pc", "^py got /home/from_python\\.txt$", "Python's shell.upload returns the new file's path", "Error|Traceback")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .await(r -> hasFile(r, "from_python.txt", "py"), "from_python.txt arrived", 2_000);
        b.mutate(r -> pick(r, "from_lua.txt", "lua".getBytes(StandardCharsets.UTF_8)), "the player will pick from_lua.txt")
                .send("pc", "lua /home/up.lua")
                .expect("pc", "^ready$", "Lua: upload_supported() is ready")
                .expect("pc", "^lua got /home/from_lua\\.txt$", "Lua's shell.upload returns the new file's path", "error")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .await(r -> hasFile(r, "from_lua.txt", "lua"), "from_lua.txt arrived", 2_000);
        return b.build();
    }

    // ------------------------------------------------------------ upload_files_app

    static Scenario filesApp() {
        var b = base("upload_files_app", "Files on the desktop has an Upload button that brings a file from the player's computer");
        b.note("Exception: the player's pick in their system's file picker is armed for them (they have no game window);"
                + " docs/SCENARIO_EXCEPTIONS.md, \"upload file picker\".");
        b.await(UploadScenarios::uploadInstalled, "the server has the upload add-on (evanscomputermod-upload)", 1_000);
        b.send("pc", "mkdir -p /home").expect("pc", PlayerKit.PROMPT, "mkdir -p /home");
        b.mutate(r -> pick(r, "photo.png", "not really a png".getBytes(StandardCharsets.UTF_8)), "the player will pick photo.png");
        b.send("pc", "gui")
                .expect("pc", "^gui: started", "the GUI starts with the desktop", "(?i)unknown command|no GUI device")
                .await(r -> GuiScenarios.comp(r) != null && GuiScenarios.comp(r).shellConnected() ? null : "no desktop yet",
                        "the desktop is connected", 15_000);
        GuiScenarios.launch(b, "Files", "files")
                .await(r -> GuiScenarios.visible(r, "files", "home"), "Files lists the disk", 10_000)
                .mutate(r -> AppScenarios.doubleClick(r, "files", "home"), "double-click home")
                .await(r -> GuiScenarios.shows(r, "files", "0 items"), "Files shows the empty /home", 10_000)
                .await(r -> GuiScenarios.visible(r, "files", "Upload"), "the Upload button is there (the player can upload)", 5_000)
                .mutate(r -> GuiScenarios.clickText(r, "files", "Upload"), "click Upload")
                .await(r -> GuiScenarios.shows(r, "files", "Uploaded photo.png"), "Files says it uploaded photo.png", 10_000)
                .await(r -> GuiScenarios.visible(r, "files", "photo.png"), "and lists it", 5_000)
                .await(r -> hasFile(r, "photo.png", "not really a png"), "the file on the computer has the same bytes", 2_000)
                .mutate(r -> GuiScenarios.snapshot(r, "upload_files_app"), "snapshot: Files after the upload");
        return b.build();
    }

    // ------------------------------------------------------------ upload_phone

    static final String FILES = "dev.ecm.files";

    static Scenario phone() {
        var b = PhoneGuiScenarios.start("upload_phone",
                "the ePhone's Files (on its home grid) uploads a file from the holder's computer into its documents");
        b.note("Exception: the player's pick in their system's file picker is armed for them (they have no game window);"
                + " docs/SCENARIO_EXCEPTIONS.md, \"upload file picker\".");
        b.await(UploadScenarios::uploadInstalled, "the server has the upload add-on (evanscomputermod-upload)", 1_000);
        b.mutate(r -> pick(r, "selfie.txt", "cheese".getBytes(StandardCharsets.UTF_8)), "the player will pick selfie.txt");
        PhoneGuiScenarios.openApp(b, "Files", FILES);
        b.await(r -> {
            var w = PhoneGuiScenarios.onScreen(r, FILES);
            return w != null && PhoneGuiScenarios.comp(r).textsOf(w.id()).stream().anyMatch(s -> s.endsWith("items") || s.endsWith("item"))
                    ? null : "Files hasn't listed a folder";
        }, "Files opens in the documents folder (/home)", 10_000);
        b.mutate(r -> PhoneGuiScenarios.tap(r, PhoneGuiScenarios.comp(r).locateText(PhoneGuiScenarios.onScreen(r, FILES).id(), "...", false)),
                "tap ... (Files' menu)");
        b.await(r -> PhoneGuiScenarios.comp(r).locatePopupText("Upload", false) != null ? null : "no Upload in the menu",
                "the menu offers Upload (the holder's game can upload)", 5_000);
        b.mutate(r -> PhoneGuiScenarios.tap(r, PhoneGuiScenarios.comp(r).locatePopupText("Upload", false)), "tap Upload");
        b.await(r -> PhoneGuiScenarios.comp(r).textsOf(PhoneGuiScenarios.onScreen(r, FILES).id()).stream().anyMatch(s -> s.contains("Uploaded selfie.txt"))
                ? null : "texts " + PhoneGuiScenarios.comp(r).textsOf(PhoneGuiScenarios.onScreen(r, FILES).id()), "Files says it uploaded selfie.txt", 10_000);
        b.await(r -> {
            try {
                Path f = PhoneGuiScenarios.disk(r).resolve("home/selfie.txt");
                return Files.isRegularFile(f) && Files.readString(f).equals("cheese") ? null : "no /home/selfie.txt on the phone";
            } catch (IOException e) {
                return e.toString();
            }
        }, "the phone's /home/selfie.txt has the same bytes", 2_000);
        b.mutate(r -> VideoScenarios.snapshot(r, PhoneGuiScenarios.comp(r), "upload_phone"), "snapshot: Files on the phone after the upload");
        b.timeLimit(60_000);
        return b.build();
    }

    static {
        for (Scenario s : List.of(terminal(), filesApp(), phone())) ALL.put(s.name, s);
    }
}
//?}
