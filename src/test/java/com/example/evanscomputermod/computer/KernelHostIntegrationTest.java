package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IVisualProgramming;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Layer-3 contract test: the real kernel (terminal_os.wasm) running inside
 * the real Java host ({@link ComputerInstance}) on the pure-Java Chicory
 * runtime. Catches host/kernel ABI drift (strict import linking runs in
 * loadModule), the event-driven worker loop, non-blocking job control and
 * Ctrl+T handling -- without starting Minecraft.
 *
 * <p>Needs the kernel and a few WASI programs built:
 * {@code cargo build --release --target wasm32-unknown-unknown -p terminal-os} and
 * {@code cargo build --release --target wasm32-wasip1 -p echo -p sleep}
 * (run in {@code rust/}).
 */
public class KernelHostIntegrationTest {

    private ComputerInstance computer;
    private TerminalDisplay display;
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
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        computer = new ComputerInstance(new FakeHost(id, display), new byte[0][]);
        computer.loadModule("terminal_os"); // strict import check runs here
        computer.executeMain();
        computer.startWorkerThread();
        waitForScreen(s -> s.contains("Welcome to Terminal OS"), 10_000, "boot banner");
    }

    @AfterEach
    void shutdown() throws Exception {
        if (computer != null) computer.close();
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
    }

    @Test
    void childProgramOutputReachesScreenAndPromptReturns() throws Exception {
        int prompts = count(screen(), "/ > ");
        computer.sendInput("echo hello-from-child\n");
        // A line consisting only of the program's output (not the echoed command).
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().equals("hello-from-child")), 10_000, "child output");
        waitForScreen(s -> count(s, "/ > ") > prompts, 10_000, "prompt after child exit");
    }

    @Test
    void ctrlTStopsForegroundProgram() throws Exception {
        computer.sendInput("sleep 30\n");
        Thread.sleep(500);
        // Same sequence TerminalBlockEntity uses for Ctrl+T.
        computer.interrupt();
        computer.queueInterrupt(15, "{}");
        waitForScreen(s -> s.contains("^T - Program terminated"), 3_000, "Ctrl+T acknowledgement");
    }

    @Test
    void switchWithoutInterfacesExplainsWhy() throws Exception {
        computer.sendInput("switch on\n");
        waitForScreen(s -> s.contains("no network interfaces"), 5_000, "switch message");
    }

    // ------------------------------------------------------------ helpers

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

    /** Minimal host: no world, no redstone, a plain TerminalDisplay. */
    private record FakeHost(UUID id, TerminalDisplay display) implements IComputerHost {
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
