package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * Video (docs/gui/APPS.md "Video Player", the {@code video} command), spawnable
 * with {@code /ecm scenario spawn <name>} and run as GameTests
 * ({@code video_desktop} in {@code ecm_apps}, {@code video_phone} in
 * {@code ecm_phone}). The video is a 6 s, 320x180 test pattern with a 440 Hz
 * tone, copied into the computer's folder (a binary file: the standing
 * exception in docs/SCENARIO_EXCEPTIONS.md).
 *
 * <ul>
 *   <li>{@code video_desktop}: a Terminal with a Speaker beside it. On the
 *       console, {@code video info}, {@code video ls} and {@code video play}
 *       (the picture on the Terminal's screen, the tone on the Speaker; control:
 *       a missing file). Then {@code gui}: Files from the launcher, double-click
 *       the video, and the Video Player plays it with sound; Pause stops the
 *       clock (control: the time stays); the library lists the video and not the
 *       typed text file beside it (control).</li>
 *   <li>{@code video_phone}: an ePhone held by the player. The Video Player is
 *       on the home grid; its library lists the video; a tap plays it on the
 *       phone's speaker; hotkey slot 1 pauses (control: the time stays) and
 *       plays; home (Alt+H) suspends it after it saved the file and position;
 *       opened again, it is paused at the same place.</li>
 * </ul>
 */
