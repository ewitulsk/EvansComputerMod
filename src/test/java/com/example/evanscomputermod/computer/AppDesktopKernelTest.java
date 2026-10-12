package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.apps.AppManager;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiManager;
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
import java.util.function.Supplier;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * The app platform wired to the desktop (docs/gui/APPS.md "Integration"), on
 * the real kernel (Chicory, no world): a computer that boots to the desktop,
 * with test apps whose entry is {@code apptest.wasm}.
 *
 * <ul>
 *   <li>Prompter routing: an app's first {@code network} use shows the
 *   desktop's permission dialog naming the app; Allow (a click where the
 *   compositor draws it) lets the socket open, Deny makes it fail, no answer
 *   within the prompt timeout denies without remembering, and a late
 *   remembered answer still counts.</li>
 *   <li>Launch identity: {@code gui launch <id>} starts the app through the
 *   app manager and the compositor reports its app id; {@code /dev/uictl
 *   launch} from an app starts the program in the app's group (its
 *   identity, its permissions) and an app can't stop the GUI.</li>
 *   <li>Open with: opening a {@code .py} file starts the Text Editor app
 *   ({@code dev.ecm.editor}) on it, and it can read only that file; with two
 *   apps for the type and no default the chooser appears.</li>
 * </ul>
 */
