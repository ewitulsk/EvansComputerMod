package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IVisualProgramming;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.testing.WasmTestModules;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Pause/resume of child processes and whole-computer sleep, on the real
 * kernel and host running on Chicory (the Wasmtime sidecar has its own
 * {@code WasmtimePauseTest}). Children are hand-assembled spin loops
 * ({@link WasmTestModules#spinCounter}) spawned straight into the computer's
 * {@link ProcessManager}, so the test reads their output itself. Each case
 * pauses for about one second; the class takes well under a minute.
 */
public class ProcessPauseTest {

    /** Pure-wasm iterations between two dots: big enough that no dot comes for ages. */
    private static final int NO_HOST_CALLS = 1 << 30;
    /** A dot every few thousand iterations: the dot count is the counter. */
    private static final int DOTS = 20_000;

    private ComputerInstance computer;
    private TerminalDisplay display;
    private ProcessManager pm;
    private Path binDir;
    private Path dataDir;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) {
            p = p.getParent();
        }
        return p;
    }

    @BeforeEach
    void boot() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        assumeTrue(Files.exists(progs.resolve("echo.wasm")) && Files.exists(progs.resolve("sleep.wasm")),
                "echo/sleep programs not built in " + progs);

        binDir = Files.createTempDirectory("ecm-wasm-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        Files.copy(progs.resolve("echo.wasm"), binDir.resolve("echo.wasm"));
        Files.copy(progs.resolve("sleep.wasm"), binDir.resolve("sleep.wasm"));
        Files.write(binDir.resolve("spin.wasm"), WasmTestModules.spinCounter(NO_HOST_CALLS));
        Files.write(binDir.resolve("dots.wasm"), WasmTestModules.spinCounter(DOTS));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        computer = new ComputerInstance(new FakeHost(id, display), new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        waitFor(() -> screen().contains("Welcome to Terminal OS"), 10_000, "boot banner");
        pm = computer.getProcessManager();
    }

    @AfterEach
    void shutdown() throws Exception {
        if (computer != null) computer.close();
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
    }

    /**
     * Chicory parks a child inside its pure-wasm loop (no host call for
     * ~10^9 iterations) and inside the dot loop; neither counter moves for a
     * second; after resume both carry on from where they stopped.
     */
    @Test
    void pausedChildStopsMidLoopAndResumesWhereItWas() throws Exception {
        int spin = spawn("spin.wasm");
        int dots = spawn("dots.wasm");
        DotCounter out = new DotCounter(dots);
        waitFor(() -> spinCounter(spin) > 1_000_000, 10_000, "spin loop running");
        waitFor(() -> out.poll() > 20, 10_000, "dots flowing");

        assertEquals(0, pm.pause(spin));
        assertEquals(0, pm.pause(dots));
        assertTrue(pm.isPaused(spin) && pm.isPaused(dots));
        assertTrue(pm.awaitHalted(spin, 2_000), "spin loop didn't stop");
        assertTrue(pm.awaitHalted(dots, 2_000), "dot loop didn't stop");
        assertTrue(spinCounter(spin) % NO_HOST_CALLS != 0, "stopped mid-loop, not at a host call");

        Thread.sleep(50); // let a dot already in the pipe arrive
        long c1 = spinCounter(spin);
        long d1 = out.poll();
        Thread.sleep(1_000);
        long c2 = spinCounter(spin);
        long d2 = out.poll();
        assertEquals(c1, c2, "guest loop ran while paused");
        assertEquals(d1, d2, "dots written while paused");
        assertTrue(pm.isAlive(spin) && pm.isAlive(dots));
        assertTrue(pm.pausedNanos(spin) >= 900_000_000L);

        assertEquals(0, pm.resume(spin));
        assertEquals(0, pm.resume(dots));
        assertFalse(pm.isPaused(spin));
        waitFor(() -> spinCounter(spin) > c1 + 1_000_000, 5_000, "spin loop continuing");
        waitFor(() -> out.poll() > d1 + 20, 5_000, "dots continuing");
        // The state was kept: the counter grew from c1, it didn't restart.
        assertTrue(spinCounter(spin) > c1);
        System.out.println("pause/resume on Chicory: spin stopped at " + c1 + ", dots at " + d1);
    }

    /** A sleep's deadline moves out by the paused time; the monotonic clock skips it. */
    @Test
    void pausedSleepIsStretchedByThePause() throws Exception {
        long t0 = System.nanoTime();
        int pid = pm.spawn(binDir.resolve("sleep.wasm"), new String[]{"sleep", "1500"}, java.util.Map.of());
        assertTrue(pid > 0);
        Thread.sleep(400);
        pm.pause(pid);
        assertTrue(pm.awaitHalted(pid, 1_000));
        Thread.sleep(1_000);
        assertTrue(pm.isAlive(pid), "the sleep finished while paused");
        pm.resume(pid);
        waitFor(() -> !pm.isAlive(pid), 5_000, "sleep exit");
        long ms = (System.nanoTime() - t0) / 1_000_000;
        // 1.5 s of sleep + 1 s paused (plus instantiate time).
        assertTrue(ms >= 2_400, "slept only " + ms + " ms: the pause counted towards the sleep");
        assertTrue(ms < 4_500, "slept " + ms + " ms");
        assertEquals(0, pm.tryWait(pid));
    }

    /** A group pauses together, takes late joiners paused, and a kill beats a pause. */
    @Test
    void groupPauseAndKillWhilePaused() throws Exception {
        int a = pm.spawn(binDir.resolve("dots.wasm"), new String[]{"dots"}, java.util.Map.of(), 77);
        int b = pm.spawn(binDir.resolve("dots.wasm"), new String[]{"dots"}, java.util.Map.of(), 77);
        int other = spawn("dots.wasm");
        DotCounter outA = new DotCounter(a), outB = new DotCounter(b), outO = new DotCounter(other);
        waitFor(() -> outA.poll() > 5 && outB.poll() > 5 && outO.poll() > 5, 10_000, "all running");

        assertEquals(2, pm.pauseGroup(77));
        assertTrue(pm.awaitHalted(a, 2_000) && pm.awaitHalted(b, 2_000));
        assertFalse(pm.isPaused(other), "pausing a group touched another process");
        int late = pm.spawn(binDir.resolve("dots.wasm"), new String[]{"dots"}, java.util.Map.of(), 77);
        DotCounter outL = new DotCounter(late);
        Thread.sleep(50);
        long a1 = outA.poll(), b1 = outB.poll(), o1 = outO.poll();
        Thread.sleep(700);
        assertEquals(a1, outA.poll());
        assertEquals(b1, outB.poll());
        assertEquals(0, outL.poll(), "a member spawned into a paused group ran");
        assertTrue(outO.poll() > o1, "the process outside the group must keep running");

        // Killing a paused process ends it without resuming it first.
        pm.kill(b);
        waitFor(() -> !pm.isAlive(b), 3_000, "paused process killed");
        assertEquals(a1, outA.poll());

        pm.resumeGroup(77);
        waitFor(() -> outA.poll() > a1 + 5 && outL.poll() > 5, 5_000, "group resumed");
        for (int pid : new int[]{a, late, other}) pm.kill(pid);
    }

    /**
     * Whole-computer sleep: the kernel worker and every child stop, the
     * kernel's clock skips the sleep, everything works after the wake, and a
     * budgeted wake goes back to sleep by itself.
     */
    @Test
    void computerSleepStopsKernelAndChildrenAndWakes() throws Exception {
        int dots = spawn("dots.wasm");
        int spin = spawn("spin.wasm");
        pm.pause(spin); // paused on its own: must stay paused across the wake
        DotCounter out = new DotCounter(dots);
        waitFor(() -> out.poll() > 20, 10_000, "dots flowing");

        computer.sleep();
        assertTrue(computer.isAsleep());
        assertTrue(computer.awaitAsleep(2_000), "computer didn't go to sleep");
        // Typed while asleep: queued, handled after the wake.
        computer.sendInput("echo after-wake\n");
        Thread.sleep(50);
        long d1 = out.poll();
        String screen1 = screen();
        Thread.sleep(1_000);
        assertEquals(d1, out.poll(), "a child ran while the computer slept");
        assertEquals(screen1, screen(), "the kernel ran while the computer slept");
        assertTrue(computer.sleptNanos() >= 1_000_000_000L);

        computer.wake();
        assertFalse(computer.isAsleep());
        waitFor(() -> screen().lines().anyMatch(l -> l.trim().equals("after-wake")), 10_000, "command typed while asleep");
        waitFor(() -> out.poll() > d1 + 20, 5_000, "dots after wake");
        assertTrue(pm.isPaused(spin), "a process paused on its own was resumed by the wake");
        pm.resume(spin);

        // Budgeted wake: asleep again after ~300 ms, having run in between.
        computer.sleep();
        assertTrue(computer.awaitAsleep(2_000));
        long d2 = out.poll();
        ComputerInstance.BudgetedWake w = computer.wakeFor(300);
        assertFalse(computer.isAsleep());
        waitFor(computer::isAsleep, 2_000, "re-sleep after the budget");
        assertTrue(computer.awaitAsleep(2_000));
        assertFalse(w.isCurrent());
        // "Halted" includes a child inside fd_write: that write may still land.
        Thread.sleep(50);
        long d3 = out.poll();
        assertTrue(d3 > d2, "nothing ran during the budgeted wake");
        Thread.sleep(300);
        assertEquals(d3, out.poll(), "ran after the budget ended");

        // finish() ends a window early; a superseded window does nothing.
        ComputerInstance.BudgetedWake w2 = computer.wakeFor(10_000);
        assertTrue(w2.finish());
        assertTrue(computer.isAsleep());
        ComputerInstance.BudgetedWake w3 = computer.wakeFor(10_000);
        computer.wake();
        assertFalse(w3.finish(), "a superseded window put the computer to sleep");
        assertFalse(computer.isAsleep());
        pm.kill(dots);
        pm.kill(spin);
    }

    // ------------------------------------------------------------ helpers

    private int spawn(String file) {
        int pid = pm.spawn(binDir.resolve(file), new String[]{file}, java.util.Map.of());
        assertTrue(pid > 0, "spawn " + file);
        return pid;
    }

    /** The spin loop's guest counter, read from the child's memory (Chicory memory is a plain array). */
    private long spinCounter(int pid) {
        WasmInstance inst = pm.instanceOf(pid);
        if (inst == null || inst.memory() == null) return 0;
        return Integer.toUnsignedLong(inst.memory().readInt(WasmTestModules.SPIN_COUNTER_ADDR));
    }

    /** Counts the dots a child writes to stdout. */
    private final class DotCounter {
        private final int pid;
        private long total;
        DotCounter(int pid) { this.pid = pid; }
        long poll() {
            byte[] buf = new byte[4096];
            int n;
            while ((n = pm.readOutput(pid, buf)) > 0) total += n;
            return total;
        }
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

    /** Minimal host: no world, no redstone, a plain TerminalDisplay. */
    private record FakeHost(UUID id, TerminalDisplay display) implements IComputerHost {
        @Override public com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() { return null; }
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public IVisualProgramming getVisualProgramming() { return null; }
    }
}
