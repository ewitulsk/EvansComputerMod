package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.testing.WasmTestModules;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Chicory's compiler against its interpreter on the real kernel: a pure-wasm
 * loop, Python's start-up (cold, then once python.wasm is compiled) and a Python
 * loop, and the gba emulator's frame rate. Each mode boots its own computer on a
 * fresh runtime ({@code ecm.chicory.compiler=false} is the interpreter).
 *
 * <p>The assertion is that the compiler is really in use (it compiles modules
 * and the wasm loop runs several times faster); the numbers are printed as
 * {@code CHICORY_BENCH} lines and recorded in docs/refactor/ARCHITECTURE.md.
 * Takes about 40 s with python and gba built.
 */
public class ChicoryCompilerBenchTest {

    /** Guest loop iterations between two dots of the spin program. */
    private static final int SPIN = 1 << 20;

    private ComputerInstance computer;
    private com.example.evanscomputermod.computer.peripheral.PeripheralHub hub;
    private TerminalDisplay display;
    private Path binDir;
    private Path dataDir;

    @AfterEach
    void shutdown() throws Exception {
        stop();
        System.clearProperty("ecm.chicory.compiler");
    }

    @Test
    void compilerIsUsedAndFasterThanTheInterpreter() throws Exception {
        Result interp = run(false);
        Result comp = run(true);
        System.out.println("CHICORY_BENCH interpreter " + interp);
        System.out.println("CHICORY_BENCH compiler    " + comp);
        assertTrue(comp.compiled > 0, "the compiler compiled nothing: " + comp);
        assertTrue(comp.spinPerSec > 3 * Math.max(1, interp.spinPerSec),
                "compiled wasm loop not faster: " + interp + " vs " + comp);
    }

    private record Result(int compiled, double spinPerSec, long pyColdMs, String pyWarmMs,
                          long pyLoopMs, String gba) {
        @Override public String toString() {
            return String.format("modulesCompiled=%d spinLoop=%.1f Miter/s pythonCold=%d ms pythonWarm(x3)=%s ms pythonLoop=%d ms gba=%s",
                    compiled, spinPerSec, pyColdMs, pyWarmMs, pyLoopMs, gba);
        }
    }

    private Result run(boolean compiler) throws Exception {
        int before = ChicoryRuntimeProvider.compiledModules();
        boot(compiler);

        // A pure-wasm loop: dots in 3 s, SPIN iterations each.
        computer.sendInput("spin\n");
        Thread.sleep(500);
        int d0 = count(screen(), ".");
        Thread.sleep(3000);
        int d1 = count(screen(), ".");
        computer.interrupt();
        computer.queueInterrupt(15, "{}");
        waitForScreen(s -> s.contains("^T - Program terminated"), 5_000, "spin stopped");
        double spin = (d1 - d0) * (double) SPIN / 3.0 / 1e6;

        long cold = -1, warm = -1, warm2 = -1, warm3 = -1, loop = -1;
        if (Files.exists(binDir.resolve("python.wasm"))) {
            Files.writeString(dataDir.resolve("hi.py"), "print('PY' + 'READY')\n");
            Files.writeString(dataDir.resolve("loop.py"), String.join("\n",
                    "import time",
                    "t = time.time()",
                    "s = 0",
                    "for i in range(20000):",
                    "    s += i * i",
                    "print('LOOP' + 'MS', int((time.time() - t) * 1000))",
                    ""));
            int c = ChicoryRuntimeProvider.compiledModules();
            cold = timePython("hi.py", "PYREADY");
            if (compiler) {
                // python.wasm (13 MB) compiles in the background; the next start uses it.
                waitFor(() -> ChicoryRuntimeProvider.compiledModules() > c, 120_000, "python compiled");
            }
            warm = timePython("hi.py", "PYREADY");
            if (compiler) { // the JIT keeps warming the compiled code up
                warm2 = timePython("hi.py", "PYREADY");
                warm3 = timePython("hi.py", "PYREADY");
            }
            timePython("loop.py", "LOOPMS");
            loop = screen().lines().filter(l -> l.trim().startsWith("LOOPMS ")).reduce((a, b) -> b)
                    .map(l -> Long.parseLong(l.trim().substring(7).trim())).orElse(-1L);
        }

        String gba = "not built";
        if (Files.exists(binDir.resolve("gba.wasm"))) {
            Files.copy(repoRoot().resolve("rust/wasm-programs/gba/tests/roms/arm.gba"), dataDir.resolve("arm.gba"));
            var pad = new com.example.evanscomputermod.controller.ControllerPeripheral(1);
            hub.setWireless("controller_1", pad);
            computer.sendInput("gba arm.gba\n");
            waitFor(() -> display.getDisplayMode() == 1 && display.getGfxWidth() == 240, 60_000, "the GBA screen");
            Thread.sleep(5000);
            // Guide quits; gba prints its average frame rate on the way out.
            pad.update(new com.example.evanscomputermod.controller.ControllerState(
                    com.example.evanscomputermod.controller.ControllerInput.Button.GUIDE.bit(), 0, 0, 0, 0, 0, 0));
            waitForScreen(s -> s.contains(" fps)."), 30_000, "gba exit");
            gba = screen().lines().filter(l -> l.contains(" fps).")).reduce((a, b) -> b).orElse("?").trim();
        }
        int compiled = ChicoryRuntimeProvider.compiledModules() - before;
        stop();
        return new Result(compiled, spin, cold, warm + "/" + warm2 + "/" + warm3, loop, gba);
    }