public class AppDesktopKernelTest {
    private static final String[] PROGRAMS = {"gui", "desktop", "editor", "chooser", "apptest"};
    private static final String PROBE = "dev.test.probe";
    private ComputerInstance computer;
    private TerminalDisplay display;
    private Path binDir;
    private Path dataDir;
    private byte[] apptest;

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
        binDir = Files.createTempDirectory("ecm-appdesk-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : PROGRAMS) Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        apptest = Files.readAllBytes(progs.resolve("apptest.wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        Files.createDirectories(dataDir.resolve("etc"));
        Files.writeString(dataDir.resolve("etc/ui.toml"), "autostart = true\n");
        Files.createDirectories(dataDir.resolve("home"));
        Files.writeString(dataDir.resolve("home/a.py"), "print('only-this-file')\n");
        Files.writeString(dataDir.resolve("home/b.py"), "secret-b\n");
        install(PROBE, "Probe", "permissions = [\"network\", \"notifications\"]");
        install("dev.test.net2", "Net Two", "permissions = [\"network\"]");
        install("dev.test.net3", "Net Three", "permissions = [\"network\"]");
        computer = new ComputerInstance(new Host(id, display), new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        waitFor(() -> comp() != null && comp().shellConnected(), 30_000, "the desktop shell to connect");
        waitFor(() -> comp().layerTexts("taskbar").contains("Apps"), 10_000, "the taskbar");
    }

    @AfterEach
    void shutdown() throws Exception {
        if (computer != null) computer.close();
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
    }

    private void install(String appId, String name, String extra) throws Exception {
        Path dir = dataDir.resolve("apps").resolve(appId);
        Files.createDirectories(dir);
        Files.writeString(dir.resolve("app.toml"), "id = \"" + appId + "\"\nname = \"" + name + "\"\nversion = \"1.0.0\"\n"
                + "entry = \"main.wasm\"\n" + extra + "\n");
        Files.write(dir.resolve("main.wasm"), apptest);
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

    private void click(int[] at) {
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 1, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 2, at[0], at[1], (byte) 1, (byte) 0, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 3, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
    }

    private int[] waitAt(Supplier<int[]> where, long ms, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + ms;
        while (System.currentTimeMillis() < deadline) {
            int[] p = where.get();
            if (p != null) return p;
            Thread.sleep(20);
        }
        int[] p = where.get();
        assertNotNull(p, "timed out waiting to see " + what + "; layers: " + comp().layerTexts("permission"));
        return p;
    }

    /** The running app {@code appId} launched last. */
    private AppInstance running(String appId) {
        AppInstance last = null;
        for (AppInstance i : apps().running()) if (i.appId().equals(appId)) last = i;
        return last;
    }

    private static String runToEnd(AppInstance app) throws Exception {
        StringBuilder sb = new StringBuilder();
        long deadline = System.currentTimeMillis() + 20_000;
        while (System.currentTimeMillis() < deadline) {
            sb.append(new String(app.takeOutput(65536), StandardCharsets.UTF_8));
            if (app.exitCode() != null) {
                Thread.sleep(30);
                sb.append(new String(app.takeOutput(65536), StandardCharsets.UTF_8));
                return sb.toString().trim();
            }
            Thread.sleep(10);
        }
        fail(app.appId() + " didn't exit; output so far: " + sb);
        return null;
    }

    /** {@code gui launch <id> sock} (the terminal's command): the app through the app manager. */
    private AppInstance guiLaunch(String appId, String... args) throws Exception {
        String r = ui().control("launch " + appId + (args.length == 0 ? "" : " " + String.join(" ", args)));
        assertTrue(r.startsWith("gui: launched " + appId), r);
        int pid = Integer.parseInt(r.substring(r.indexOf("(pid ") + 5, r.indexOf(')')));
        AppInstance i = apps().byRootPid(pid);
        assertNotNull(i, "launched as an app: " + r);
        assertEquals(appId, i.appId());
        assertEquals(appId, ui().appId(pid), "the compositor reports the app id, not the program name");
        return i;
    }

    private void answer(String question, String button) throws Exception {
        waitFor(() -> comp().layerTexts("permission").contains(question), 20_000, "the dialog: " + question);
        click(waitAt(() -> comp().locateLayerText("permission", button, false), 5_000, button));
    }

    @Test
    void permissionPromptsUseTheDesktopDialog() throws Exception {
        // Allow (not remembered): the socket opens.
        AppInstance probe = guiLaunch(PROBE, "sock");
        answer("Allow Probe to use network?", "Allow");
        assertEquals("sock ok", runToEnd(probe));
        assertNull(apps().permissions().decision(PROBE, "network"), "Remember my choice was off");
        assertTrue(ui().status().contains("last permission answer: " + PROBE + " network: allowed"), ui().status());

        // Deny: sock_socket fails with -13 (control).
        AppInstance two = guiLaunch("dev.test.net2", "sock");
        answer("Allow Net Two to use network?", "Deny");
        assertEquals("sock denied -13", runToEnd(two));
        assertTrue(two.denials().contains("network"));

        // No answer within the prompt timeout: denied, not remembered.
        apps().setPromptTimeoutMs(1_500);
        long t0 = System.currentTimeMillis();
        AppInstance three = guiLaunch("dev.test.net3", "sock");
        assertEquals("sock denied -13", runToEnd(three));
        assertTrue(System.currentTimeMillis() - t0 < 15_000, "the timeout ended the wait");
        assertNull(apps().permissions().decision("dev.test.net3", "network"));
        // The dialog is still up; a late remembered Allow still counts for the next run.
        waitFor(() -> comp().layerTexts("permission").contains("Allow Net Three to use network?"), 5_000, "the stale dialog");
        click(waitAt(() -> comp().locateLayerText("permission", "Remember my choice", false), 5_000, "Remember my choice"));
        click(waitAt(() -> comp().locateLayerText("permission", "Allow", false), 5_000, "Allow"));
        waitFor(() -> Boolean.TRUE.equals(apps().permissions().decision("dev.test.net3", "network")), 5_000, "the late answer remembered");
        apps().setPromptTimeoutMs(-1);
        assertEquals("sock ok", runToEnd(guiLaunch("dev.test.net3", "sock")), "remembered: no prompt this time");

        // GUI stopped: console policy again (no prompter), denied for this run.
        assertEquals("gui: stopped\n", ui().control("stop"));
        assertNull(apps().prompter(), "no prompter without the GUI");
        AppInstance console = apps().launch("dev.test.net2", List.of("sock"));
        assertEquals("sock denied -13", runToEnd(console));
    }

    @Test
    void launchIdentityUictlAndOpenWith() throws Exception {
        // An app writes "launch apptest wait 3000" to /dev/uictl: the program joins the app's group.
        AppInstance probe = apps().launch(PROBE, List.of("write", "/dev/uictl", "launch apptest wait 3000"));
        assertEquals("write ok", runToEnd(probe));
        waitFor(() -> !computer.getProcessManager().groupMembers(probe.group()).isEmpty(), 5_000, "the launched program in the probe's group");
        for (int pid : computer.getProcessManager().groupMembers(probe.group())) {
            assertEquals(PROBE, apps().appIdOf(pid), "pid " + pid + " runs as the app that launched it");
            assertTrue(apps().isConfined(pid));
        }
        // Control: the same launch from the terminal (unconfined) starts an unconfined program.
        String r = ui().control("launch apptest wait 100");
        int free = Integer.parseInt(r.substring(r.indexOf("(pid ") + 5, r.indexOf(')')));
        assertNull(apps().appIdOf(free), r);
        // An app can't stop the GUI through /dev/uictl.
        assertEquals("write ok", runToEnd(apps().launch(PROBE, List.of("write", "/dev/uictl", "stop"))));
        assertTrue(ui().isRunning(), "the GUI still runs");

        // Open with: a .py opens in the Text Editor app (a system app), which gets only that file.
        assertNotNull(apps().manifest(AppManager.TEXT_EDITOR), "the Text Editor system app is installed");
        ui().open(0, "/home/a.py");
        waitFor(() -> comp().windows().stream().anyMatch(w -> w.appId().equals(AppManager.TEXT_EDITOR) && w.mapped()), 30_000, "the editor app's window");
        Compositor.WindowInfo ed = comp().windows().stream().filter(w -> w.appId().equals(AppManager.TEXT_EDITOR)).findFirst().orElseThrow();
        waitFor(() -> comp().textsOf(ed.id()).stream().anyMatch(t -> t.contains("only-this-file")), 20_000, "a.py's text in the editor");
        assertTrue(apps().checkPath(ed.pid(), "/home/a.py", false), "the opened file");
        assertTrue(apps().checkPath(ed.pid(), "/home/a.py", true), "read-write");
        assertFalse(apps().checkPath(ed.pid(), "/home/b.py", false), "any other file: no");
        assertFalse(comp().textsOf(ed.id()).stream().anyMatch(t -> t.contains("secret-b")));

        // Two apps open .py and none is the default: the chooser asks.
        install("dev.test.py2", "Py Two", "open_with = [\".py\"]");
        apps().reindex();
        ui().open(0, "/home/b.py");
        waitFor(() -> comp().windows().stream().anyMatch(w -> w.title().equals("Open with") && w.mapped()), 30_000, "the chooser");
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

    private record Host(UUID id, TerminalDisplay display) implements IComputerHost {
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public boolean acceptsPointerInput() { return true; }
    }
}
