package com.example.evanscomputermod.computer;

import java.util.*;

/**
 * The long-distance fiber ring as block paths. Every site has two endpoints, one above
 * each of its two patch panels: chord {@code i} runs from site {@code i}'s "next"
 * endpoint to site {@code i + 1}'s "previous" endpoint (wrapping), so neighbouring chords
 * never share a block at a mast. Plain Java so the bookkeeping is unit-testable:
 * positions are packed exactly like Minecraft's {@code BlockPos.asLong} and chunks like
 * {@code ChunkPos.asLong}.
 */
public final class FiberChords {
    private final int[][] nextEnds, prevEnds;
    private final int[][][] paths;
    /** Packed position -> bitmask of the chords whose path contains it. */
    private final Map<Long, Integer> membership = new HashMap<>();
    /** Packed chunk -> packed (chord, index) entries inside it. */
    private final Map<Long, long[]> byChunk = new HashMap<>();
    private final Set<Long> endpointSet = new HashSet<>();
    /** Packed position -> its (chord << 32 | index) entries (two at a shared endpoint). */
    private final Map<Long, long[]> entries = new HashMap<>();

    /** One endpoint per site, shared by its two chords (the single-panel layout). */
    public FiberChords(List<int[]> ends) {
        this(ends, ends);
    }

    /** Chord {@code i}: {@code nextEnds[i]} to {@code prevEnds[i + 1]}. */
    public FiberChords(List<int[]> nextEnds, List<int[]> prevEnds) {
        int n = nextEnds.size();
        if (prevEnds.size() != n) throw new IllegalArgumentException("endpoint lists differ in length");
        this.nextEnds = nextEnds.toArray(new int[0][]);
        this.prevEnds = prevEnds.toArray(new int[0][]);
        paths = new int[n][][];
        Map<Long, List<Long>> chunks = new HashMap<>();
        for (int c = 0; c < n; c++) {
            int[] a = this.nextEnds[c], b = this.prevEnds[(c + 1) % n];
            endpointSet.add(pack(a[0], a[1], a[2]));
            endpointSet.add(pack(b[0], b[1], b[2]));
            paths[c] = FiberLine.rasterise(a[0], a[1], a[2], b[0], b[1], b[2]);
            for (int i = 0; i < paths[c].length; i++) {
                int[] p = paths[c][i];
                long packed = pack(p[0], p[1], p[2]);
                membership.merge(packed, 1 << c, (x, y) -> x | y);
                long entry = ((long) c << 32) | i;
                entries.merge(packed, new long[] {entry}, (x, y) -> {
                    long[] r = Arrays.copyOf(x, x.length + 1);
                    r[x.length] = entry;
                    return r;
                });
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

    /** Site {@code village}'s (0-based) endpoint toward the next site or the previous one. */
    public int[] endpoint(int village, boolean next) {
        return (next ? nextEnds : prevEnds)[village];
    }

    /** Horizontal Direction ordinals (Minecraft order): NORTH 2, SOUTH 3, WEST 4, EAST 5. */
    public static final int NORTH = 2, SOUTH = 3, WEST = 4, EAST = 5;

    /** Unit (dx, dz) of a horizontal Direction ordinal. */
    public static int[] step(int side) {
        return switch (side) {
            case NORTH -> new int[] {0, -1};
            case SOUTH -> new int[] {0, 1};
            case WEST -> new int[] {-1, 0};
            case EAST -> new int[] {1, 0};
            default -> throw new IllegalArgumentException("not a horizontal side: " + side);
        };
    }

    /**
     * Which sides of a mast at (x, z) get the panels toward the previous and the next site:
     * {prevSide, nextSide}, each the side facing its neighbour as squarely as possible, and
     * never the same side. Opposite sides are preferred. As a chord's path is monotonic on
     * every axis, a chord leaving a side that faces its target never crosses back over the
     * mast or the other panel.
     */
    public static int[] sides(int x, int z, int prevX, int prevZ, int nextX, int nextZ) {
        double[] toPrev = unit(prevX - x, prevZ - z), toNext = unit(nextX - x, nextZ - z);
        int[] best = null;
        double bestScore = Double.NEGATIVE_INFINITY;
        for (int p = NORTH; p <= EAST; p++)
            for (int n = NORTH; n <= EAST; n++) {
                if (p == n) continue;
                int[] sp = step(p), sn = step(n);
                double score = sp[0] * toPrev[0] + sp[1] * toPrev[1] + sn[0] * toNext[0] + sn[1] * toNext[1];
                if (sp[0] == -sn[0] && sp[1] == -sn[1]) score += 0.25;
                if (score > bestScore + 1e-9) {
                    bestScore = score;
                    best = new int[] {p, n};
                }
            }
        return best;
    }

    private static double[] unit(double dx, double dz) {
        double len = Math.sqrt(dx * dx + dz * dz);
        return len == 0 ? new double[] {0, 0} : new double[] {dx / len, dz / len};
    }

    public boolean isEndpoint(long packed) {
        return endpointSet.contains(packed);
    }

    /** Bitmask of chords through this block, 0 if none. */
    public int chordsAt(long packed) {
        return membership.getOrDefault(packed, 0);
    }

    private static final long[] NONE = new long[0];

    /** Entries ((chord << 32) | index) of the path blocks at this position; empty if none. */
    public long[] entriesAt(long packed) {
        return entries.getOrDefault(packed, NONE);
    }

    /** Index of a chord's midpoint (where an administrative cut is placed). */
    public int midpoint(int chord) {
        return paths[chord].length / 2;
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

    /**
     * Directions (bit {@code 1 << Direction.ordinal()}, in Minecraft's DOWN, UP, NORTH, SOUTH,
     * WEST, EAST order) in which the path block at {@code packed} joins its path neighbours,
     * plus DOWN at an endpoint (the patch panel). 0 if the position is not on the ring.
     */
    public int arms(long packed) {
        long[] at = entries.get(packed);
        if (at == null) return 0;
        int x = unpackX(packed), y = unpackY(packed), z = unpackZ(packed), mask = 0;
        for (long e : at)
            for (int[] n : neighbours((int) (e >> 32), (int) e)) mask |= 1 << direction(n[0] - x, n[1] - y, n[2] - z);
        if (isEndpoint(packed)) mask |= 1;
        return mask;
    }

    /** Minecraft Direction ordinal of a unit step. */
    public static int direction(int dx, int dy, int dz) {
        if (dy < 0) return 0;
        if (dy > 0) return 1;
        if (dz < 0) return 2;
        if (dz > 0) return 3;
        if (dx < 0) return 4;
        return 5;
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
