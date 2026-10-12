package com.example.evanscomputermod.wasmtime;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

import com.example.evanscomputermod.api.wasm.WasmInstance;

import org.junit.jupiter.api.Test;

import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;

/**
 * Several computers loading the real kernel at the same moment on a cold
 * Wasmtime runtime: the module cache compiles it once, the first instance
 * leases the cached engine and the others get private engines. Every
 * instance must link and answer the kernel's layout query. (ComputerInstance
 * itself can't load here: the sidecar's tests have no FML; the main mod's
 * ConcurrentKernelBootTest boots whole computers on Chicory.)
 */
public class WasmtimeConcurrentKernelTest {

    private static final int COMPUTERS = 8;
    private static final int ROUNDS = 3;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    @Test
    void kernelsLoadedTogetherFromAColdCacheAllLink() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        byte[] bytes = Files.readAllBytes(kernel);

        ExecutorService pool = Executors.newFixedThreadPool(COMPUTERS);
        try {
            for (int round = 0; round < ROUNDS; round++) {
                try (WasmtimeRuntime runtime = new WasmtimeRuntime()) {
                    CountDownLatch go = new CountDownLatch(1);
                    List<Future<WasmInstance>> loads = new ArrayList<>();
                    for (int i = 0; i < COMPUTERS; i++) {
                        loads.add(pool.submit(() -> {
                            go.await();
                            WasmInstance inst = runtime.instantiate(runtime.compile(bytes), List.of());
                            long[] scratch = inst.callExport("abi_scratch");
                            long[] words = inst.callExport("abi_layout", scratch[0], 15);
                            assertTrue(words[0] >= 15, "abi_layout answered " + words[0] + " words");
                            assertEquals(1, inst.memory().readInt((int) scratch[0]), "kernel ABI layout version");
                            return inst;
                        }));
                    }
                    go.countDown();
                    List<String> failures = new ArrayList<>();
                    List<WasmInstance> live = new ArrayList<>();
                    for (int i = 0; i < COMPUTERS; i++) {
                        try {
                            live.add(loads.get(i).get(60, TimeUnit.SECONDS));
                        } catch (ExecutionException e) {
                            StringWriter sw = new StringWriter();
                            e.getCause().printStackTrace(new PrintWriter(sw));
                            failures.add("instance " + i + ": " + sw);
                        }
                    }
                    for (WasmInstance inst : live) inst.close();
                    if (!failures.isEmpty()) {
                        fail("round " + round + ": " + failures.size() + "/" + COMPUTERS
                                + " kernels failed to load:\n" + String.join("\n", failures));
                    }
                }
            }
        } finally {
            pool.shutdownNow();
        }
    }
}
