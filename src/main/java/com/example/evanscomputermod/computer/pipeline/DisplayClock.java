package com.example.evanscomputermod.computer.pipeline;

/**
 * The clock of timed display frames and speaker audio (DISPLAY_PIPELINE_SPEC.md
 * Part C1): microseconds of {@link System#nanoTime()} from a JVM-wide origin,
 * so every computer on a server shares it.
 *
 * <p>Programs give presentation times on their own monotonic clock (WASI
 * {@code clock_time_get(MONOTONIC)}, which leaves out time the program was
 * paused). The host converts them to this clock when the call is made
 * ({@link #fromMonotonicUs}), so everything on the wire and in the queues is
 * one clock and a client needs one offset per server connection, not one per
 * computer.
 */
public final class DisplayClock {
    private DisplayClock() {}

    private static final long ORIGIN = System.nanoTime();

    /** Now, in display-clock microseconds. */
    public static long nowUs() {
        return (System.nanoTime() - ORIGIN) / 1000L;
    }

    /** A {@link System#nanoTime()} value in display-clock microseconds. */
    public static long ofNanoTime(long nanoTime) {
        return (nanoTime - ORIGIN) / 1000L;
    }

    /** A display-clock time as a {@link System#nanoTime()} value. */
    public static long toNanoTime(long us) {
        return ORIGIN + us * 1000L;
    }

    /**
     * A program's monotonic time (microseconds) as display-clock time, given
     * what its monotonic clock reads now ({@code programNowUs}): the same
     * distance from now on both clocks.
     */
    public static long fromMonotonicUs(long programUs, long programNowUs) {
        return nowUs() + (programUs - programNowUs);
    }
}
