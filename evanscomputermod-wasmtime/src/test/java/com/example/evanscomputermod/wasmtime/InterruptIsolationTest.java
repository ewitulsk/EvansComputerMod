package com.example.evanscomputermod.wasmtime;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.api.wasm.*;

import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.concurrent.*;

public class InterruptIsolationTest {
    private static final byte[] MODULE = {
        0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 2, 96, 0, 0, 96, 0, 1, 127, 3, 3, 2, 0, 1, 7, 13, 2, 4,
        115, 112, 105, 110, 0, 0, 2, 111, 107, 0, 1, 10, 14, 2, 7, 0, 3, 64, 12, 0, 11, 11, 4, 0,
        65, 7, 11
    };

    @Test
    void interruptDoesNotTrapAnotherInstanceAndCanBeRearmed() throws Exception {
        try (var runtime = new WasmtimeRuntime()) {
            var module = runtime.compile(MODULE);
            var a = runtime.instantiate(module, List.of());
            var b = runtime.instantiate(module, List.of());
            var pool = Executors.newFixedThreadPool(2);
            try {
                var entered = new CountDownLatch(2);
                Future<WasmTrap.Kind> first = pool.submit(() -> spin(a, entered));
                Future<WasmTrap.Kind> second = pool.submit(() -> spin(b, entered));
                assertTrue(entered.await(3, TimeUnit.SECONDS));
                a.requestInterrupt();
                assertEquals(WasmTrap.Kind.INTERRUPTED, first.get(3, TimeUnit.SECONDS));
                assertThrows(
                        TimeoutException.class,
                        () -> second.get(200, TimeUnit.MILLISECONDS),
                        "other instance must still be executing");
                b.requestInterrupt();
                assertEquals(WasmTrap.Kind.INTERRUPTED, second.get(3, TimeUnit.SECONDS));
                a.clearInterrupt();
                b.clearInterrupt();
                assertEquals(7, a.callExport("ok")[0]);
                assertEquals(7, b.callExport("ok")[0]);
            } finally {
                a.requestInterrupt();
                b.requestInterrupt();
                pool.shutdownNow();
                assertTrue(pool.awaitTermination(3, TimeUnit.SECONDS));
                a.close();
                b.close();
            }
        }
    }

    private static WasmTrap.Kind spin(WasmInstance instance, CountDownLatch entered) {
        entered.countDown();
        try {
            instance.callExport("spin");
            throw new AssertionError("infinite loop returned");
        } catch (WasmTrap e) {
            return e.kind();
        }
    }
}
