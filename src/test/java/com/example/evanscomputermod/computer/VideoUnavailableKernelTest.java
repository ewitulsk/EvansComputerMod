package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.api.video.VideoBackends;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * The main mod without the evanscomputermod-ffmpeg sidecar (its own test
 * classpath has no video backend), on the real kernel: {@code video} and
 * {@code player} say video isn't installed instead of failing obscurely, and
 * nothing takes the display. Control: {@code video --help} still answers.
 * The sidecar's own tests cover playback.
 */
public class VideoUnavailableKernelTest {
    private ComputerInstance computer;
    private TerminalDisplay display;
    private Path binDir;
    private Path dataDir;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    @AfterEach
    void shutdown() throws Exception {
        if (computer != null) computer.close();
        for (Path p : new Path[]{binDir, dataDir}) {
            if (p != null && Files.exists(p)) {
                try (var walk = Files.walk(p)) {
                    walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
                }
            }
        }
    }

    @Test
    void videoCommandsSayVideoIsNotInstalled() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built");
        for (String p : new String[]{"video", "player"}) assumeTrue(Files.exists(progs.resolve(p + ".wasm")), p + " not built");
        VideoBackends.reset();
        assertFalse(VideoBackends.available(), "no backend on the main mod's classpath");

        binDir = Files.createTempDirectory("ecm-novideo-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : new String[]{"video", "player"}) Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        Files.createDirectories(dataDir.resolve("home"));
        Files.write(dataDir.resolve("home/clip.mp4"), new byte[64]);
        display = new TerminalDisplay();
        computer = new ComputerInstance(new Host(id, display), new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        waitForScreen(s -> s.contains("Welcome to Terminal OS"), 10_000, "boot banner");

        computer.sendInput("video info /home/clip.mp4\n");
        waitForScreen(s -> s.contains("video: video isn't installed on this server"), 10_000, "video says it isn't installed");
        computer.sendInput("player /home/clip.mp4\n");
        waitForScreen(s -> s.contains("player: video isn't installed on this server"), 10_000, "player says it isn't installed");
        assertEquals(0, display.getDisplayMode(), "nothing took the display");
        computer.sendInput("video --help\n");
        waitForScreen(s -> s.contains("usage: video info FILE"), 10_000, "control: video's usage still answers");
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

    private record Host(UUID id, TerminalDisplay display) implements IComputerHost {
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
    }
}
