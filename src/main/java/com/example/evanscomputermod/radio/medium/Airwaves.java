package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.phys.SpectralMask;

import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReferenceArray;

/**
 * One band shard's emissions on the air: a fixed lock-free ring for packet-length
 * emissions plus a copy-on-write list of long ones (no shared
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
    public static final class Active {
        private final Emission emission;
        private final Node sender;
        private final Pose pose;
        private final SpectralMask mask;
        /** The latest end time of this and every emission added to the ring before it (set by {@link #add}). */
        volatile long endsByNow = Long.MAX_VALUE;

        public Active(Emission emission, Node sender, Pose pose, SpectralMask mask) {
            this.emission = emission;
            this.sender = sender;
            this.pose = pose;
            this.mask = mask;
        }

        public Emission emission() { return emission; }
        public Node sender() { return sender; }
        public Pose pose() { return pose; }
        public SpectralMask mask() { return mask; }
    }

    /** Emissions longer than this (SDR streams, jammers) live in a small separate list, not the ring. */
    public static final long LONG_MICROS = 100_000;
    private static final Active[] NONE = new Active[0];

    private final AtomicReferenceArray<Active> ring = new AtomicReferenceArray<>(SIZE);
    private final AtomicLong head = new AtomicLong();
    /** Latest end time of anything added to the ring so far. */
    private final AtomicLong maxEnd = new AtomicLong(Long.MIN_VALUE);
    private volatile Active[] longs = NONE;

    public void add(Active a) {
        if (a.emission().durationMicros() > LONG_MICROS) {
            addLong(a);
            return;
        }
        a.endsByNow = maxEnd.accumulateAndGet(a.emission().endMicros(), Math::max);
        long i = head.getAndIncrement();
        ring.set((int) (i & MASK), a);
    }

    private synchronized void addLong(Active a) {
        long cutoff = a.emission().startMicros() - KEEP_MICROS;
        java.util.ArrayList<Active> keep = new java.util.ArrayList<>(longs.length + 1);
        for (Active o : longs) if (o.emission().endMicros() >= cutoff) keep.add(o);
        keep.add(a);
        longs = keep.toArray(NONE);
    }

    /** Long emissions (copy-on-write snapshot; may include ended ones). */
    public Active[] longs() {
        return longs;
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

    /**
     * True once a scan back from the head has passed every ring emission that could
     * still be on air at {@code micros}: this one and every emission added before it end
     * before {@code micros} (1 ms slack for concurrent writers). The ring is in the order
     * emissions were sent, not their start times: an SDR schedules its samples ahead of the
     * clock (a transmit lead of a few hundred ms), so an emission sent earlier can start later
     * than one sent after it by another transmitter. Judging by the entry's own start time
     * stopped scans at such a later-sent, earlier-starting entry and skipped the earlier-sent
     * one still on the air (SDR receivers lost samples whenever two transmitters overlapped).
     */
    public boolean olderThan(Active a, long micros) {
        return a.endsByNow < micros - 1_000;
    }
}
