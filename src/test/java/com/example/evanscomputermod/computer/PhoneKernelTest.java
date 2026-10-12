package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.apps.AppManager;
import com.example.evanscomputermod.computer.apps.BackgroundScheduler;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.wasi.ProcessManager;
import com.example.evanscomputermod.speaker.SpeakerAudio;
import com.example.evanscomputermod.speaker.SpeakerPeripheral;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;
import java.util.function.BooleanSupplier;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * The ePhone as a GUI phone (ROADMAP P5 part b) on the real kernel, on
 * Chicory with no world: a portrait host (what PhoneHost reports) boots
 * straight into the phone shell; apps open one at a time from the home grid;
 * hotkey slots reach the open app; the switch-app key; iPhone lifecycle
 * (suspend on home and sleep with SaveState, the same state after a wake,
 * Restore after a cold boot); a refresh wake that posts a notification while
 * the phone sleeps and is cut after its budget; the audio exception keeps a
 * suspended app playing (and without the permission it stops); the
 * permission dialog; the GBA app.
 *
 * <p>The phone's hardware glue (PhoneHost, 1.21.1) is played by the test:
 * sleep = {@code PhoneUi.willSleep()} then {@code ComputerInstance.sleep()}, as
 * PhoneHost does; the speaker is a {@link SpeakerAudio} drained by a thread.
 */
public class PhoneKernelTest {
    private static final String[] PROGRAMS = {"phone", "notes", "gba-app", "phonetest"};
    private ComputerInstance computer;
    private TerminalDisplay display;
    private com.example.evanscomputermod.computer.peripheral.PeripheralHub hub;
    private Path binDir;
    private Path dataDir;
    private UUID id;
    private Host host;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    @BeforeEach
    void setUp() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        for (String p : PROGRAMS) assumeTrue(Files.exists(progs.resolve(p + ".wasm")), p + ".wasm not built in " + progs);
        binDir = Files.createTempDirectory("ecm-phone-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : PROGRAMS) Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());
        id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        display = new TerminalDisplay();
        host = new Host(id, display, new java.util.concurrent.atomic.AtomicReference<>());
        boot();
    }

