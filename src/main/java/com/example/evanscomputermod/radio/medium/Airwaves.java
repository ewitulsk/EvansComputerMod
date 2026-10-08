package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.phys.SpectralMask;

import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReferenceArray;

/**
 * One band shard's emissions on the air: a fixed lock-free ring (no shared
 * state with other bands). Emissions stay readable for {@link #KEEP_MICROS}
 * after they end (late overlaps, SDR synthesis windows) unless the ring wraps.
 * Writers claim a slot with one atomic increment; readers scan backwards from
 * the head and stop once emissions are older than the window they need.
 */
public final class Airwaves {

    public static final int SIZE = 1 << 13;
    public static final long KEEP_MICROS = 1_000_000;
    private static final int MASK = SIZE - 1;

    /** One emission on the air with its sender, the sender's pose at the time and its spectral mask. */
    public record Active(Emission emission, Node sender, Pose pose, SpectralMask mask) {}

    private final AtomicReferenceArray<Active> ring = new AtomicReferenceArray<>(SIZE);
    private final AtomicLong head = new AtomicLong();
    private volatile long maxDurationMicros = 1;

    public void add(Active a) {
        long i = head.getAndIncrement();
        ring.set((int) (i & MASK), a);
        long d = a.emission().durationMicros();
        if (d > maxDurationMicros) maxDurationMicros = d;
    }

    /** Newest-first cursor start (pass to {@link #at}). */
    public long head() {
        return head.get();
    }

    /** Entry {@code i} (head-1 is the newest), or null if overwritten / not yet written. */
    public Active at(long i) {
        return ring.get((int) (i & MASK));
    }

    /** Oldest index still in the ring for a cursor started at {@code head}. */
    public static long tail(long head) {
        return Math.max(0, head - SIZE);
    }

    /** True once a scan back from the head has passed every emission that could still be on air at {@code micros}. */
    public boolean olderThan(Active a, long micros) {
        return a.emission().startMicros() + maxDurationMicros + KEEP_MICROS / 1000 < micros;
    }
}
