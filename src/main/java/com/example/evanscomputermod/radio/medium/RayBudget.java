package com.example.evanscomputermod.radio.medium;

/**
 * Per-tick ray budget shared by every radio-side world tracer (link cache now,
 * later SDR/hazard probes): {@code LidarScanner.claim} generalised to any
 * owner and a configurable size ({@code RadioConfig.raysPerTick}). The lidar
 * keeps its own 4096-ray budget, so sensors and radios cannot starve each other.
 * Pure; the caller supplies the tick number.
 */
public final class RayBudget {

    private final int defaultPerTick;
    private long tick = Long.MIN_VALUE;
    private int used;
    private int perTick;

    public RayBudget(int perTick) {
        this.defaultPerTick = perTick;
        this.perTick = perTick;
    }

    /** Claim up to {@code want} rays from tick {@code now}'s budget of {@code perTick}; returns the grant. */
    public synchronized int claim(long now, int perTick, int want) {
        if (now != tick) {
            tick = now;
            used = 0;
        }
        this.perTick = perTick;
        int granted = Math.max(0, Math.min(want, perTick - used));
        used += granted;
        return granted;
    }

    public synchronized int claim(long now, int want) {
        return claim(now, perTick > 0 ? perTick : defaultPerTick, want);
    }

    /** Rays left in tick {@code now}. */
    public synchronized int remaining(long now) {
        return now == tick ? Math.max(0, perTick - used) : perTick;
    }
}
