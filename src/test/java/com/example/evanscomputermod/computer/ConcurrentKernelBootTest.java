package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.world.level.storage.LevelStorageSource;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.io.StringWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.function.Function;

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Several computers booting the real kernel at the same moment on a fresh
 * server (a Tech Village's computers, or an ePhone next to a Terminal). Every
 * one must load {@code terminal_os}, start, and reach the shell prompt.
 *
 * <p>Each round is cold: a fresh runtime (the kernel is parsed and compiled
 * again while all boots race for it) and a fresh server whose world directory
 * no computer has resolved yet. The server is a bare {@link MinecraftServer}
 * whose {@code getWorldPath} is vanilla's {@code LevelStorageAccess.getLevelPath}
 * (a memoising, non-thread-safe {@code HashMap}); the map's first resolve is
 * slowed down so boots that resolve it concurrently collide every time
 * instead of rarely. That collision is the bug this test was written for: a
 * Terminal booting beside an ePhone failed with "Error loading WASM module:
 * terminal_os - null" (a ConcurrentModificationException from
 * {@code ComputerStorage.path}).
 */
public class ConcurrentKernelBootTest {

    private static final int COMPUTERS = 8;
    private static final int ROUNDS = 4;

    private final List<ComputerInstance> computers = new ArrayList<>();
    private final List<Path> worldDirs = new ArrayList<>();
    private Path binDir;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    @AfterEach
    void cleanup() throws Exception {
        closeAll();
        for (Path d : worldDirs) deleteTree(d);
        if (binDir != null) deleteTree(binDir);
    }

    @Test
    void computersBootingTogetherFromAColdCacheAllReachThePrompt() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        binDir = Files.createTempDirectory("ecm-wasm-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);

        ExecutorService pool = Executors.newFixedThreadPool(COMPUTERS);
        try {
            for (int round = 0; round < ROUNDS; round++) {
                // Cold: a new runtime (and an emptied kernel cache) and a new server each round.
                WasmManager.bind(new ChicoryRuntimeProvider().create());
                Path world = Files.createTempDirectory("ecm-world");
                worldDirs.add(world);
                MinecraftServer server = fakeServer(world);
                CountDownLatch go = new CountDownLatch(1);
                List<TerminalDisplay> displays = new ArrayList<>();
                List<Future<ComputerInstance>> boots = new ArrayList<>();
                for (int i = 0; i < COMPUTERS; i++) {
                    TerminalDisplay display = new TerminalDisplay();
                    displays.add(display);
                    UUID id = UUID.randomUUID();
                    boots.add(pool.submit(() -> {
                        go.await();
                        ComputerInstance c = new ComputerInstance(new FakeHost(id, display, server), new byte[0][]);
                        c.loadModule("terminal_os");
                        c.executeMain();
                        c.startWorkerThread();
                        return c;
                    }));
                }
                go.countDown();
                List<String> failures = new ArrayList<>();
                for (int i = 0; i < COMPUTERS; i++) {
                    try {
                        computers.add(boots.get(i).get(60, TimeUnit.SECONDS));
                    } catch (java.util.concurrent.ExecutionException e) {
                        StringWriter sw = new StringWriter();
                        e.getCause().printStackTrace(new PrintWriter(sw));
                        failures.add("computer " + i + ": " + sw);
                    }
                }
                if (!failures.isEmpty()) {
                    fail("round " + round + ": " + failures.size() + "/" + COMPUTERS
                            + " computers failed to load the kernel:\n" + String.join("\n", failures));
                }
                for (int i = 0; i < COMPUTERS; i++) {
                    TerminalDisplay d = displays.get(i);
                    long deadline = System.currentTimeMillis() + 30_000;
                    while (System.currentTimeMillis() < deadline && !screen(d).contains("/ > ")) Thread.sleep(20);
                    assertTrue(screen(d).contains("/ > "),
                            "round " + round + ", computer " + i + " never reached the prompt. Screen:\n" + screen(d));
                }
                closeAll();
            }
        } finally {
            pool.shutdownNow();
        }
    }

    private void closeAll() {
        for (ComputerInstance c : computers) {
            try { c.close(); } catch (Exception ignored) {}
        }
        computers.clear();
    }

    private static String screen(TerminalDisplay display) {
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

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    /**
     * A server with only its world storage: vanilla's {@code LevelStorageAccess}
     * for {@code world}, its path memo slowed on a first resolve. Built without
     * constructors (a real server needs a whole game to start).
     */
    private static MinecraftServer fakeServer(Path world) throws Exception {
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        Field theUnsafe = unsafeClass.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        Object unsafe = theUnsafe.get(null);
        Method allocate = unsafeClass.getMethod("allocateInstance", Class.class);

        Object access = allocate.invoke(unsafe, LevelStorageSource.LevelStorageAccess.class);
        set(access, LevelStorageSource.LevelStorageAccess.class, "levelDirectory", new LevelStorageSource.LevelDirectory(world));
        set(access, LevelStorageSource.LevelStorageAccess.class, "resources", new SlowFirstResolveMap<>());
        MinecraftServer server = (MinecraftServer) allocate.invoke(unsafe, DedicatedServer.class);
        set(server, MinecraftServer.class, "storageSource", access);
        return server;
    }

    private static void set(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    /** A HashMap whose computeIfAbsent takes 50 ms to compute a value: concurrent first resolves overlap. */
    private static final class SlowFirstResolveMap<K, V> extends HashMap<K, V> {
        @Override
        public V computeIfAbsent(K key, Function<? super K, ? extends V> mapping) {
            return super.computeIfAbsent(key, k -> {
                try {
                    Thread.sleep(50);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                }
                return mapping.apply(k);
            });
        }
    }

    /** Minimal host: a storage-only server, no peripherals, a plain TerminalDisplay. */
    private record FakeHost(UUID id, TerminalDisplay display, MinecraftServer server) implements IComputerHost {
        @Override public com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() { return null; }
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return server; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
    }
}
