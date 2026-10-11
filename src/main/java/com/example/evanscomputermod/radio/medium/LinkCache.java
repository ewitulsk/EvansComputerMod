package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.phys.Fading;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReferenceArray;

/**
 * Path-loss cache: an immutable snapshot map from pair key to {@link Link},
 * split into {@value #SEGMENTS} segments that are each published by a single
 * volatile swap (the {@code CableNetworkManager} pattern, per segment so a
 * publish only copies what changed). {@link #get} is lock-free and
 * allocation-free and may run on any thread; {@link #put}, {@link #remove}
 * and {@link #publish} belong to the one writer thread (the server thread).
 */
public final class LinkCache {

    public static final int SEGMENTS = 64;

    /**
     * One cached path. Endpoint "a" is the one with the lower index in the key.
     *
     * @param excessDb  path loss beyond free space at the traced geometry (walls, terrain, ground)
     * @param gainA     a's antenna gain towards b, dBi (cached with the poses)
     * @param polDb     polarization mismatch (random per pair on skywave)
     * @param kLinear   Rician K for fading (0 = Rayleigh, no line of sight)
     */
    public record Link(double excessDb, double freqHz, double gainA, double gainB, double polDb, double kLinear,
                       boolean lineOfSight, long computedTick, PathTracer.Result path) {}

    private static final class Segment {
        static final Segment EMPTY = new Segment(new long[8], new Link[8], 0);
        final long[] keys;
        final Link[] vals;
        final int size;

        Segment(long[] keys, Link[] vals, int size) {
            this.keys = keys;
            this.vals = vals;
            this.size = size;
        }
    }

    private final AtomicReferenceArray<Segment> segments = new AtomicReferenceArray<>(SEGMENTS);
    @SuppressWarnings("unchecked")
    private final Map<Long, Link>[] pending = new Map[SEGMENTS];
    private static final Link REMOVED = new Link(0, 0, 0, 0, 0, 0, false, 0, null);

    public LinkCache() {
        for (int i = 0; i < SEGMENTS; i++) segments.set(i, Segment.EMPTY);
    }

    private static long hash(long key) {
        return Fading.mix64(key);
    }

    private static int segment(long h) {
        return (int) (h >>> 58);
    }

    /** The published entry for a key, or null. Lock-free, no allocation. */
    public Link get(long key) {
        long h = hash(key);
        Segment s = segments.get(segment(h));
        long[] keys = s.keys;
        int mask = keys.length - 1;
        for (int i = (int) h & mask, n = 0; n <= mask; i = (i + 1) & mask, n++) {
            long k = keys[i];
            if (k == key) return s.vals[i];
            if (k == 0) return null;
        }
        return null;
    }

    /** Stage an entry (visible after {@link #publish}). Writer thread only. */
    public void put(long key, Link link) {
        int s = segment(hash(key));
        if (pending[s] == null) pending[s] = new HashMap<>();
        pending[s].put(key, link);
    }

    /** Stage a removal. Writer thread only. */
    public void remove(long key) {
        put(key, REMOVED);
    }

    /** Publish every staged change: each touched segment is rebuilt and swapped in. */
    public void publish() {
        for (int si = 0; si < SEGMENTS; si++) {
            Map<Long, Link> p = pending[si];
            if (p == null || p.isEmpty()) continue;
            Segment old = segments.get(si);
            int live = old.size;
            for (Map.Entry<Long, Link> e : p.entrySet()) live++;
            int cap = 8;
            while (cap < live * 2) cap <<= 1;
            long[] keys = new long[cap];
            Link[] vals = new Link[cap];
            int size = 0;
            for (int i = 0; i < old.keys.length; i++) {
                long k = old.keys[i];
                if (k == 0 || p.containsKey(k)) continue;
                size += insert(keys, vals, k, old.vals[i]);
            }
            for (Map.Entry<Long, Link> e : p.entrySet())
                if (e.getValue() != REMOVED) size += insert(keys, vals, e.getKey(), e.getValue());
            segments.set(si, new Segment(keys, vals, size));
            p.clear();
        }
    }

    private static int insert(long[] keys, Link[] vals, long key, Link v) {
        int mask = keys.length - 1;
        for (int i = (int) hash(key) & mask; ; i = (i + 1) & mask) {
            if (keys[i] == 0) {
                keys[i] = key;
                vals[i] = v;
                return 1;
            }
            if (keys[i] == key) {
                vals[i] = v;
                return 0;
            }
        }
    }

    /** Published entries. */
    public int size() {
        int n = 0;
        for (int i = 0; i < SEGMENTS; i++) n += segments.get(i).size;
        return n;
    }
}