public final class VideoScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String APP = "dev.ecm.video";
    static final String VIDEO = "test_pattern.mp4";
    static final String VIDEO_PATH = "/home/videos/" + VIDEO;
    static final String RESOURCES = "/evanscomputermod/scenarios/video/";
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos SPEAKER = PC.west();

    /** Per run: speaker samples, a time shown, a picture checksum. */
    static final class State {
        long samples;
        String time = "";
        long crc;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private VideoScenarios() {}

    // ------------------------------------------------------------ shared checks

    /** The "m:ss / m:ss" text in a window, or "". */
    static String timeText(Compositor c, Compositor.WindowInfo w) {
        if (c == null || w == null) return "";
        return c.textsOf(w.id()).stream().filter(t -> t.contains(" / ")).findFirst().orElse("");
    }

    /** CRC of the composited frame inside a window's picture area (below its top bar, above its controls). */
    static long pictureCrc(Compositor c, Compositor.WindowInfo w) {
        c.repaintAll();
        byte[] f = c.frameRgba();
        int dw = c.display().deviceW();
        java.util.zip.CRC32 crc = new java.util.zip.CRC32();
        for (int y = w.y() + 24; y < w.y() + w.h() - 50; y++) {
            int at = (y * dw + w.x()) * 4;
            if (at >= 0 && at + w.w() * 4 <= f.length) crc.update(f, at, w.w() * 4);
        }
        return crc.getValue();
    }

    /** A compositor's frame as {@code <screenshots>/<name>.png} (the runner keeps it with the receipt). */
    static void snapshot(ScenarioRun r, Compositor c, String name) {
        if (c == null) return;
        try {
            int w = c.display().deviceW(), h = c.display().deviceH();
            byte[] f = c.frameRgba();
            java.awt.image.BufferedImage img = new java.awt.image.BufferedImage(w, h, java.awt.image.BufferedImage.TYPE_INT_RGB);
            for (int y = 0; y < h; y++)
                for (int x = 0; x < w; x++) {
                    int i = (y * w + x) * 4;
                    img.setRGB(x, y, (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF));
                }
            Path dir = com.example.evanscomputermod.testing.ScreenCapture.dir();
            Files.createDirectories(dir);
            javax.imageio.ImageIO.write(img, "png", dir.resolve(name + ".png").toFile());
            r.say("§7  wrote " + dir.resolve(name + ".png") + " (" + w + "x" + h + ")");
        } catch (java.io.IOException e) {
            r.say("§c  snapshot failed: " + e);
        }
    }

    // ------------------------------------------------------------ video_desktop

    static long speakerSamples(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(SPEAKER)) instanceof SpeakerBlockEntity s ? s.audio().samplesDrained() : -1;
    }

    /** Null while the Terminal shows the video (graphics, 320x180 rgb565, varied pixels). */
    static String terminalPicture(ScenarioRun r) {
        TerminalDisplay d = r.terminal("pc").getDisplay();
        byte[] px;
        synchronized (d) {
            if (d.getDisplayMode() != 1 || d.getGfxWidth() != 320 || d.getGfxHeight() != 180) {
                return "the Terminal shows mode " + d.getDisplayMode() + " " + d.getGfxWidth() + "x" + d.getGfxHeight();
            }
            if (d.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGB565) return "format " + d.getPixelFormat();
            px = d.getPixelData().clone();
        }
        java.util.Set<Integer> colours = new java.util.HashSet<>();
        for (int i = 0; i + 1 < px.length; i += 22) colours.add((px[i] & 0xFF) | (px[i + 1] & 0xFF) << 8);
        return colours.size() > 10 ? null : "only " + colours.size() + " colours";
    }

    static Scenario desktop() {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -2, 3, 0, 2);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.add(SPEAKER);
        var b = Scenario.builder("video_desktop",
                        "the video command plays a video on the Terminal with sound; Files opens it in the Video Player on the desktop")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    r.player().place(SpeakerContent.SPEAKER.get(), r.abs(SPEAKER), Direction.SOUTH,
                            s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
                    // Exception (docs/SCENARIO_EXCEPTIONS.md): a binary video can't be typed; players copy it into the folder.
                    PlayerKit.dropFiles(r, "pc", "home/videos", RESOURCES, List.of(VIDEO));
                }, r -> {
                    var u = GuiScenarios.ui(r);
                    if (u != null) u.stop();
                }))
                .timeLimit(60_000)
                .note("Setup (done for you): a Speaker placed west of the Terminal; " + VIDEO + " (6 s, 320x180, a 440 Hz tone)"
                        + " copied into the computer's folder <world>/computer-data/<id>/home/videos/ [exception: binary files enter"
                        + " a computer from outside the game; docs/SCENARIO_EXCEPTIONS.md].");
        b.note("Write a text file next to the video (the library's control)");
        PlayerKit.typeFiles(b, Map.of("pc", Map.of("/home/videos/notes.txt", "not a video")));
        b.note("On the console: what the video is, what videos there are");
        b.send("pc", "video info " + VIDEO_PATH)
                .expect("pc", "^sound:    22050 Hz, mono", "video info: 320x180, 15 fps, 0:06, a mono 22050 Hz sound track", "^video: ")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .send("pc", "video ls")
                .expect("pc", "test_pattern\\.mp4 .* 0:06  320x180  sound", "video ls finds it in /home/videos (not notes.txt)")
                .expect("pc", PlayerKit.PROMPT, "shell")
                .note("Control: a file that isn't there")
                .send("pc", "video info /home/videos/missing.mp4")
                .expect("pc", "missing\\.mp4: not found", "the missing file is named")
                .expect("pc", PlayerKit.PROMPT, "shell");
        b.note("Play it on the Terminal's screen; the Speaker plays its sound");
        b.mutate(r -> st(r).samples = speakerSamples(r), "note the Speaker's samples")
                .send("pc", "video play " + VIDEO_PATH)
                .await(VideoScenarios::terminalPicture, "the video on the Terminal's screen: 320x180 graphics, the test pattern", 15_000)
                .await(r -> speakerSamples(r) - st(r).samples > 22_050 ? null : "speaker samples +" + (speakerSamples(r) - st(r).samples),
                        "the Speaker plays the tone (over a second of sound)", 10_000)
                .expect("pc", "^done: \\d+ frames", "the video plays to the end (6 s)", "^video: ")
                .await(r -> r.terminal("pc").getDisplay().getDisplayMode() == 0 ? null : "still in graphics",
                        "the console is back", 5_000)
                .expect("pc", PlayerKit.PROMPT, "shell");
        b.note("The desktop: Files from the launcher, double-click home, videos, then the video");
        b.send("pc", "gui")
                .expect("pc", "^gui: started", "the GUI starts with the desktop", "(?i)unknown command|no GUI device")
                .await(r -> GuiScenarios.comp(r) != null && GuiScenarios.comp(r).shellConnected() ? null : "no desktop yet",
                        "the desktop is connected", 15_000);
        GuiScenarios.launch(b, "Files", "files")
                .await(r -> GuiScenarios.visible(r, "files", "home"), "Files lists the disk", 10_000)
                .mutate(r -> AppScenarios.doubleClick(r, "files", "home"), "double-click home")
                .await(r -> GuiScenarios.visible(r, "files", "videos"), "Files shows /home", 10_000)
                .mutate(r -> AppScenarios.doubleClick(r, "files", "videos"), "double-click videos")
                .await(r -> GuiScenarios.visible(r, "files", VIDEO), "Files shows the video", 10_000)
                .mutate(r -> st(r).samples = speakerSamples(r), "note the Speaker's samples")
                .mutate(r -> AppScenarios.doubleClick(r, "files", VIDEO), "double-click " + VIDEO)
                .await(r -> {
                    String s = GuiScenarios.shows(r, APP, "Pause");
                    return s != null ? s : GuiScenarios.shows(r, APP, VIDEO);
                }, "the Video Player opens it and plays (Open -> the app manager's default for .mp4)", 30_000)
                .await(r -> {
                    var w = GuiScenarios.appWindow(r, APP);
                    var apps = GuiScenarios.computer(r).getAppManager();
                    return w != null && apps.checkPath(w.pid(), VIDEO_PATH, false) ? null : "no grant for the video";
                }, "it runs as dev.ecm.video with the file", 2_000)
                .await(r -> timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)).startsWith("0:02") ? null
                        : "time " + timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)), "the time runs (0:02)", 10_000)
                .mutate(r -> st(r).crc = pictureCrc(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)), "look at the picture")
                .waitMs(400, "a moment")
                .await(r -> pictureCrc(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)) != st(r).crc ? null : "the same picture",
                        "the picture moves", 2_000)
                .await(r -> speakerSamples(r) - st(r).samples > 11_000 ? null : "speaker samples +" + (speakerSamples(r) - st(r).samples),
                        "the Speaker plays the video's sound", 10_000)
                .mutate(r -> GuiScenarios.snapshot(r, "video_desktop_playing"), "snapshot: the Video Player playing")
                .note("Pause")
                .mutate(r -> GuiScenarios.clickText(r, APP, "Pause"), "click Pause")
                .await(r -> GuiScenarios.shows(r, APP, "Play"), "paused: the button says Play", 5_000)
                .mutate(r -> st(r).time = timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)), "note the time")
                .waitMs(1_500, "1.5 s")
                .await(r -> {
                    String now = timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP));
                    if (!now.equals(st(r).time)) r.fail("paused, but the time moved from " + st(r).time + " to " + now);
                    return null;
                }, "control: paused, the time stays", 1_000)
                .note("Back to the library: it lists the video, not the text file beside it")
                .mutate(r -> GuiScenarios.clickText(r, APP, "< Library"), "click < Library")
                .await(r -> GuiScenarios.shows(r, APP, "Videos"), "the library", 5_000)
                .await(r -> GuiScenarios.shows(r, APP, VIDEO), "it lists " + VIDEO, 2_000)
                .await(r -> GuiScenarios.shows(r, APP, "notes.txt") == null ? "it lists notes.txt" : null,
                        "control: notes.txt isn't a video and isn't listed", 1_000)
                .mutate(r -> GuiScenarios.snapshot(r, "video_desktop_library"), "snapshot: the library");
        return b.build();
    }

    // ------------------------------------------------------------ video_phone

    static Compositor.WindowInfo onScreen(ScenarioRun r) {
        return PhoneGuiScenarios.onScreen(r, APP);
    }

    static String phoneTime(ScenarioRun r) {
        return timeText(PhoneGuiScenarios.comp(r), onScreen(r));
    }

    static long phoneSamples(ScenarioRun r) {
        var h = PhoneGuiScenarios.host(r);
        return h == null || h.speakerAudio() == null ? -1 : h.speakerAudio().samplesDrained();
    }

    /** The home grid's label for the Video Player (the grid may shorten it). */
    static String homeLabel(ScenarioRun r) {
        Compositor c = PhoneGuiScenarios.comp(r);
        return c == null ? null : c.layerTexts("home").stream().filter(t -> t.startsWith("Video")).findFirst().orElse(null);
    }

    static Scenario phone() {
        var b = PhoneGuiScenarios.start("video_phone",
                "the Video Player on an ePhone: its library, a video with sound, hotkey slot 1 pauses, home saves the place");
        b.mutate(r -> {
            try (InputStream in = VideoScenarios.class.getResourceAsStream(RESOURCES + VIDEO)) {
                if (in == null) throw new IllegalStateException(VIDEO + " is missing from the mod jar");
                Path dir = PhoneGuiScenarios.disk(r).resolve("home/videos");
                Files.createDirectories(dir);
                Files.write(dir.resolve(VIDEO), in.readAllBytes());
            } catch (Exception e) {
                r.fail(e.toString());
            }
        }, "put a video in the phone's documents (/home/videos/" + VIDEO + ") [exception: the binary file is copied into the"
                + " phone's folder; docs/SCENARIO_EXCEPTIONS.md]");
        b.await(r -> homeLabel(r) != null ? null : "home grid " + PhoneGuiScenarios.comp(r).layerTexts("home"),
                "the Video Player is on the home grid (a system app)", 20_000);
        b.mutate(r -> PhoneGuiScenarios.tap(r, PhoneGuiScenarios.comp(r).locateLayerText("home", homeLabel(r), false)), "tap Video Player");
        b.await(r -> onScreen(r) != null && PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id()).contains(VIDEO) ? null
                : "windows " + PhoneGuiScenarios.comp(r).windows(), "its library lists the video", 30_000);
        b.mutate(r -> st(r).samples = phoneSamples(r), "note the phone speaker's samples");
        b.mutate(r -> PhoneGuiScenarios.tap(r, PhoneGuiScenarios.comp(r).locateText(onScreen(r).id(), VIDEO, false)), "tap " + VIDEO);
        b.await(r -> PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id()).contains("Pause") && phoneTime(r).startsWith("0:01") ? null
                : "texts " + PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id()), "it plays: the time runs", 15_000);
        b.mutate(r -> st(r).crc = pictureCrc(PhoneGuiScenarios.comp(r), onScreen(r)), "look at the picture");
        b.waitMs(400, "a moment");
        b.await(r -> pictureCrc(PhoneGuiScenarios.comp(r), onScreen(r)) != st(r).crc ? null : "the same picture", "the picture moves", 2_000);
        b.await(r -> phoneSamples(r) - st(r).samples > 11_000 ? null : "samples +" + (phoneSamples(r) - st(r).samples),
                "the phone's speaker plays the video's sound", 10_000);
        b.mutate(r -> snapshot(r, PhoneGuiScenarios.comp(r), "video_phone_playing"), "snapshot: the video on the phone");
        b.note("In hand: hotkey slot 1 is Play/Pause (2 back 10 s, 3 forward 10 s)");
        b.mutate(r -> PhoneGuiScenarios.hotkey(r, 1), "press hotkey slot 1");
        b.await(r -> PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id()).contains("Play") ? null : "still playing",
                "paused by the hotkey", 5_000);
        b.mutate(r -> st(r).time = phoneTime(r), "note the time");
        b.waitMs(1_500, "1.5 s");
        b.await(r -> {
            if (!phoneTime(r).equals(st(r).time)) r.fail("paused, but the time moved from " + st(r).time + " to " + phoneTime(r));
            return null;
        }, "control: paused, the time stays", 1_000);
        b.mutate(r -> PhoneGuiScenarios.hotkey(r, 1), "press hotkey slot 1 again");
        b.await(r -> PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id()).contains("Pause") ? null : "still paused", "playing again", 5_000);
        b.mutate(r -> PhoneGuiScenarios.altKey(r, PhoneGuiScenarios.HID_H), "Alt+H: home");
        b.await(r -> {
            AppInstance a = PhoneGuiScenarios.app(r, APP);
            Path state = PhoneGuiScenarios.disk(r).resolve("var/apps/" + APP + "/state");
            try {
                return a != null && a.state() == AppInstance.State.SUSPENDED && Files.exists(state)
                        && Files.readString(state).contains(VIDEO_PATH) && Files.readString(state).contains("\"pos\":") ? null : "app " + a;
            } catch (Exception e) {
                return e.toString();
            }
        }, "home: it saved the file and position (WillSuspend -> SaveState) and is suspended", 10_000);
        b.mutate(r -> st(r).time = "", "open it again from the home grid");
        b.mutate(r -> PhoneGuiScenarios.tap(r, PhoneGuiScenarios.comp(r).locateLayerText("home", homeLabel(r), false)), "tap Video Player");
        b.await(r -> onScreen(r) != null && PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id()).contains("Play")
                        && phoneTime(r).startsWith("0:0") ? null : "texts " + (onScreen(r) == null ? "-" : PhoneGuiScenarios.comp(r).textsOf(onScreen(r).id())),
                "back on screen, paused where it was (it paused when it left the screen)", 10_000);
        b.timeLimit(60_000);
        return b.build();
    }

    static {
        for (Scenario s : List.of(desktop(), phone())) ALL.put(s.name, s);
    }
}
//?}
