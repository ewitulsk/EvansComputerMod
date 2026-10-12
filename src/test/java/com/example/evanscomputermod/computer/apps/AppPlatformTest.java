package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.wasi.ProcessManager;
import com.example.evanscomputermod.testing.WasmTestModules;
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
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CopyOnWriteArrayList;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * The app platform on the real kernel and host (Chicory), no world
 * (docs/gui/APPS.md). Apps are installed by writing {@code /apps/<id>} on
 * the computer's disk; their entry is {@code apptest.wasm}
 * (rust/wasm-programs/apptest), which tries one thing and prints what
 * happened. Each case boots one computer (two for the restart) and stays
 * well under a minute.
 */
public class AppPlatformTest {

    private static final String PROBE = "dev.test.probe";

    private Path binDir;
    private final List<Path> dataDirs = new CopyOnWriteArrayList<>();
    private final List<ComputerInstance> computers = new CopyOnWriteArrayList<>();
    private byte[] apptest;

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
        assumeTrue(Files.exists(progs.resolve("apptest.wasm")), "apptest not built in " + progs);
        binDir = Files.createTempDirectory("ecm-wasm-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        Files.copy(progs.resolve("apptest.wasm"), binDir.resolve("apptest.wasm"));
        apptest = Files.readAllBytes(progs.resolve("apptest.wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());
    }

    @AfterEach
    void tearDown() throws Exception {
        for (ComputerInstance c : computers) c.close();
        if (binDir != null) deleteTree(binDir);
        for (Path d : dataDirs) deleteTree(d);
    }

    // ------------------------------------------------------------------ cases

    /**
     * An app that declares network asks on first use; the prompter answers
     * deny, so its socket open fails with -13 and the answer is remembered.
     * Controls: the same program typed in the terminal (unconfined) gets its
     * socket; after `pkg perm grant` the app gets one too.
     */
    @Test
    void appWithoutNetworkIsDeniedAfterPrompt() throws Exception {
        UUID id = UUID.randomUUID();
        install(id, PROBE, "permissions = [\"network\"]");
        ComputerInstance c = boot(id);
        AppManager apps = c.getAppManager();
        List<PermissionPrompter.Request> asked = new CopyOnWriteArrayList<>();
        apps.setPrompter(req -> {
            asked.add(req);
            return CompletableFuture.completedFuture(new PermissionPrompter.Answer(false, true));
        });

        AppInstance app = apps.launch(PROBE, List.of("sock"));
        assertEquals("sock denied -13", runToEnd(apps, app));
        assertEquals(1, asked.size(), "prompted once");
        assertEquals(PROBE, asked.get(0).appId());
        assertEquals("network", asked.get(0).permission());
        assertEquals(Boolean.FALSE, apps.permissions().decision(PROBE, "network"), "answer remembered");
        assertEquals(java.util.Set.of("network"), app.denials());

        // Remembered: no second prompt.
        assertEquals("sock denied -13", runToEnd(apps, apps.launch(PROBE, List.of("sock"))));
        assertEquals(1, asked.size());

        // Control: not an app -> unconfined.
        assertEquals("sock ok", runUnconfined(c, "sock"));
        // Control: granted (what `pkg perm grant` does through app_ctl).
        AppHostFunctions.Reply r = AppHostFunctions.control(apps, 0, List.of("perm", "grant", PROBE, "network"), 4096);
        assertEquals(0, r.status(), new String(r.data(), StandardCharsets.UTF_8));
        assertEquals("sock ok", runToEnd(apps, apps.launch(PROBE, List.of("sock"))));

        // An app may not use a permission it didn't declare, and isn't even asked.
        install(id, "dev.test.plain", "");
        apps.reindex();
        assertEquals("sock denied -13", runToEnd(apps, apps.launch("dev.test.plain", List.of("sock"))));
        assertEquals(1, asked.size(), "undeclared permissions are never prompted");
        // Console mode (no prompter): denied, not remembered.
        apps.setPrompter(null);
        install(id, "dev.test.net2", "permissions = [\"network\"]");
        apps.reindex();
        assertEquals("sock denied -13", runToEnd(apps, apps.launch("dev.test.net2", List.of("sock"))));
        assertNull(apps.permissions().decision("dev.test.net2", "network"));
    }

    /**
     * A program the app has the kernel start (a line typed into its shell
     * session) joins the app's group and is denied the same way. Control: the
     * same chain started from the terminal gets its socket.
     */
    @Test
    void childSpawnedByAppHasAppPermissions() throws Exception {
        UUID id = UUID.randomUUID();
        install(id, PROBE, "permissions = [\"network\"]");
        ComputerInstance c = boot(id);
        AppManager apps = c.getAppManager();
        List<PermissionPrompter.Request> asked = new CopyOnWriteArrayList<>();
        apps.setPrompter(req -> {
            asked.add(req);
            return CompletableFuture.completedFuture(new PermissionPrompter.Answer(false, false));
        });

        AppInstance app = apps.launch(PROBE, List.of("sh", "sock ", "apptest", "sock"));
        String out = runToEnd(apps, app);
        assertTrue(out.contains("sh| sock denied -13"), out);
        assertEquals(1, asked.size(), "the child asked as the app");
        assertEquals(PROBE, asked.get(0).appId());
        assertNotEquals(app.pid(), asked.get(0).pid(), "the request came from the child, not the app's own process");
        assertNull(apps.permissions().decision(PROBE, "network"), "a this-run answer isn't remembered");

        // Control: the same chain, unconfined.
        String ctl = runUnconfined(c, "sh", "sock ", "apptest", "sock");
        assertTrue(ctl.contains("sh| sock ok"), ctl);
    }

    /**
     * Open with: the handler gets a read-write grant for exactly that file; a
     * sibling file stays out of reach, and so does /home without a grant.
     * Its own /var/apps/<id> is always allowed; other apps' data never is.
     */
    @Test
    void openWithGrantAllowsOnlyThatFile() throws Exception {
        UUID id = UUID.randomUUID();
        Path disk = install(id, PROBE, "open_with = [\".txt\"]");
        Files.createDirectories(disk.resolve("home"));
        Files.writeString(disk.resolve("home/a.txt"), "alpha");
        Files.writeString(disk.resolve("home/b.txt"), "beta");
        Files.createDirectories(disk.resolve("var/apps/dev.other.app"));
        Files.writeString(disk.resolve("var/apps/dev.other.app/secret"), "x");
        ComputerInstance c = boot(id);
        AppManager apps = c.getAppManager();

        assertEquals(PROBE, apps.defaultHandlerFor("/home/a.txt"));
        AppInstance opened = apps.open("/home/a.txt", 0);
        assertEquals("opened /home/a.txt ok 5", runToEnd(apps, opened));
        assertEquals("write ok", runToEnd(apps, apps.launch(PROBE, List.of("write", "/home/a.txt", "edited"))));
        assertEquals("edited", Files.readString(disk.resolve("home/a.txt")));

        assertTrue(runToEnd(apps, apps.launch(PROBE, List.of("read", "/home/b.txt"))).startsWith("read denied"));
        assertTrue(runToEnd(apps, apps.launch(PROBE, List.of("write", "/home/b.txt", "x"))).startsWith("write denied"));
        assertEquals("beta", Files.readString(disk.resolve("home/b.txt")));
        assertTrue(runToEnd(apps, apps.launch(PROBE, List.of("read", "/var/apps/dev.other.app/secret"))).startsWith("read denied"));
        assertTrue(runToEnd(apps, apps.launch(PROBE, List.of("read", "/var/lib/apps/db.json"))).startsWith("read denied"));
        assertEquals("write ok", runToEnd(apps, apps.launch(PROBE, List.of("write", "/var/apps/" + PROBE + "/mine", "1"))));
        // Its own code is readable, not writable.
        assertTrue(runToEnd(apps, apps.launch(PROBE, List.of("read", "/apps/" + PROBE + "/app.toml"))).startsWith("read ok"));
        assertTrue(runToEnd(apps, apps.launch(PROBE, List.of("write", "/apps/" + PROBE + "/app.toml", "x"))).startsWith("write denied"));
        // Control: unconfined reads anything.
        assertEquals("read ok 4", runUnconfined(c, "read", "/home/b.txt"));
    }

    /**
     * PC policy: at shutdown a running app gets WillSuspend, its SaveState is
     * kept, and after a restart (a new ComputerInstance on the same disk) it
     * is offered as Restore. An app that misses the deadline saves nothing and
     * doesn't hold the shutdown past it.
     */
    @Test
    void stateSurvivesRestart() throws Exception {
        UUID id = UUID.randomUUID();
        install(id, PROBE, "");
        install(id, "dev.test.slow", "");
        ComputerInstance c = boot(id);
        AppManager apps = c.getAppManager();
        List<String> asked = new CopyOnWriteArrayList<>();
        // A stand-in compositor: the app "answers" WillSuspend with SaveState.
        apps.setAppEvents(new AppEvents() {
            @Override public boolean willSuspend(AppInstance app, int deadlineMs) {
                asked.add(app.appId() + "@" + deadlineMs);
                if (app.appId().equals(PROBE)) {
                    new Thread(() -> {
                        sleepQuietly(50);
                        apps.onSaveState(app.pid(), "notes=[\"milk\"]".getBytes(StandardCharsets.UTF_8));
                    }).start();
                }
                return true; // the slow one never answers
            }
        });
        AppInstance a = apps.launch(PROBE, List.of("wait", "60000"));
        AppInstance s = apps.launch("dev.test.slow", List.of("wait", "60000"));
        waitFor(() -> outputOf(a).contains("ready") && outputOf(s).contains("ready"), 10_000, "apps running");

        long t0 = System.nanoTime();
        assertEquals(1, c.shutdownApps());
        long ms = (System.nanoTime() - t0) / 1_000_000;
        assertTrue(ms >= 450 && ms < 2_000, "shutdown waited " + ms + " ms for a 500 ms deadline");
        assertEquals(2, asked.size());
        assertTrue(asked.contains(PROBE + "@500"));
        c.close();
        computers.remove(c);

        // "Server restart": a fresh instance on the same disk.
        ComputerInstance again = boot(id);
        AppManager apps2 = again.getAppManager();
        assertTrue(apps2.peekRestore(PROBE));
        assertEquals("notes=[\"milk\"]", new String(apps2.takeRestore(PROBE), StandardCharsets.UTF_8));
        assertFalse(apps2.peekRestore("dev.test.slow"), "missed the deadline: nothing saved");
        assertEquals(List.of(PROBE, "dev.test.slow"), apps2.lastSession());
        // The state is still there for a second cold start until the app saves again.
        assertNotNull(apps2.takeRestore(PROBE));
    }

    /**
     * iPhone policy on the pause API: a suspended app gets no CPU; a refresh
     * wake runs it and the budget cuts it off; BackgroundDone ends a window
     * early; without background:refresh there is no wake. An exception mode
     * keeps a suspended app running, duty-cycled, until it stops.
     */
    @Test
    void schedulerBudgetCutsRefreshWake() throws Exception {
        UUID id = UUID.randomUUID();
        Path disk = install(id, PROBE, "background = { refresh = true, modes = [\"audio\"] }");
        install(id, "dev.test.norefresh", "background = { refresh = true }");
        byte[] dots = WasmTestModules.spinCounter(20_000);
        Files.write(disk.resolve("apps/" + PROBE + "/main.wasm"), dots);
        Files.write(disk.resolve("apps/dev.test.norefresh/main.wasm"), dots);
        ComputerInstance c = boot(id);
        AppManager apps = c.getAppManager();
        apps.setPolicy(LifecyclePolicy.PHONE);
        BackgroundScheduler bg = apps.background();
        bg.configure(60_000, 300, 10);
        apps.permissions().setDecision(PROBE, "background:refresh", true);
        apps.permissions().setDecision(PROBE, "background:audio", true);

        AppInstance app = apps.launch(PROBE, List.of());
        Dots out = new Dots(app);
        waitFor(() -> out.count() > 50, 10_000, "app running");
        apps.suspend(app);
        assertEquals(AppInstance.State.SUSPENDED, app.state());
        ProcessManager pm = c.getProcessManager();
        assertTrue(pm.awaitHalted(app.pid(), 2_000));
        long d0 = out.settled();
        Thread.sleep(400);
        assertEquals(d0, out.count(), "a suspended app ran");

        // Refresh wake: runs, then the 300 ms budget pauses it again.
        BackgroundScheduler.Window w = bg.refreshNow(app);
        assertNotNull(w);
        waitFor(w::isOver, 3_000, "the budget ending the window");
        assertEquals("budget", w.endedBy());
        assertTrue(w.ranMs() >= 280 && w.ranMs() < 800, "window lasted " + w.ranMs() + " ms");
        assertTrue(pm.awaitHalted(app.pid(), 2_000));
        long d1 = out.settled();
        assertTrue(d1 > d0 + 10, "nothing ran during the wake: " + d0 + " -> " + d1);
        Thread.sleep(400);
        assertEquals(d1, out.count(), "ran after the budget");

        // BackgroundDone ends it early.
        BackgroundScheduler.Window w2 = bg.refreshNow(app);
        assertNotNull(w2);
        Thread.sleep(50);
        apps.onBackgroundDone(app.pid());
        assertTrue(w2.isOver());
        assertEquals("done", w2.endedBy());
        assertTrue(w2.ranMs() < 250, "BackgroundDone took " + w2.ranMs() + " ms");

        // Control: no background:refresh permission, no wake.
        AppInstance other = apps.launch("dev.test.norefresh", List.of());
        apps.suspend(other);
        assertNull(bg.refreshNow(other), "a wake without background:refresh");

        // Exception mode: suspended but audio playing -> keeps running, capped.
        assertTrue(bg.startMode(app, "audio"));
        assertEquals(Map.of(PROBE, java.util.Set.of("audio")), bg.activeModes());
        long m0 = out.settled();
        Thread.sleep(600);
        long m1 = out.count();
        assertTrue(m1 > m0, "audio mode didn't run the app");
        bg.stopMode(app, "audio");
        assertTrue(bg.activeModes().isEmpty());
        assertTrue(pm.awaitHalted(app.pid(), 2_000));
        long m2 = out.settled();
        Thread.sleep(300);
        assertEquals(m2, out.count(), "ran after the mode stopped");
        assertFalse(bg.startMode(other, "audio"), "a mode the app didn't declare");
        apps.kill(app);
        apps.kill(other);
    }

    // ------------------------------------------------------------------ helpers

    /** Write /apps/<appId> (app.toml + apptest as main.wasm) on the disk of computer {@code id}. */
    private Path install(UUID id, String appId, String extraToml) throws Exception {
        Path disk = Path.of("computer-data", id.toString());
        if (!dataDirs.contains(disk)) dataDirs.add(disk);
        Path dir = disk.resolve("apps").resolve(appId);
        Files.createDirectories(dir);
        Files.writeString(dir.resolve("app.toml"), "id = \"" + appId + "\"\nname = \"Probe\"\nversion = \"1.0.0\"\n"
                + "entry = \"main.wasm\"\n" + extraToml + "\n");
        Files.write(dir.resolve("main.wasm"), apptest);
        return disk;
    }

    private ComputerInstance boot(UUID id) throws Exception {
        TerminalDisplay display = new TerminalDisplay();
        ComputerInstance c = new ComputerInstance(new FakeHost(id, display), new byte[0][]);
        computers.add(c);
        c.loadModule("terminal_os");
        c.executeMain();
        c.startWorkerThread();
        waitFor(() -> screen(display).contains("Welcome to Terminal OS"), 10_000, "boot banner");
        return c;
    }

    /** The app's output once its process has exited (trimmed). */
    private static String runToEnd(AppManager apps, AppInstance app) throws Exception {
        StringBuilder sb = new StringBuilder();
        long deadline = System.currentTimeMillis() + 15_000;
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

    /** Run apptest as a plain program (not an app) and return its output. */
    private static String runUnconfined(ComputerInstance c, String... args) throws Exception {
        ProcessManager pm = c.getProcessManager();
        String[] argv = new String[args.length + 1];
        argv[0] = "apptest";
        System.arraycopy(args, 0, argv, 1, args.length);
        int pid = pm.spawn(WasmManager.getWasmBinPath().resolve("apptest.wasm"), argv, Map.of());
        assertTrue(pid > 0);
        StringBuilder sb = new StringBuilder();
        byte[] buf = new byte[4096];
        long deadline = System.currentTimeMillis() + 15_000;
        while (System.currentTimeMillis() < deadline) {
            int n;
            while ((n = pm.readOutput(pid, buf)) > 0) sb.append(new String(buf, 0, n, StandardCharsets.UTF_8));
            Integer code = pm.tryWait(pid);
            if (code != null && code != Integer.MIN_VALUE) {
                while ((n = pm.readOutput(pid, buf)) > 0) sb.append(new String(buf, 0, n, StandardCharsets.UTF_8));
                return sb.toString().trim();
            }
            Thread.sleep(10);
        }
        fail("apptest didn't exit: " + sb);
        return null;
    }

    private static String outputOf(AppInstance a) {
        return accumulated.computeIfAbsent(a, k -> new StringBuilder())
                .append(new String(a.takeOutput(65536), StandardCharsets.UTF_8)).toString();
    }

    private static final Map<AppInstance, StringBuilder> accumulated = new java.util.concurrent.ConcurrentHashMap<>();

    /** Counts the dots an app (spinCounter) has written. */
    private static final class Dots {
        private final AppInstance app;
        private long total;

        Dots(AppInstance app) {
            this.app = app;
        }

        synchronized long count() {
            total += app.takeOutput(1 << 20).length;
            return total;
        }

        /** The count once output already in flight has landed (the pump runs every 20 ms). */
        long settled() throws InterruptedException {
            long last = count();
            for (int i = 0; i < 20; i++) {
                Thread.sleep(100);
                long now = count();
                if (now == last) return now;
                last = now;
            }
            return last;
        }
    }

    private static String screen(TerminalDisplay d) {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < d.getHeight(); y++) {
            for (int x = 0; x < d.getWidth(); x++) {
                int ch = d.getCharAt(x, y) & 0xFF;
                sb.append(ch >= 32 && ch < 127 ? (char) ch : ' ');
            }
            sb.append('\n');
        }
        return sb.toString();
    }

    private static void sleepQuietly(long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException ignored) {
        }
    }

    private static void waitFor(java.util.function.BooleanSupplier cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.getAsBoolean()) return;
            Thread.sleep(10);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
    }

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    private record FakeHost(UUID id, TerminalDisplay display) implements IComputerHost {
        @Override public com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() { return null; }
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
    }
}
