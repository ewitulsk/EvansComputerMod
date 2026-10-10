package com.example.evanscomputermod.computer;

import java.util.*;

/**
 * The long-distance fiber ring as block paths: chord {@code i} runs from endpoint
 * {@code i} to endpoint {@code i + 1} (wrapping). Plain Java so the bookkeeping is
 * unit-testable: positions are packed exactly like Minecraft's {@code BlockPos.asLong}
 * and chunks like {@code ChunkPos.asLong}.
 */
public final class FiberChords {
    private final int[][] endpoints;
    private final int[][][] paths;
    /** Packed position -> bitmask of the chords whose path contains it. */
    private final Map<Long, Integer> membership = new HashMap<>();
    /** Packed chunk -> packed (chord, index) entries inside it. */
    private final Map<Long, long[]> byChunk = new HashMap<>();
    private final Set<Long> endpointSet = new HashSet<>();

    public FiberChords(List<int[]> ends) {
        int n = ends.size();
        endpoints = ends.toArray(new int[0][]);
        paths = new int[n][][];
        Map<Long, List<Long>> chunks = new HashMap<>();
        for (int c = 0; c < n; c++) {
            int[] a = endpoints[c], b = endpoints[(c + 1) % n];
            endpointSet.add(pack(a[0], a[1], a[2]));
            paths[c] = FiberLine.rasterise(a[0], a[1], a[2], b[0], b[1], b[2]);
            for (int i = 0; i < paths[c].length; i++) {
                int[] p = paths[c][i];
                membership.merge(pack(p[0], p[1], p[2]), 1 << c, (x, y) -> x | y);
                chunks.computeIfAbsent(chunk(p[0] >> 4, p[2] >> 4), k -> new ArrayList<>())
                        .add(((long) c << 32) | i);
            }
        }
        chunks.forEach((k, v) -> byChunk.put(k, v.stream().mapToLong(Long::longValue).toArray()));
    }

    public int chordCount() {
        return paths.length;
    }

    public int[][] path(int chord) {
        return paths[chord];
    }

    public int[] endpoint(int village) {
        return endpoints[village];
    }

    public boolean isEndpoint(long packed) {
        return endpointSet.contains(packed);
    }

    /** Bitmask of chords through this block, 0 if none. */
    public int chordsAt(long packed) {
        return membership.getOrDefault(packed, 0);
    }

    /** Entries ((chord << 32) | index) in a chunk; empty if the ring misses it. */
    public long[] inChunk(int chunkX, int chunkZ) {
        return byChunk.getOrDefault(chunk(chunkX, chunkZ), new long[0]);
    }

    /** A chord is physically intact when none of its blocks is in {@code broken}. */
    public boolean intact(int chord, Collection<Long> broken) {
        for (long p : broken) if ((chordsAt(p) & (1 << chord)) != 0) return false;
        return true;
    }

    public int brokenCount(int chord, Collection<Long> broken) {
        int count = 0;
        for (long p : broken) if ((chordsAt(p) & (1 << chord)) != 0) count++;
        return count;
    }

    /** Neighbouring path blocks of {@code (chord, index)} (one or two). */
    public List<int[]> neighbours(int chord, int index) {
        List<int[]> out = new ArrayList<>(2);
        if (index > 0) out.add(paths[chord][index - 1]);
        if (index + 1 < paths[chord].length) out.add(paths[chord][index + 1]);
        return out;
    }

    public static long pack(int x, int y, int z) {
        return ((long) x & 0x3FFFFFFL) << 38 | ((long) y & 0xFFFL) | ((long) z & 0x3FFFFFFL) << 12;
    }

    public static int unpackX(long p) {
        return (int) (p >> 38);
    }

    public static int unpackY(long p) {
        return (int) (p << 52 >> 52);
    }

    public static int unpackZ(long p) {
        return (int) (p << 26 >> 38);
    }

    public static long chunk(int x, int z) {
        return (long) x & 0xFFFFFFFFL | ((long) z & 0xFFFFFFFFL) << 32;
    }
}