    private long timePython(String file, String marker) throws Exception {
        int seen = count(screen(), marker);
        int prompts = count(screen(), "/ > ");
        long t0 = System.nanoTime();
        computer.sendInput("python " + file + "\n");
        waitForScreen(s -> count(s, marker) > seen, 240_000, marker);
        long ms = (System.nanoTime() - t0) / 1_000_000;
        waitForScreen(s -> count(s, "/ > ") > prompts, 60_000, "prompt after python");
        return ms;
    }

    private void boot(boolean compiler) throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        binDir = Files.createTempDirectory("ecm-wasm-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : new String[]{"echo.wasm", "sleep.wasm", "python.wasm", "gba.wasm"}) {
            if (Files.exists(progs.resolve(p))) Files.copy(progs.resolve(p), binDir.resolve(p));
        }
        Files.write(binDir.resolve("spin.wasm"), WasmTestModules.spinCounter(SPIN));
        WasmManager.setWasmBinPathForTesting(binDir);
        System.setProperty("ecm.chicory.compiler", Boolean.toString(compiler));
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        var hubRef = new java.util.concurrent.atomic.AtomicReference<com.example.evanscomputermod.computer.peripheral.PeripheralHub>();
        hub = new com.example.evanscomputermod.computer.peripheral.PeripheralHub(new com.example.evanscomputermod.computer.peripheral.PeripheralHub.Owner() {
            @Override public net.minecraft.world.level.Level level() { return null; }
            @Override public net.minecraft.core.BlockPos pos() { return net.minecraft.core.BlockPos.ZERO; }
            @Override public net.minecraft.core.Direction facing() { return net.minecraft.core.Direction.NORTH; }
            @Override public UUID computerId() { return id; }
            @Override public com.example.evanscomputermod.computer.peripheral.PeripheralEventBus events() {
                return computer == null ? null : computer.getPeripheralEvents();
            }
        });
        hubRef.set(hub);
        computer = new ComputerInstance(new FakeHost(id, display, hubRef), new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        waitForScreen(s -> s.contains("Welcome to Terminal OS"), 60_000, "boot banner");
        Files.createDirectories(dataDir);
    }

    private void stop() throws Exception {
        if (computer != null) computer.close();
        computer = null;
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
        binDir = null;
        dataDir = null;
    }

    // ------------------------------------------------------------ helpers

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
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
            Thread.sleep(20);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
    }

    private void waitForScreen(Predicate<String> cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.test(screen())) return;
            Thread.sleep(20);
        }
        assertTrue(cond.test(screen()), "timed out waiting for " + what + ". Screen:\n" + screen());
    }

    private static int count(String s, String needle) {
        int n = 0;
        for (int i = s.indexOf(needle); i >= 0; i = s.indexOf(needle, i + 1)) n++;
        return n;
    }

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    private record FakeHost(UUID id, TerminalDisplay display,
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
    }
}
