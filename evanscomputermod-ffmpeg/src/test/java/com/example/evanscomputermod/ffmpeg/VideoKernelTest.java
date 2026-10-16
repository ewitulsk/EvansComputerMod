package com.example.evanscomputermod.ffmpeg;

import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.TerminalDisplay;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.apps.AppManager;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.computer.peripheral.PeripheralHub;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.speaker.SpeakerAudio;
import com.example.evanscomputermod.speaker.SpeakerPeripheral;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BooleanSupplier;
import java.util.function.Predicate;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Video on the real kernel (Chicory, no world), with a speaker drained the way
 * the server tick drains it. The video is {@code video/sample_av.mp4} (6 s,
 * 320x180, 15 fps, a 440 Hz tone) copied to {@code /home/videos/test.mp4}.
 *
 * <ul>
 *   <li>The {@code video} command on the console: {@code info}, {@code ls},
 *   {@code play} into the Terminal's graphics (sized to fit the box, with
 *   sound), and the display given back at the end; control: a missing file.</li>
 *   <li>The Video Player on the desktop: a system app; opening the file the way
 *   Files does starts it with a one-file grant; it plays (the picture moves,
 *   the time advances, the speaker gets sound), Pause stops the clock, the seek
 *   bar and -10s jump.</li>
 *   <li>The Video Player on an ePhone: on the home grid; tapped, its library
 *   lists the video, a tap plays it; hotkey slot 1 pauses; home saves the file
 *   and position.</li>
 * </ul>
 */
