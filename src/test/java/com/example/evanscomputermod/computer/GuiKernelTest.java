package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IVisualProgramming;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * The GUI on the real kernel (ROADMAP P2 "Done when"), on Chicory with no
 * world: {@code gui} starts the compositor and the test shell, which tiles
 * windows; {@code gba} and {@code waterfall} run unchanged as legacy windows
 * side by side next to the Notes app; killing the shell enters safe mode and
 * the shell comes back within a second; Ctrl+Alt+Backspace switches between
 * the console and the GUI while both keep running.
 */
public class GuiKernelTest {
    private ComputerInstance computer;
    private TerminalDisplay display;
    private com.example.evanscomputermod.computer.peripheral.PeripheralHub hub;
    private Path binDir;
    private Path dataDir;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    @BeforeEach
    void boot() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        for (String p : new String[]{"gui", "test-shell", "notes", "gba", "waterfall", "echo"}) {
            assumeTrue(Files.exists(progs.resolve(p + ".wasm")), p + ".wasm not built in " + progs);
        }
        binDir = Files.createTempDirectory("ecm-gui-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : new String[]{"gui", "test-shell", "notes", "gba", "waterfall", "echo"}) {
            Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        }
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        Host host = new Host(id, display, new java.util.concurrent.atomic.AtomicReference<>());
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
        waitForScreen(s -> s.contains("Welcome to Terminal OS"), 10_000, "boot banner");
        Files.createDirectories(dataDir);
        Files.copy(root.resolve("rust/wasm-programs/gba/tests/roms/arm.gba"), dataDir.resolve("arm.gba"));
    }

    @AfterEach
    void shutdown() throws Exception {
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

    /** Start the GUI the way a player does: type {@code gui}. */
    private void startGui() throws Exception {
        // These cases are about the P2 test shell's tiling (the default shell is the desktop now).
        Files.createDirectories(dataDir.resolve("etc"));
        Files.writeString(dataDir.resolve("etc/ui.toml"), "shell = \"test-shell\"\n");
        computer.sendInput("gui\n");
        waitFor(() -> comp() != null && comp().shellConnected(), 20_000, "the test shell to connect");
        waitFor(() -> display.getDisplayMode() == 1 && display.getGfxWidth() == 640
                && display.getPixelFormat() == TerminalDisplay.PIXEL_FORMAT_RGBA8888, 5_000, "the GUI on the Terminal display");
    }

    private List<Compositor.WindowInfo> mapped() {
        return comp().windows().stream().filter(w -> w.mapped() && w.placed()).toList();
    }

    /** Console input while the GUI is shown: the kernel shell still reads the console (gui launch ...). */
    private void console(String line) {
        computer.sendInput(line + "\n");
    }

    /** Run a console command and wait for its answer (the next one would otherwise go to its stdin). */
    private void console(String line, String answer) throws InterruptedException {
        int prompts = count(screen(), "/ > ");
        console(line);
        waitForScreen(s -> s.contains(answer) && count(s, "/ > ") > prompts, 20_000, "'" + answer + "' after " + line);
    }

    private static int count(String s, String needle) {
        int n = 0;
        for (int i = s.indexOf(needle); i >= 0; i = s.indexOf(needle, i + 1)) n++;
        return n;
    }

    private com.example.evanscomputermod.radio.medium.BasicRadioMedium attachSdr() {
        var medium = new com.example.evanscomputermod.radio.medium.BasicRadioMedium(() -> System.nanoTime() / 1000, 7);
        var ep = new com.example.evanscomputermod.radio.api.RadioEndpoint() {
            final UUID id = UUID.randomUUID();
            com.example.evanscomputermod.radio.sdr.SdrRadio radio;
            @Override public UUID id() { return id; }
            @Override public com.example.evanscomputermod.radio.api.Pose pose() { return com.example.evanscomputermod.radio.api.Pose.at("o", 0, 64, 0); }
            @Override public com.example.evanscomputermod.radio.api.AntennaPattern antenna() { return com.example.evanscomputermod.radio.api.AntennaPattern.VERTICAL_DIPOLE; }
            @Override public com.example.evanscomputermod.radio.api.Channel tunedChannel() { return radio == null ? null : radio.channel(); }
            @Override public double maxTxPowerDbm() { return 37; }
            @Override public void onReceive(com.example.evanscomputermod.radio.api.Reception r) {}
        };
        var radio = new com.example.evanscomputermod.radio.sdr.SdrRadio(com.example.evanscomputermod.radio.sdr.SdrTier.STANDARD, ep,
                () -> System.nanoTime() / 1000, () -> 250_000, e -> {}, 5);
        ep.radio = radio;
        medium.register(ep);
        hub.setWireless("sdr_0", new com.example.evanscomputermod.radio.sdr.SdrPeripheral(radio, () -> medium, () -> {}));
        return medium;
    }

    /** True if the composited frame isn't one flat colour inside {@code w}'s rectangle. */
    private boolean varied(Compositor.WindowInfo w) {
        byte[] f = comp().frameRgba();
        int first = -1;
        for (int y = w.y() + 2; y < w.y() + w.h() - 2; y += 3) {
            for (int x = w.x() + 2; x < w.x() + w.w() - 2; x += 3) {
                int i = (y * 640 + x) * 4;
                int c = (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
                if (first < 0) first = c;
                else if (c != first) return true;
            }
        }
        return false;
    }

    @Test
    void guiTilesLegacyGbaAndWaterfallNextToNotes() throws Exception {
        attachSdr();
        console("notes", "no GUI is running");

        startGui();
        console("gui launch gba arm.gba", "gui: launched gba");
        console("gui launch waterfall 100M --size 200x120 --rate 48000", "gui: launched waterfall");
        waitFor(() -> mapped().size() == 2, 30_000, "gba and waterfall windows");
        // The shell's 120 ms move transition settles first.
        waitFor(() -> {
            List<Compositor.WindowInfo> m = mapped();
            return m.size() == 2 && (m.get(0).x() + m.get(0).w() <= m.get(1).x() || m.get(1).x() + m.get(1).w() <= m.get(0).x());
        }, 2_000, "the tiles to settle side by side: " + mapped());
        List<Compositor.WindowInfo> two = mapped();
        Compositor.WindowInfo a = two.get(0), b = two.get(1);
        assertTrue(a.legacy() && b.legacy(), "both are legacy pixel windows: " + two);
        assertTrue(a.x() + a.w() <= b.x() || b.x() + b.w() <= a.x(), "side by side, not overlapping: " + two);
        assertEquals(a.y(), b.y(), "same row");
        waitFor(() -> varied(mapped().get(0)) && varied(mapped().get(1)), 30_000, "both windows drawing");

        console("gui launch notes", "gui: launched notes");
        waitFor(() -> mapped().size() == 3, 30_000, "the Notes window");
        waitFor(() -> {
            List<Compositor.WindowInfo> m = mapped();
            return m.size() == 3 && m.get(0).x() + m.get(0).w() <= m.get(1).x() && m.get(1).x() + m.get(1).w() <= m.get(2).x();
        }, 2_000, "three tiles to settle: " + mapped());
        List<Compositor.WindowInfo> three = mapped();
        for (int i = 0; i < 3; i++) {
            for (int j = i + 1; j < 3; j++) {
                Compositor.WindowInfo p = three.get(i), q = three.get(j);
                assertTrue(p.x() + p.w() <= q.x() || q.x() + q.w() <= p.x(), "three tiles side by side: " + three);
            }
        }
        assertTrue(three.stream().anyMatch(w -> w.appId().equals("notes") && !w.legacy()), "Notes is a tree window: " + three);
        // Both legacy programs still run (the console never sees their output as a frame).
        assertTrue(three.stream().filter(Compositor.WindowInfo::legacy).count() == 2);
        // The Terminal display carries the composite: its top rows are the shell's bar.
        byte[] px = display.getPixelData();
        assertEquals(0x20, px[(4 * 640 + 630) * 4] & 0xFF, "bar background at the top right");
        // ...and the attached Screen mirrors it.
        waitFor(() -> barRows(SCREEN) != null && java.util.Arrays.equals(barRows(SCREEN), barRows(display)), 3_000,
                "the Screen cluster mirroring the GUI");
        assertEquals(1, SCREEN.getDisplayMode());
        System.out.println("GUI: " + ui().status());
        // For a look: build/ui-kernel/three_windows.png
        Path out = repoRoot().resolve("build/ui-kernel");
        Files.createDirectories(out);
        javax.imageio.ImageIO.write(com.example.evanscomputermod.computer.ui.UiGoldenTest.toImage(comp().frameRgba(), 640, 400),
                "png", out.resolve("three_windows.png").toFile());
    }

    @Test
    void killingTheShellEntersSafeModeAndTheShellComesBack() throws Exception {
        startGui();
        console("gui launch notes");
        waitFor(() -> mapped().size() == 1, 30_000, "the Notes window");
        int shellPid = comp().shellPid();
        long t0 = System.nanoTime();
        computer.getProcessManager().kill(shellPid);
        waitFor(() -> comp().safeMode(), 2_000, "safe mode");
        long safeAt = System.nanoTime();
        Compositor.WindowInfo w = comp().windows().get(0);
        waitFor(() -> comp().windows().get(0).y() == Compositor.SAFE_BAR_H, 2_000, "the focused window maximized under the safe-mode bar");
        waitFor(() -> comp().shellConnected() && !comp().safeMode(), 3_000, "the shell back");
        long back = System.nanoTime();
        double backMs = (back - t0) / 1e6;
        System.out.printf("shell killed -> safe mode in %.0f ms -> shell back in %.0f ms%n", (safeAt - t0) / 1e6, backMs);
        assertTrue(backMs <= 1000, "the shell must be back within 1 s, took " + backMs + " ms");
        assertTrue(comp().shellPid() != shellPid);
        waitFor(() -> mapped().size() == 1 && mapped().get(0).y() != Compositor.SAFE_BAR_H, 5_000, "the new shell re-tiles the window");
        assertEquals(w.id(), comp().windows().get(0).id(), "the app kept its window through it all");
    }

    @Test
    void chordSwitchesConsoleAndGuiWhileBothKeepRunning() throws Exception {
        startGui();
        console("gui launch gba arm.gba");
        waitFor(() -> mapped().size() == 1 && varied(mapped().get(0)), 30_000, "gba drawing in its window");
        // Ctrl+Alt+Backspace: the console comes back.
        computer.handleKeyEvent(0x2A, 0x2 | 0x4, true, false);
        computer.handleKeyEvent(0x2A, 0x2 | 0x4, false, false);
        waitFor(() -> !ui().guiVisible() && display.getDisplayMode() == 0, 5_000, "the console");
        // Typed keys reach the console again (the real input path, not sendInput).
        computer.handleKeyInput("echo console-alive\n");
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().equals("console-alive")), 10_000, "console output");
        // The GUI kept running: gba still presents frames into its window while hidden.
        long frames = comp().windows().get(0).id();
        assertTrue(ui().isRunning());
        long before = presented();
        Thread.sleep(500);
        assertTrue(presented() > before, "gba keeps drawing while the console is shown");
        // And back.
        computer.handleKeyEvent(0x2A, 0x2 | 0x4, true, false);
        computer.handleKeyEvent(0x2A, 0x2 | 0x4, false, false);
        waitFor(() -> ui().guiVisible() && display.getDisplayMode() == 1, 5_000, "the GUI again");
        waitFor(() -> varied(mapped().get(0)), 5_000, "the gba window shown again");
        assertFalse(frames < 0);
    }

    /** Frames gba presented into its legacy window so far. */
    private long presented() {
        int pid = comp().windows().get(0).pid();
        var dev = ui().existingDevice(pid);
        return dev == null ? -1 : dev.info().presented();
    }

    // ------------------------------------------------------------ helpers

    private static void waitFor(BooleanSupplier cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.getAsBoolean()) return;
            Thread.sleep(20);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
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

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    /** A Terminal-like host: its own display takes pointer input. */
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
        @Override public IVisualProgramming getVisualProgramming() { return null; }
        @Override public boolean acceptsPointerInput() { return true; }

        /** An attached Screen cluster: the GUI mirrors onto it ([screen] mode = "mirror", the default). */
        @Override public com.example.evanscomputermod.api.IAttachedDisplay getAttachedDisplay() {
            return new com.example.evanscomputermod.api.IAttachedDisplay() {
                @Override public TerminalDisplay display() { return SCREEN; }
                @Override public int gfxWidth() { return 640; }
                @Override public int gfxHeight() { return 400; }
                @Override public void setPowered(boolean on) {}
            };
        }
    }

    private static final TerminalDisplay SCREEN = new TerminalDisplay(1, 1);

    /** The bar rows (static content) of a display's graphics plane. */
    private static byte[] barRows(TerminalDisplay d) {
        synchronized (d) {
            byte[] px = d.getPixelData();
            if (px == null || d.getGfxWidth() != 640 || px.length < 640 * 16 * 4) return null;
            return java.util.Arrays.copyOf(px, 640 * 16 * 4);
        }
    }
}
