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

    @Test
    void unknownClockIdIsRejected() {
        assertEquals(-1, WasiFunctions.clockNow(7));
    }
}
