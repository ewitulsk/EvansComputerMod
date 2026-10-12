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
import java.util.function.Supplier;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * The P3 desktop on the real kernel (Chicory, no world): a fresh computer
 * whose {@code /etc/ui.toml} says {@code autostart = true} boots to the
 * desktop shell; apps open from the taskbar's launcher with clicks a player
 * would make (at the drawn text, through the compositor's hit-testing); the
 * Terminal app runs a kernel shell session; text copied in the Text Editor
 * is pasted into the Terminal and runs; picking the Light theme in Settings
 * restyles every open app; Settings shows the Wi-Fi fix-it message without a
 * Wi-Fi Module (the join itself needs an Access Point: GameTest
 * {@code ecm_gui}, scenario {@code gui_desktop}).
 */
public class DesktopKernelTest {
    private static final String[] PROGRAMS = {"gui", "desktop", "terminal", "files", "settings", "editor", "echo"};
    private ComputerInstance computer;
    private TerminalDisplay display;
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
        for (String p : PROGRAMS) assumeTrue(Files.exists(progs.resolve(p + ".wasm")), p + ".wasm not built in " + progs);
        binDir = Files.createTempDirectory("ecm-desktop-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : PROGRAMS) Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        // A fresh computer whose only setting is: start the GUI at boot.
        Files.createDirectories(dataDir.resolve("etc"));
        Files.writeString(dataDir.resolve("etc/ui.toml"), "autostart = true\n");
        Host host = new Host(id, display);
        computer = new ComputerInstance(host, new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
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

    /** App windows (not the shell's frames), drawn and placed. */
    private List<Compositor.WindowInfo> apps() {
        Compositor c = comp();
        return c == null ? List.of() : c.windows().stream().filter(w -> !w.shell() && w.mapped() && w.placed() && w.visible()).toList();
    }

    private Compositor.WindowInfo app(String id) {
        return apps().stream().filter(w -> w.appId().equals(id)).findFirst().orElse(null);
    }

    private void click(int[] at, int button) {
        byte b = (byte) button;
        byte held = (byte) (button == 1 ? 2 : 1);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 1, at[0], at[1], (byte) 0, b, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 2, at[0], at[1], held, b, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 3, at[0], at[1], (byte) 0, b, (byte) 0);
    }

    private void key(int hid, int mods) {
        computer.handleKeyEvent(hid, mods, true, false);
        computer.handleKeyEvent(hid, mods, false, false);
    }

    private int[] waitAt(Supplier<int[]> where, long ms, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + ms;
        while (System.currentTimeMillis() < deadline) {
            int[] p = where.get();
            if (p != null) return p;
            Thread.sleep(20);
        }
        int[] p = where.get();
        assertNotNull(p, "timed out waiting to see " + what + "; windows: " + (comp() == null ? "-" : comp().windows()));
        return p;
    }

    /** Open an app the way a player does: the Apps button, then its launcher entry. */
    private Compositor.WindowInfo launch(String label, String appId) throws InterruptedException {
        int[] apps = waitAt(() -> comp().locateLayerText("taskbar", "Apps", false), 10_000, "the taskbar's Apps button");
        click(apps, 0);
        int[] entry = waitAt(() -> comp().locatePopupText(label, false), 10_000, "'" + label + "' in the launcher");
        click(entry, 0);
        waitFor(() -> app(appId) != null, 30_000, appId + "'s window");
        return app(appId);
    }

    private boolean shows(String appId, String text) {
        Compositor.WindowInfo w = app(appId);
        return w != null && comp().textsOf(w.id()).stream().anyMatch(t -> t.contains(text));
    }

    private int count(String appId, String text) {
        Compositor.WindowInfo w = app(appId);
        return w == null ? 0 : (int) comp().textsOf(w.id()).stream().filter(t -> t.contains(text)).count();
    }

    /** The composited colour (0xRRGGBB) at a device pixel. */
    private int pixel(int x, int y) {
        byte[] f = comp().frameRgba();
        int i = (y * 640 + x) * 4;
        return (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
    }

    @Test
    void autostartBootsToTheDesktopAndTheAppsWorkTogether() throws Exception {
        // Boot to the desktop with no command typed.
        waitFor(() -> comp() != null && comp().shellConnected(), 30_000, "the desktop shell to connect");
        waitFor(() -> comp().layerTexts("taskbar").contains("Apps"), 10_000, "the taskbar");
        assertEquals("ecm-default", comp().themeName(), "the desktop set the default theme");
        assertTrue(ui().status().contains("shell dev.ecm.desktop connected"), ui().status());

        // The Terminal app runs a kernel shell session.
        launch("Terminal", "terminal");
        waitFor(() -> shows("terminal", "logged in as"), 30_000, "the session's banner in the Terminal window");
        computer.handleTextEvent("echo hello-from-term");
        key(0x28, 0);
        waitFor(() -> count("terminal", "hello-from-term") >= 2, 20_000, "the echo command and its output in the Terminal window");

        // The Text Editor: type, select all, copy.
        launch("Text Editor", "editor");
        waitFor(() -> shows("editor", "New file"), 20_000, "the editor's status line");
        computer.handleTextEvent("echo copied-from-editor");
        waitFor(() -> shows("editor", "echo copied-from-editor"), 10_000, "the typed text in the editor");
        key(0x04, 0x2); // Ctrl+A
        key(0x06, 0x2); // Ctrl+C
        waitFor(() -> comp().clipboardText().equals("echo copied-from-editor"), 10_000, "the clipboard to hold the editor's text");

        // Back to the Terminal (its taskbar button) and paste with Ctrl+Shift+V, then Enter.
        int[] tb = waitAt(() -> comp().locateLayerText("taskbar", "Terminal", false), 5_000, "the Terminal's taskbar button");
        click(tb, 0);
        waitFor(() -> app("terminal") != null && app("terminal").focused(), 10_000, "the Terminal focused");
        key(0x19, 0x2 | 0x1); // Ctrl+Shift+V
        waitFor(() -> shows("terminal", "echo copied-from-editor"), 10_000, "the pasted command in the Terminal");
        key(0x28, 0);
        waitFor(() -> count("terminal", "copied-from-editor") >= 2, 20_000, "the pasted command's output");

        // Files and Settings.
        launch("Files", "files");
        waitFor(() -> shows("files", "etc"), 20_000, "the disk's folders in Files");
        launch("Settings", "settings");
        waitFor(() -> shows("settings", "To fix it"), 20_000, "Settings' Wi-Fi page with the fix-it message (no Wi-Fi Module here)");

        // Appearance -> Light: the desktop sends the theme and every open app restyles.
        int[] appearance = waitAt(() -> comp().locateText(app("settings").id(), "Appearance", false), 5_000, "Appearance");
        click(appearance, 0);
        int[] light = waitAt(() -> comp().locateText(app("settings").id(), "Light", false), 10_000, "the Light theme");
        click(light, 0);
        waitFor(() -> comp().themeName().equals("ecm-light"), 10_000, "the desktop to apply ecm-light from /etc/ui.toml");
        assertTrue(Files.readString(dataDir.resolve("etc/ui.toml")).contains("theme = \"ecm-light\""));
        int lightBg = 0xf3f4f6;
        for (String a : List.of("terminal", "editor", "files", "settings")) {
            waitFor(() -> {
                Compositor.WindowInfo w = app(a);
                return w != null && pixel(w.x() + w.w() - 3, w.y() + w.h() - 3) == lightBg;
            }, 10_000, a + "'s window background in the light theme");
        }
        Path out = repoRoot().resolve("build/ui-kernel");
        Files.createDirectories(out);
        javax.imageio.ImageIO.write(com.example.evanscomputermod.computer.ui.UiGoldenTest.toImage(comp().frameRgba(), 640, 400),
                "png", out.resolve("desktop_light.png").toFile());
        System.out.println("GUI: " + ui().status());
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

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    /** A Terminal-like host whose display takes pointer input. */
    private record Host(UUID id, TerminalDisplay display) implements IComputerHost {
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public IVisualProgramming getVisualProgramming() { return null; }
        @Override public boolean acceptsPointerInput() { return true; }
    }
}