    /** Boot the phone's computer (again, for a cold boot) and wait for the phone shell. */
    private void boot() throws Exception {
        hub = new com.example.evanscomputermod.computer.peripheral.PeripheralHub(new com.example.evanscomputermod.computer.peripheral.PeripheralHub.Owner() {
            @Override public net.minecraft.world.level.Level level() { return null; }
            @Override public net.minecraft.core.BlockPos pos() { return net.minecraft.core.BlockPos.ZERO; }
            @Override public net.minecraft.core.Direction facing() { return net.minecraft.core.Direction.NORTH; }
            @Override public UUID computerId() { return id; }
            @Override public com.example.evanscomputermod.computer.peripheral.PeripheralEventBus events() {
                return computer == null ? null : computer.getPeripheralEvents();
            }
        });
        host.hub().set(hub);
        computer = new ComputerInstance(host, new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        waitFor(() -> ui().phone() != null && comp() != null && comp().shellConnected(), 30_000, "the phone shell to connect");
        waitFor(() -> comp().layerTexts("home").contains("Notes"), 20_000, "the home grid with Notes: " + comp().layerTexts("home"));
    }

    @AfterEach
    void tearDown() throws Exception {
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

    private PhoneUi phone() {
        return ui().phone();
    }

    private AppManager apps() {
        return computer.getAppManager();
    }

    // ------------------------------------------------------------ the player's hands

    private void tap(int[] at) {
        assertNotNull(at, "nothing to tap");
        ui().pointer(2, at[0], at[1], 1, 0, 0);
        ui().pointer(3, at[0], at[1], 0, 0, 0);
    }

    /** Tap the text {@code label} on the shell's layer {@code layer} (home, status, nav, overlay). */
    private void tapShell(String layer, String label) throws InterruptedException {
        int[][] at = new int[1][];
        waitFor(() -> (at[0] = comp().locateLayerText(layer, label, false)) != null, 15_000,
                "'" + label + "' on the " + layer + " layer: " + comp().layerTexts(layer));
        tap(at[0]);
    }

    private void chord(int hid) {
        computer.handleKeyEvent(hid, UiProto.Mods.ALT, true, false);
        computer.handleKeyEvent(hid, UiProto.Mods.ALT, false, false);
    }

    /** Sleep as PhoneHost does: the apps are suspended first, then the computer. */
    private void sleepPhone() throws InterruptedException {
        long grace = phone().willSleep();
        Thread.sleep(grace);
        computer.sleep();
    }

    private void wakePhone() {
        computer.wake();
        phone().wake();
    }

    /** The app window on screen (placed between the bars, focused). */
    private Compositor.WindowInfo onScreen(String appId) {
        for (Compositor.WindowInfo w : comp().windows()) {
            if (w.appId().equals(appId) && w.focused() && w.visible() && w.mapped() && w.y() == 16) return w;
        }
        return null;
    }

    private void open(String name, String appId) throws InterruptedException {
        if (phone().foreground() != null) chord(0x0B); // home first: the grid is under the open app
        tapShell("home", name);
        waitFor(() -> onScreen(appId) != null, 30_000, appId + " on screen: " + comp().windows());
        waitFor(() -> phone().foreground() != null && phone().foreground().appId().equals(appId), 5_000, "the foreground app " + appId);
    }

    private AppInstance app(String appId) {
        return apps().running().stream().filter(i -> i.appId().equals(appId)).findFirst().orElse(null);
    }

    /** Install a phonetest probe as app {@code appId} with extra manifest lines. */
    private void install(String appId, String name, String extra) throws Exception {
        Path dir = dataDir.resolve("apps").resolve(appId);
        Files.createDirectories(dir);
        Files.copy(binDir.resolve("phonetest.wasm"), dir.resolve("main.wasm"));
        Files.writeString(dir.resolve("app.toml"), "id = \"" + appId + "\"\nname = \"" + name + "\"\nversion = \"1.0.0\"\n"
                + "entry = \"main.wasm\"\n" + extra, StandardCharsets.UTF_8);
        apps().reindex();
        waitFor(() -> comp().layerTexts("home").contains(name), 15_000, name + " on the home grid");
    }

    private int countOf(String appId) {
        try {
            Path f = dataDir.resolve("var/apps").resolve(appId).resolve("count");
            return Files.exists(f) ? Integer.parseInt(Files.readString(f).trim()) : -1;
        } catch (Exception e) {
            return -1;
        }
    }

    // ------------------------------------------------------------ tests

    @Test
    void bootsIntoThePhoneShellAndAppsOpenOneAtATime() throws Exception {
        // Always GUI: the portrait display, the phone shell, the built-in apps, no console switch.
        assertEquals(UiProto.Device.EPHONE, comp().display().info().device());
        assertEquals(240, comp().display().logicalW());
        assertEquals(400, comp().display().logicalH());
        assertTrue(comp().layerTexts("home").contains("GBA"), "GBA on the home grid: " + comp().layerTexts("home"));
        waitFor(() -> display.getDisplayMode() == 1 && display.getGfxWidth() == 240 && display.getGfxHeight() == 400, 5_000,
                "the GUI on the phone's display");
        computer.handleKeyEvent(0x2A, 0x2 | 0x4, true, false);
        computer.handleKeyEvent(0x2A, 0x2 | 0x4, false, false);
        assertTrue(ui().guiVisible(), "Ctrl+Alt+Backspace doesn't show a console on a phone");

        open("Notes", "dev.ecm.notes");
        Compositor.WindowInfo notes = onScreen("dev.ecm.notes");
        assertEquals(0, notes.x());
        assertEquals(240, notes.w());
        assertEquals(400 - 16 - 20, notes.h(), "between the status bar and the nav bar");
        assertNotNull(app("dev.ecm.notes"), "launched through the app manager (an app instance)");

        // Type a note, then hotkey slot 1 ("Add note") reaches the open app.
        int[][] field = new int[1][];
        waitFor(() -> (field[0] = comp().locateText(notes.id(), "New note", false)) != null, 10_000, "the note field");
        tap(field[0]);
        computer.handleTextEvent("milk");
        waitFor(() -> comp().textsOf(notes.id()).contains("milk"), 10_000, "typed text: " + comp().textsOf(notes.id()));
        assertEquals("dev.ecm.notes", phone().hotkey(1, true), "slot 1 goes to the open app");
        phone().hotkey(1, false);
        waitFor(() -> comp().textsOf(notes.id()).stream().filter("milk"::equals).count() == 1
                && comp().textsOf(notes.id()).contains("New note"), 10_000, "the note added by the hotkey: " + comp().textsOf(notes.id()));
        assertEquals(null, phone().hotkey(2, true), "an unclaimed slot goes nowhere");
        assertTrue(ui().control("phone").contains("hotkey dev.ecm.notes 1 * Add note"), ui().control("phone"));

        // Home (Alt+H): the app is hidden and suspended after saving its state.
        chord(0x0B);
        waitFor(() -> app("dev.ecm.notes").state() == AppInstance.State.SUSPENDED, 10_000, "Notes suspended: " + phone().events());
        Path state = dataDir.resolve("var/apps/dev.ecm.notes/state");
        waitFor(() -> Files.exists(state), 2_000, "Notes' saved state");
        assertTrue(Files.readString(state).contains("milk"), Files.readString(state));
        assertEquals(null, phone().hotkey(1, true), "no app open: no hotkey");

        // The switch-app key brings it back without the phone being opened.
        phone().switchApp();
        waitFor(() -> onScreen("dev.ecm.notes") != null && app("dev.ecm.notes").state() == AppInstance.State.RUNNING, 10_000,
                "Notes back and running: " + phone().events());

        // A second app: Notes goes to the background (suspended), one app on screen at a time.
        install("dev.ecm.test.counter", "Counter", "hotkeys = [{ slot = 1, action = \"bump\", label = \"Bump\" }]\n");
        open("Counter", "dev.ecm.test.counter");
        waitFor(() -> app("dev.ecm.notes").state() == AppInstance.State.SUSPENDED, 10_000, "Notes suspended behind Counter");
        assertFalse(comp().windows().stream().filter(w -> w.appId().equals("dev.ecm.notes")).findFirst().orElseThrow().visible());
        phone().switchApp();
        waitFor(() -> onScreen("dev.ecm.notes") != null, 10_000, "the switch-app key cycles to Notes");
        saveFrame("phone_notes");

        // Panel hot-plug: a Tier 1 panel (120x200). The shell gets DisplayChanged and lays out again.
        tier = PanelTier.PHOSPHOR;
        try {
            ui().panelChanged();
            waitFor(() -> comp().display().logicalW() == 120 && comp().layerTexts("home").contains("Notes")
                    && onScreen("dev.ecm.notes") != null && onScreen("dev.ecm.notes").w() == 120
                    && onScreen("dev.ecm.notes").h() == 200 - 16 - 20, 10_000, "the shell at 120x200: " + comp().windows());
            saveFrame("phone_tier1", 120, 200);
        } finally {
            tier = PanelTier.COLOR_LCD;
        }
    }

    @Test
    void sleepAndWakeKeepTheAppsStateAndAColdBootRestoresIt() throws Exception {
        install("dev.ecm.test.counter", "Counter", "hotkeys = [{ slot = 1, action = \"bump\", label = \"Bump\" }]\n");
        open("Counter", "dev.ecm.test.counter");
        waitFor(() -> countOf("dev.ecm.test.counter") >= 4, 20_000, "the counter counting");
        sleepPhone();
        AppInstance counter = app("dev.ecm.test.counter");
        waitFor(() -> counter.state() == AppInstance.State.SUSPENDED, 5_000, "suspended by the sleep");
        Thread.sleep(300);
        int frozen = countOf("dev.ecm.test.counter");
        Thread.sleep(1500);
        assertEquals(frozen, countOf("dev.ecm.test.counter"), "frozen while asleep");
        wakePhone();
        waitFor(() -> countOf("dev.ecm.test.counter") > frozen, 10_000, "counting again");
        int after = countOf("dev.ecm.test.counter");
        assertTrue(after <= frozen + 8, "it went on from where it was (" + frozen + " -> " + after + "), not from 0 and not catching up");
        assertEquals(counter, app("dev.ecm.test.counter"), "the same process");

        // Cold boot (logout, battery): the shell reopens it and it gets its state back.
        computer.close();
        long saved = Long.parseLong(Files.readString(dataDir.resolve("var/apps/dev.ecm.test.counter/state")).replaceAll("\\D", ""));
        assertTrue(saved >= frozen, "saved " + saved + " >= " + frozen);
        boot();
        waitFor(() -> app("dev.ecm.test.counter") != null, 20_000, "the counter reopened from the last session");
        waitFor(() -> countOf("dev.ecm.test.counter") >= saved, 20_000, "restored: counting on from " + saved);
        assertTrue(countOf("dev.ecm.test.counter") < saved + 40, "from the restored count, not from 0");
        // Reopened in the background: the home screen is shown and it's in the switcher.
        assertEquals(null, phone().foreground());
        phone().switchApp();
        waitFor(() -> onScreen("dev.ecm.test.counter") != null, 10_000, "the reopened app comes up with the switch-app key");
    }

    @Test
    void refreshWakePostsANotificationWhileAsleepAndIsCutAfterItsBudget() throws Exception {
        install("dev.ecm.test.refresh", "Refresh", "permissions = [\"notifications\"]\nbackground = { refresh = true }\n");
        BackgroundScheduler bg = apps().background();
        bg.configure(1_500, 1_000, 10);
        open("Refresh", "dev.ecm.test.refresh");
        waitFor(() -> countOf("dev.ecm.test.refresh") >= 2, 20_000, "the app running");
        chord(0x0B);
        AppInstance app = app("dev.ecm.test.refresh");
        waitFor(() -> app.state() == AppInstance.State.SUSPENDED, 10_000, "suspended at home");
        sleepPhone();
        // Control: without background:refresh nothing runs.
        int frozen = countOf("dev.ecm.test.refresh");
        int notes0 = notificationsOf("dev.ecm.test.refresh");
        Thread.sleep(3_500);
        assertEquals(frozen, countOf("dev.ecm.test.refresh"), "no refresh without the permission");
        assertEquals(notes0, notificationsOf("dev.ecm.test.refresh"));
        // The player allows it (Settings): wakes every 1.5 s with a 1 s budget.
        apps().permissions().setDecision("dev.ecm.test.refresh", "background:refresh", true);
        BackgroundScheduler.Window[] w = new BackgroundScheduler.Window[1];
        waitFor(() -> (w[0] = bg.currentWindow()) != null, 5_000, "a refresh window");
        assertTrue(computer.isAsleep() || !w[0].isOver(), "the phone sleeps around the window");
        waitFor(() -> notificationsOf("dev.ecm.test.refresh") > notes0, 10_000, "a notification posted from the refresh wake");
        waitFor(w[0]::isOver, 5_000, "the window to end");
        assertEquals("budget", w[0].endedBy(), "cut by its budget (the app never says BackgroundDone)");
        assertTrue(w[0].ranMs() >= 950 && w[0].ranMs() < 1_600, "ran about its 1 s budget: " + w[0].ranMs());
        waitFor(computer::isAsleep, 1_000, "the computer asleep again");
        int cut = countOf("dev.ecm.test.refresh");
        Thread.sleep(250);
        assertEquals(cut, countOf("dev.ecm.test.refresh"), "nothing runs after the budget");
        assertTrue(cut > frozen, "it ran in the window (" + frozen + " -> " + cut + ")");
        System.out.println("refresh: " + phone().events() + " notifications " + comp().notifications());
    }

    private int notificationsOf(String appId) {
        return (int) comp().notifications().stream().filter(n -> n.appId().equals(appId)).count();
    }

    private Thread drain;

    @Test
    void audioKeepsPlayingInTheBackgroundUnderTheAudioException() throws Exception {
        SpeakerAudio audio = new SpeakerAudio();
        audio.setWriteListener(() -> phone().audioActivity(ProcessManager.currentPid()));
        hub.setWireless("speaker", new SpeakerPeripheral(new SpeakerPeripheral.Owner() {
            @Override public SpeakerAudio audio() { return audio; }
            @Override public void markVolumeChanged() {}
            @Override public boolean playSound(String sound, float volume, float pitch) { return false; }
        }));
        // The server tick: drain the speaker (no listeners; the counter is the signal).
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
        install("dev.ecm.test.tone", "Tone", "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"audio\"] }\n");
        apps().permissions().setDecision("dev.ecm.test.tone", "background:audio", false);

        // First use of the speaker: the permission dialog; the player allows it.
        tapShell("home", "Tone");
        waitFor(() -> String.join(" ", comp().layerTexts("overlay")).contains("use the speaker"), 30_000,
                "the permission dialog: " + comp().layerTexts("overlay"));
        saveFrame("phone_permission");
        tapShell("overlay", "Allow");
        waitFor(() -> Boolean.TRUE.equals(apps().permissions().decision("dev.ecm.test.tone", "peripherals:speaker")), 5_000,
                "the answer remembered");
        waitFor(() -> onScreen("dev.ecm.test.tone") != null, 20_000, "Tone on screen");
        waitFor(() -> audio.samplesDrained() > 8_000, 20_000, "the tone playing: " + audio.samplesDrained());

        // Control: background:audio denied -> home + asleep silences it.
        chord(0x0B);
        sleepPhone();
        Thread.sleep(1_200);
        long a0 = audio.samplesDrained();
        Thread.sleep(2_000);
        long quiet = audio.samplesDrained() - a0;
        assertTrue(quiet < 1_600, "without background:audio a suspended app is silent (" + quiet + " samples in 2 s)");

        // Allowed: back in hand it plays (the mode starts), then home + asleep keeps playing.
        apps().permissions().setDecision("dev.ecm.test.tone", "background:audio", true);
        wakePhone();
        phone().switchApp();
        waitFor(() -> onScreen("dev.ecm.test.tone") != null, 10_000, "Tone on screen again");
        waitFor(() -> apps().background().activeModes().containsKey("dev.ecm.test.tone"), 10_000, "the audio mode on: " + phone().events());
        chord(0x0B);
        AppInstance tone = app("dev.ecm.test.tone");
        waitFor(() -> tone.state() == AppInstance.State.SUSPENDED, 10_000, "suspended at home");
        assertTrue(ui().control("phone").contains("mode dev.ecm.test.tone audio"), "the status bar's indicator: " + ui().control("phone"));
        waitFor(() -> comp().layerTexts("status").contains("AUDIO"), 5_000, "AUDIO in the status bar: " + comp().layerTexts("status"));
        saveFrame("phone_audio_home");
        sleepPhone();
        Thread.sleep(500);
        long b0 = audio.samplesDrained();
        Thread.sleep(3_000);
        long played = audio.samplesDrained() - b0;
        System.out.println("audio: " + played + " samples in 3 s asleep (8000/s), events " + phone().events());
        assertTrue(played >= 12_000, "music keeps playing while suspended and asleep: " + played + " samples in 3 s");
        assertTrue(computer.isAsleep() && tone.state() == AppInstance.State.SUSPENDED, "still asleep and suspended (duty-cycled)");
    }

    @Test
    void gbaAppRunsAGameWithTouchKeysAndHotkeys() throws Exception {
        Path home = dataDir.resolve("home");
        Files.createDirectories(home);
        Files.copy(repoRoot().resolve("rust/wasm-programs/gba/tests/roms/arm.gba"), home.resolve("arm.gba"));
        apps().permissions().setDecision("dev.ecm.gba", "peripherals:speaker", false);
        apps().permissions().setDecision("dev.ecm.gba", "controller", true);
        tapShell("home", "GBA");
        // Reading the documents folder: the dialog; allowed for this run only.
        waitFor(() -> String.join(" ", comp().layerTexts("overlay")).contains("documents"), 30_000,
                "the documents permission dialog: " + comp().layerTexts("overlay"));
        tapShell("overlay", "Only this time");
        waitFor(() -> onScreen("dev.ecm.gba") != null, 30_000, "GBA on screen");
        Compositor.WindowInfo gba = onScreen("dev.ecm.gba");
        waitFor(() -> comp().textsOf(gba.id()).contains("arm.gba"), 20_000, "the ROM list: " + comp().textsOf(gba.id()));
        tap(comp().locateText(gba.id(), "arm.gba", false));
        waitFor(() -> comp().textsOf(gba.id()).contains("START"), 30_000, "the game with its touch buttons: " + comp().textsOf(gba.id()));
        waitFor(() -> varied(gba, 4, 160), 30_000, "the game drawing in its canvas");
        // In hand: the six slots are the GBA's; the Controls screen lists them.
        String st = ui().control("phone");
        for (String l : new String[]{"GBA A", "GBA B", "GBA Left", "GBA Right", "GBA Up", "GBA Start"}) {
            assertTrue(st.contains(l), "claimed " + l + ": " + st);
        }
        assertEquals("dev.ecm.gba", phone().hotkey(1, true), "slot 1 = A goes to the game");
        phone().hotkey(1, false);
        saveFrame("phone_gba");
    }

    // ------------------------------------------------------------ helpers

    /** True if the composited frame isn't one colour in rows y0..y1 of window w. */
    private boolean varied(Compositor.WindowInfo w, int y0, int y1) {
        byte[] f = comp().frameRgba();
        int first = -1;
        for (int y = w.y() + y0; y < w.y() + y1; y += 3) {
            for (int x = w.x() + 4; x < w.x() + w.w() - 4; x += 3) {
                int i = (y * 240 + x) * 4;
                int c = (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
                if (first < 0) first = c;
                else if (c != first) return true;
            }
        }
        return false;
    }

    /** For a look: build/ui-kernel/<name>.png. */
    private void saveFrame(String name) throws Exception {
        saveFrame(name, 240, 400);
    }

    private void saveFrame(String name, int w, int h) throws Exception {
        comp().repaintAll();
        Path out = repoRoot().resolve("build/ui-kernel");
        Files.createDirectories(out);
        javax.imageio.ImageIO.write(com.example.evanscomputermod.computer.ui.UiGoldenTest.toImage(comp().frameRgba(), w, h),
                "png", out.resolve(name + ".png").toFile());
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

    /** The panel in the phone (the slot screen swaps it; tests hot-plug it). */
    private static volatile PanelTier tier = PanelTier.COLOR_LCD;

    /** What PhoneHost reports: a portrait Color LCD panel that takes pointer input. */
    private record Host(UUID id, TerminalDisplay display,
                        java.util.concurrent.atomic.AtomicReference<com.example.evanscomputermod.computer.peripheral.PeripheralHub> hub)
            implements IComputerHost {
        @Override public com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() { return hub.get(); }
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public boolean acceptsPointerInput() { return true; }
        @Override public PanelTier displayTier(int displayId) { return displayId == DISPLAY_OWN ? tier : null; }
        @Override public boolean portraitDisplay() { return true; }
    }
}
