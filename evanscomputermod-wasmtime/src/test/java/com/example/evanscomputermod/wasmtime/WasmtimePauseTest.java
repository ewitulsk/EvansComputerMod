package com.example.evanscomputermod.wasmtime;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.api.wasm.WasmValType;
import com.example.evanscomputermod.computer.wasi.PausableThread;
import com.example.evanscomputermod.computer.wasi.PauseGate;
import com.example.evanscomputermod.testing.WasmTestModules;

import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Pause/resume of a guest on the Wasmtime sidecar, set up the way
 * {@code ProcessManager} runs a WASI child: a {@link PausableThread} with a
 * {@link PauseGate} that brackets every host call ({@link PauseGate#wrap}).
 * (ProcessManager itself can't load here: the sidecar's tests have no FML.)
 *
 * <p>Wasmtime runs guest code natively and wasmtime-java 0.19 has no
 * resumable interruption (its epoch deadline only traps), so a guest pauses
 * at its next host call: a loop that writes a dot every few thousand
 * iterations stops within a dot, and its pure-wasm counter stops exactly at
 * a multiple of the loop length.
 */
public class WasmtimePauseTest {

    private static final int INNER = 200_000;

    @Test
    void guestPausesAtItsNextHostCallAndResumes() throws Exception {
        AtomicLong dots = new AtomicLong();
        AtomicLong counterAtLastDot = new AtomicLong();
        WasmHostFunc fdWrite = new WasmHostFunc("wasi_snapshot_preview1", "fd_write",
                List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32),
                List.of(WasmValType.I32), (inst, args) -> {
                    counterAtLastDot.set(Integer.toUnsignedLong(
                            inst.memory().readInt(WasmTestModules.SPIN_COUNTER_ADDR)));
                    dots.incrementAndGet();
                    return WasmHostFunc.retI32(0);
                });

        try (var runtime = new WasmtimeRuntime()) {
            var module = runtime.compile(WasmTestModules.spinCounter(INNER));
            PauseGate gate = new PauseGate();
            AtomicReference<WasmInstance> instance = new AtomicReference<>();
            AtomicReference<Object> outcome = new AtomicReference<>();
            Thread child = new PausableThread(() -> {
                try {
                    WasmInstance inst = runtime.instantiate(module, List.of(gate.wrap(fdWrite)));
                    instance.set(inst);
                    gate.setWasmActive(true);
                    inst.callExport("_start");
                    outcome.set("returned");
                } catch (WasmTrap e) {
                    outcome.set(e.kind());
                } catch (Throwable t) {
                    outcome.set(t);
                } finally {
                    gate.setWasmActive(false);
                }
            }, "WASI-PID-test", gate);
            child.setDaemon(true);
            child.start();
            try {
                waitFor(() -> dots.get() > 20, 10_000, "dots flowing");

                gate.pause(PauseGate.REASON_PROCESS);
                waitFor(gate::isParked, 3_000, "guest parked at a host call");
                long d1 = dots.get();
                long c1 = counterAtLastDot.get();
                Thread.sleep(1_000);
                assertEquals(d1, dots.get(), "host calls while paused");
                assertTrue(gate.isParked());
                assertTrue(child.isAlive());
                assertEquals(0, c1 % INNER, "a Wasmtime pause lands at a host call");

                gate.resume(PauseGate.REASON_PROCESS);
                waitFor(() -> dots.get() > d1 + 20, 5_000, "dots after resume");
                assertTrue(counterAtLastDot.get() > c1, "state kept: the counter carried on");
                System.out.println("pause/resume on Wasmtime: stopped at " + d1 + " dots");

                // A kill while paused ends the guest without running it.
                gate.pause(PauseGate.REASON_PROCESS);
                waitFor(gate::isParked, 3_000, "parked again");
                long d2 = dots.get();
                child.interrupt();
                child.join(3_000);
                assertFalse(child.isAlive(), "paused guest didn't end on interrupt");
                assertEquals(d2, dots.get());
                assertNotEquals("returned", outcome.get());
            } finally {
                WasmInstance inst = instance.get();
                if (inst != null) {
                    inst.requestInterrupt();
                    child.join(3_000);
                    inst.close();
                }
            }
        }
    }

    private static void waitFor(java.util.function.BooleanSupplier cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.getAsBoolean()) return;
            Thread.sleep(5);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
    }
}