public class VideoKernelTest {
    private static final String[] PROGRAMS = {"video", "videoplayer", "gui", "desktop", "phone"};
    private ComputerInstance computer;
    private TerminalDisplay display;
    private Path binDir;
    private Path dataDir;
    private SpeakerAudio audio;
    private Thread drain;
    private UUID id;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    /** Boot a computer: a Terminal (console or desktop) or a portrait phone. */
    private void boot(boolean desktop, boolean phone) throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        for (String p : PROGRAMS) assumeTrue(Files.exists(progs.resolve(p + ".wasm")), p + ".wasm not built in " + progs);
        binDir = Files.createTempDirectory("ecm-video-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : PROGRAMS) Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        Files.createDirectories(dataDir.resolve("home/videos"));
        Files.copy(root.resolve("evanscomputermod-ffmpeg/src/test/resources/video/sample_av.mp4"), dataDir.resolve("home/videos/test.mp4"));
        if (desktop) {
            Files.createDirectories(dataDir.resolve("etc"));
            Files.writeString(dataDir.resolve("etc/ui.toml"), "autostart = true\n");
        }
        display = new TerminalDisplay();
        AtomicReference<PeripheralHub> hubRef = new AtomicReference<>();
        Host host = new Host(id, display, hubRef, phone);
        PeripheralHub hub = new PeripheralHub(new PeripheralHub.Owner() {
            @Override public net.minecraft.world.level.Level level() { return null; }
            @Override public net.minecraft.core.BlockPos pos() { return net.minecraft.core.BlockPos.ZERO; }
            @Override public net.minecraft.core.Direction facing() { return net.minecraft.core.Direction.NORTH; }
            @Override public UUID computerId() { return id; }
            @Override public com.example.evanscomputermod.computer.peripheral.PeripheralEventBus events() {
                return computer == null ? null : computer.getPeripheralEvents();
            }
        });
        hubRef.set(hub);
        audio = new SpeakerAudio();
        hub.setWireless("speaker", new SpeakerPeripheral(new SpeakerPeripheral.Owner() {
            @Override public SpeakerAudio audio() { return audio; }
            @Override public void markVolumeChanged() {}
            @Override public boolean playSound(String sound, float volume, float pitch) { return false; }
        }));
        drain = new Thread(() -> {
            while (!Thread.currentThread().isInterrupted()) {
                audio.drain();
                try {
                    Thread.sleep(50);
                } catch (InterruptedException e) {
                    return;
                }
            }
        });
        drain.setDaemon(true);
        drain.start();
        computer = new ComputerInstance(host, new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
    }

    @AfterEach
    void shutdown() throws Exception {
        if (drain != null) drain.interrupt();
        if (computer != null) computer.close();
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
    }

    private UiManager ui() {
        return computer.getUiManager();
    }

    private Compositor comp() {
        return ui().compositor();
    }

    private AppManager apps() {
        return computer.getAppManager();
    }

    // ------------------------------------------------------------ console

    @Test
    void videoCommandShowsInfoListsAndPlaysWithSound() throws Exception {
        boot(false, false);
        waitForScreen(s -> s.contains("Welcome to Terminal OS"), 10_000, "boot banner");

        computer.sendInput("video info /home/videos/test.mp4\n");
        waitForScreen(s -> s.contains("sound:    22050 Hz, mono"), 15_000, "video info");
        String s = screen();
        assertTrue(s.contains("size:     320x180"), s);
        assertTrue(s.contains("fps:      15.00"), s);
        assertTrue(s.contains("duration: 0:06"), s);

        computer.sendInput("video ls\n");
        waitForScreen(t -> t.contains("/home/videos/test.mp4") && t.contains("0:06  320x180  sound"), 15_000, "video ls");

        // Control: a file that isn't there.
        computer.sendInput("video info /home/nope.mp4\n");
        waitForScreen(t -> t.contains("/home/nope.mp4: not found"), 10_000, "the missing file named");

        long before = audio.samplesDrained();
        computer.sendInput("video play /home/videos/test.mp4 --size 160x120\n");
        // 16:9 fitted into 160x120: 160x90 in the Terminal's graphics, rgb565.
        waitFor(() -> display.getDisplayMode() == 1 && display.getGfxWidth() == 160 && display.getGfxHeight() == 90, 15_000,
                "the video on the Terminal's graphics: mode " + display.getDisplayMode() + " " + display.getGfxWidth() + "x" + display.getGfxHeight());
        assertEquals(TerminalDisplay.PIXEL_FORMAT_RGB565, display.getPixelFormat());
        // The display is cleared at init; the first frame follows.
        waitFor(() -> distinctRgb565(gfxPixels()) > 20, 5_000, "a colourful frame: " + distinctRgb565(gfxPixels()));
        byte[] a = gfxPixels();
        Thread.sleep(700);
        byte[] b = gfxPixels();
        assertFalse(java.util.Arrays.equals(a, b), "the picture moves");
        waitForScreen(t -> t.contains("done: "), 20_000, "the end of the video");
        Matcher m = Pattern.compile("done: (\\d+) frames \\((\\d+) late\\)").matcher(screen());
        assertTrue(m.find(), screen());
        int frames = Integer.parseInt(m.group(1));
        assertTrue(frames >= 85 && frames <= 92, "all 90 frames: " + frames);
        long played = audio.samplesDrained() - before;
        System.out.println("video play: " + m.group() + ", " + played + " speaker samples");
        assertTrue(played > 22050 * 4, "about 6 s of sound reached the speaker: " + played + " samples");
        assertTrue(screen().contains("playing /home/videos/test.mp4 -> terminal (320x180 -> 160x90, 15.00 fps, 0:06, sound)"), screen());
        waitFor(() -> display.getDisplayMode() == 0, 5_000, "the display given back to the console");
    }

    // ------------------------------------------------------------ desktop

    @Test
    void videoPlayerOpensFromFilesAndPlaysOnTheDesktop() throws Exception {
        boot(true, false);
        waitFor(() -> comp() != null && comp().shellConnected(), 30_000, "the desktop shell");
        waitFor(() -> apps().manifest("dev.ecm.video") != null, 10_000, "the Video Player system app: " + apps().brokenApps());
        assertEquals(Boolean.TRUE, apps().permissions().decision("dev.ecm.video", "files:documents"));
        assertEquals(Boolean.TRUE, apps().permissions().decision("dev.ecm.video", "peripherals:speaker"));
        assertEquals("dev.ecm.video", apps().defaultHandlerFor("/home/videos/test.mp4"), "what Files opens videos with");

        // Files' double-click: Open{path} -> the Video Player with that file.
        long before = audio.samplesDrained();
        ui().open(0, "/home/videos/test.mp4");
        Compositor.WindowInfo w = waitWindow("dev.ecm.video");
        waitFor(() -> comp().textsOf(w.id()).contains("Pause") && comp().textsOf(w.id()).contains("test.mp4"), 20_000,
                "playing: " + comp().textsOf(w.id()));
        assertTrue(apps().checkPath(w.pid(), "/home/videos/test.mp4", false), "the opened file is granted");
        waitFor(() -> timeText(w).startsWith("0:02"), 15_000, "the clock running: " + timeText(w));
        Compositor.WindowInfo placed = currentWindow("dev.ecm.video");
        String p1 = pictureCrc(placed);
        Thread.sleep(500);
        assertNotEquals(p1, pictureCrc(currentWindow("dev.ecm.video")), "the picture moves");
        assertTrue(comp().textsOf(w.id()).contains("sound"), "sound on: " + comp().textsOf(w.id()));
        waitFor(() -> audio.samplesDrained() - before > 22050, 10_000, "sound reaching the speaker: " + (audio.samplesDrained() - before));
        saveFrame("video_desktop_playing");

        // Pause: the clock stops.
        click(locate(w, "Pause"));
        waitFor(() -> comp().textsOf(w.id()).contains("Play"), 5_000, "paused: " + comp().textsOf(w.id()));
        String paused = timeText(w);
        Thread.sleep(1200);
        assertEquals(paused, timeText(w), "paused: the time stays");
        // The seek bar: a click near its end, then -10s back to the start.
        // The bar runs 8 lu in from each side, just above the time (lib.rs play_layout).
        int[] time = locate(w, timeText(w));
        Compositor.WindowInfo now = currentWindow("dev.ecm.video");
        click(new int[]{now.x() + 8 + (now.w() - 16) * 9 / 10, time[1] - 11});
        waitFor(() -> timeText(w).startsWith("0:05"), 5_000, "seeked to 90%: " + timeText(w));
        click(locate(w, "-10s"));
        waitFor(() -> timeText(w).startsWith("0:00"), 5_000, "back to the start: " + timeText(w));
        // Play to the end: it stops there and says so.
        click(locate(w, "Play"));
        waitFor(() -> comp().textsOf(w.id()).contains("Ended. Play starts it again."), 15_000, "the end: " + comp().textsOf(w.id()));
        assertTrue(timeText(w).startsWith("0:06"), timeText(w));
        // Library: the folder's videos.
        click(locate(w, "< Library"));
        waitFor(() -> comp().textsOf(w.id()).contains("Videos") && comp().textsOf(w.id()).contains("test.mp4"), 10_000,
                "the library: " + comp().textsOf(w.id()));
        saveFrame("video_desktop_library");
    }

    // ------------------------------------------------------------ phone

    @Test
    void videoPlayerOnAPhoneWithHotkeysAndSavedPlace() throws Exception {
        boot(false, true);
        waitFor(() -> ui().phone() != null && comp() != null && comp().shellConnected(), 30_000, "the phone shell");
        String[] label = new String[1];
        waitFor(() -> (label[0] = comp().layerTexts("home").stream().filter(t -> t.startsWith("Video")).findFirst().orElse(null)) != null,
                20_000, "the Video Player on the home grid: " + comp().layerTexts("home"));
        tap(comp().locateLayerText("home", label[0], false));
        Compositor.WindowInfo w = waitWindow("dev.ecm.video");
        waitFor(() -> comp().textsOf(w.id()).contains("test.mp4"), 20_000, "the library lists the video: " + comp().textsOf(w.id()));
        saveFrame("video_phone_library", 240, 400);
        tap(locate(w, "test.mp4"));
        waitFor(() -> comp().textsOf(w.id()).contains("Pause"), 20_000, "playing: " + comp().textsOf(w.id()));
        waitFor(() -> timeText(w).startsWith("0:01"), 10_000, "the clock running: " + timeText(w));
        saveFrame("video_phone_playing", 240, 400);
        // In hand: slot 1 = play/pause, claimed with its label.
        assertTrue(ui().control("phone").contains("Play/Pause"), ui().control("phone"));
        assertEquals("dev.ecm.video", ui().phone().hotkey(1, true));
        ui().phone().hotkey(1, false);
        waitFor(() -> comp().textsOf(w.id()).contains("Play"), 5_000, "paused by the hotkey: " + comp().textsOf(w.id()));
        String at = timeText(w);
        // Home: suspended after saving the file and the place.
        computer.handleKeyEvent(0x0B, UiProto.Mods.ALT, true, false);
        computer.handleKeyEvent(0x0B, UiProto.Mods.ALT, false, false);
        AppInstance app = apps().running().stream().filter(i -> i.appId().equals("dev.ecm.video")).findFirst().orElseThrow();
        waitFor(() -> app.state() == AppInstance.State.SUSPENDED, 10_000, "suspended at home");
        Path state = dataDir.resolve("var/apps/dev.ecm.video/state");
        waitFor(() -> Files.exists(state), 5_000, "the saved state");
        String saved = Files.readString(state, StandardCharsets.UTF_8);
        assertTrue(saved.contains("/home/videos/test.mp4") && saved.contains("\"pos\":"), saved);
        System.out.println("phone: paused at " + at + ", saved " + saved);
    }

    // ------------------------------------------------------------ helpers

    private Compositor.WindowInfo waitWindow(String appId) throws InterruptedException {
        waitFor(() -> currentWindow(appId) != null, 30_000, appId + "'s window: " + comp().windows());
        return currentWindow(appId);
    }

    private Compositor.WindowInfo currentWindow(String appId) {
        return comp().windows().stream().filter(x -> x.appId().equals(appId) && x.mapped() && x.visible()).findFirst().orElse(null);
    }

    private String timeText(Compositor.WindowInfo w) {
        return comp().textsOf(w.id()).stream().filter(t -> t.contains(" / ")).findFirst().orElse("");
    }

    private int[] locate(Compositor.WindowInfo w, String text) throws InterruptedException {
        int[][] at = new int[1][];
        waitFor(() -> (at[0] = comp().locateText(w.id(), text, false)) != null, 10_000, "'" + text + "' in " + comp().textsOf(w.id()));
        return at[0];
    }

    private void click(int[] at) {
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 1, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 2, at[0], at[1], (byte) 1, (byte) 0, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 3, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
    }

    private void tap(int[] at) {
        assertNotNull(at, "nothing to tap");
        ui().pointer(2, at[0], at[1], 1, 0, 0);
        ui().pointer(3, at[0], at[1], 0, 0, 0);
    }

    /** CRC of the composited frame inside a window (the picture is most of it). */
    private String pictureCrc(Compositor.WindowInfo w) {
        comp().repaintAll();
        byte[] f = comp().frameRgba();
        int fw = comp().display().logicalW();
        java.util.zip.CRC32 crc = new java.util.zip.CRC32();
        for (int y = w.y() + 24; y < w.y() + w.h() - 50; y++) {
            crc.update(f, (y * fw + w.x()) * 4, w.w() * 4);
        }
        return Long.toHexString(crc.getValue());
    }

    /** The Terminal's graphics pixels now (rgb565: two bytes each). */
    private byte[] gfxPixels() {
        byte[] px = new byte[display.getGfxWidth() * display.getGfxHeight() * 2];
        display.snapshotGfx(new int[2], px, null);
        return px;
    }

    private static int distinctRgb565(byte[] px) {
        java.util.Set<Integer> s = new java.util.HashSet<>();
        for (int i = 0; i + 1 < px.length; i += 2) s.add((px[i] & 0xFF) | (px[i + 1] & 0xFF) << 8);
        return s.size();
    }

    /** For a look: build/ui-kernel/<name>.png. */
    private void saveFrame(String name) throws Exception {
        saveFrame(name, comp().display().logicalW(), comp().display().logicalH());
    }

    private void saveFrame(String name, int w, int h) throws Exception {
        comp().repaintAll();
        Path out = repoRoot().resolve("evanscomputermod-ffmpeg/build-mc26.1/ui-kernel");
        Files.createDirectories(out);
        byte[] f = comp().frameRgba();
        java.awt.image.BufferedImage img = new java.awt.image.BufferedImage(w, h, java.awt.image.BufferedImage.TYPE_INT_RGB);
        for (int y = 0; y < h; y++)
            for (int x = 0; x < w; x++) {
                int i = (y * w + x) * 4;
                img.setRGB(x, y, (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF));
            }
        javax.imageio.ImageIO.write(img, "png", out.resolve(name + ".png").toFile());
    }

    private String screen() {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < display.getHeight(); y++) {
            for (int x = 0; x < display.getWidth(); x++) {
                int c = display.getCharAt(x, y) & 0xFF;
                sb.append(c >= 32 && c < 127 ? (char) c : ' ');
            }
            sb.append('\n');
        }
        return sb.toString();
    }

    private void waitForScreen(Predicate<String> cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.test(screen())) return;
            Thread.sleep(20);
        }
        assertTrue(cond.test(screen()), "timed out waiting for " + what + ". Screen:\n" + screen());
    }

    private static void waitFor(BooleanSupplier cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            try {
                if (cond.getAsBoolean()) return;
            } catch (RuntimeException ignored) {
                // not there yet
            }
            Thread.sleep(20);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
    }

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    private record Host(UUID id, TerminalDisplay display, AtomicReference<PeripheralHub> hub, boolean phone) implements IComputerHost {
        @Override public PeripheralHub getPeripheralHub() { return hub.get(); }
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public boolean acceptsPointerInput() { return true; }
        @Override public PanelTier displayTier(int displayId) { return displayId == DISPLAY_OWN ? PanelTier.COLOR_LCD : null; }
        @Override public boolean portraitDisplay() { return phone; }
    }
}
