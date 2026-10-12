package com.example.evanscomputermod.computer.wasi;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

/** WASI clocks honour the clock id and have sub-millisecond resolution. */
class WasiClockTest {

    @Test
    void monotonicClockAdvancesBelowOneMillisecond() {
        long a = WasiFunctions.clockNow(WasiFunctions.CLOCK_MONOTONIC);
        long b = a;
        // A busy loop shorter than 1 ms must still see the clock move.
        while (b == a) b = WasiFunctions.clockNow(WasiFunctions.CLOCK_MONOTONIC);
        assertTrue(b - a < 1_000_000, "step was " + (b - a) + " ns");
        assertTrue(a >= 0);
    }

    @Test
    void realtimeIsTheWallClockInNanoseconds() {
        long now = WasiFunctions.clockNow(WasiFunctions.CLOCK_REALTIME);
        long ms = System.currentTimeMillis();
        assertTrue(Math.abs(now / 1_000_000 - ms) < 1000);
    }

    /** On a paused-then-resumed child thread the monotonic clock leaves out the pause. */
    @Test
    void monotonicClockSkipsPausedTime() throws Exception {
        PauseGate gate = new PauseGate();
        long[] t = new long[2];
        var started = new java.util.concurrent.CountDownLatch(1);
        var proceed = new java.util.concurrent.CountDownLatch(1);
        Thread child = new PausableThread(() -> {
            t[0] = WasiFunctions.clockNow(WasiFunctions.CLOCK_MONOTONIC);
            started.countDown();
            try { proceed.await(); } catch (InterruptedException e) { return; }
            t[1] = WasiFunctions.clockNow(WasiFunctions.CLOCK_MONOTONIC);
        }, "clock-test", gate);
        child.start();
        assertTrue(started.await(2, java.util.concurrent.TimeUnit.SECONDS));
        long wall0 = System.nanoTime();
        gate.pause(PauseGate.REASON_PROCESS);
        Thread.sleep(300);
        gate.resume(PauseGate.REASON_PROCESS);
        proceed.countDown();
        child.join(2000);
        long wall = System.nanoTime() - wall0;
        assertTrue(wall >= 300_000_000L);
        assertTrue(gate.pausedNanos() >= 300_000_000L);
        assertTrue(t[1] - t[0] < wall - 250_000_000L,
                "monotonic advanced " + (t[1] - t[0]) + " ns over a " + wall + " ns window with 300 ms paused");
        // Off a child thread the clock is the plain monotonic clock.
        assertEquals(0, PauseGate.currentPausedNanos());
    }

    @Test
    void unknownClockIdIsRejected() {
        assertEquals(-1, WasiFunctions.clockNow(7));
    }
}
