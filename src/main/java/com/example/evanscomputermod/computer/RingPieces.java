package com.example.evanscomputermod.computer;

import java.util.*;

/**
 * The generated fiber ring as conducting pieces: each chord's path split at its breaks
 * (removed path blocks and administrative cuts). Plain Java and independent of what is
 * loaded: a piece is defined by the plan and the recorded breaks only, so it works in
 * loaded, unloaded and never-generated chunks alike.
 *
 * <p>Blocks attached to a piece are the blocks face-adjacent to one of its path blocks
 * that may conduct into it: the two ISP patch panels under the chord's endpoints, and
 * any patch panel or Fiber Span a player put against the path (a tap). Whether such a
 * block really is there is decided by the segment flood fill reading the world.
 */
public final class RingPieces implements SegmentGraph.Ring {
    private final FiberChords chords;
    private final int dim;
    /** Per chord: sorted break indices. */
    private final int[][] breaks;
    /** Per chord: id of its first piece. */
    private final int[] base;
    private final int count;
    /** Per chord: path index -> attached positions next to that path block. */
    private final List<TreeMap<Integer, long[]>> attached = new ArrayList<>();

    /**
     * @param broken removed path blocks (packed positions)
     * @param cuts per chord, extra break indices (administrative cuts)
     * @param attachments candidate attached blocks (packed positions next to the path)
     */
    public RingPieces(FiberChords chords, int dim, Collection<Long> broken, Map<Integer, int[]> cuts,
            Collection<Long> attachments) {
        this.chords = chords;
        this.dim = dim;
        int n = chords.chordCount();
        breaks = new int[n][];
        base = new int[n];
        List<SortedSet<Integer>> sets = new ArrayList<>();
        for (int c = 0; c < n; c++) sets.add(new TreeSet<>());
        for (long p : broken)
            for (long e : chords.entriesAt(p)) sets.get((int) (e >> 32)).add((int) e);
        cuts.forEach((c, idx) -> {
            if (c >= 0 && c < n) for (int i : idx) sets.get(c).add(i);
        });
        int id = 0;
        for (int c = 0; c < n; c++) {
            breaks[c] = sets.get(c).stream().mapToInt(Integer::intValue).toArray();
            base[c] = id;
            id += pieceCountOf(c);
            attached.add(new TreeMap<>());
        }
        count = id;
        for (long a : new LinkedHashSet<>(attachments)) addAttachment(a);
    }

    private void addAttachment(long a) {
        for (int dir = 0; dir < 6; dir++)
            for (long e : chords.entriesAt(SegmentGraph.neighbour(a, dir))) {
                int c = (int) (e >> 32), i = (int) e;
                attached.get(c).merge(i, new long[] {a}, (x, y) -> {
                    for (long v : x) if (v == a) return x;
                    long[] r = Arrays.copyOf(x, x.length + 1);
                    r[x.length] = a;
                    return r;
                });
            }
    }

    /** True when {@code pos} is next to a ring path block (a tap candidate). */
    public static boolean touchesPath(FiberChords chords, long pos) {
        for (int dir = 0; dir < 6; dir++) if (chords.entriesAt(SegmentGraph.neighbour(pos, dir)).length > 0) return true;
        return false;
    }

    private int pieceCountOf(int chord) {
        int len = chords.path(chord).length, pieces = 0, prev = -1;
        for (int b : breaks[chord]) {
            if (b > prev + 1) pieces++;
            prev = b;
        }
        if (len - 1 > prev) pieces++;
        return pieces;
    }

    /** Piece id of a chord's path index, -1 if that index is a break. */
    public int piece(int chord, int index) {
        int[] b = breaks[chord];
        int k = Arrays.binarySearch(b, index);
        if (k >= 0) return -1;
        int before = -k - 1; // breaks below index
        // Pieces before: count gaps among the first `before` breaks.
        int pieces = 0, prev = -1;
        for (int j = 0; j < before; j++) {
            if (b[j] > prev + 1) pieces++;
            prev = b[j];
        }
        return base[chord] + pieces;
    }

    /** {chord, first index, last index} of a piece. */
    public int[] range(int piece) {
        int c = 0;
        while (c + 1 < base.length && base[c + 1] <= piece) c++;
        int k = piece - base[c], prev = -1, seen = 0;
        int[] b = breaks[c];
        for (int j = 0; j <= b.length; j++) {
            int end = j < b.length ? b[j] : chords.path(c).length;
            if (end > prev + 1) {
                if (seen == k) return new int[] {c, prev + 1, end - 1};
                seen++;
            }
            prev = end;
        }
        throw new IllegalArgumentException("no piece " + piece);
    }

    public int[] breaks(int chord) {
        return breaks[chord].clone();
    }

    /** Pieces of one chord, in path order. */
    public int[] piecesOf(int chord) {
        int n = pieceCountOf(chord);
        int[] out = new int[n];
        for (int i = 0; i < n; i++) out[i] = base[chord] + i;
        return out;
    }

    /** Positions attached to a chord's path, with the path index they touch. */
    public SortedMap<Integer, long[]> attachedTo(int chord) {
        return Collections.unmodifiableSortedMap(attached.get(chord));
    }

    @Override
    public int dim() {
        return dim;
    }

    @Override
    public int[] piecesAt(long pos) {
        long[] at = chords.entriesAt(pos);
        if (at.length == 0) return new int[0];
        int[] out = new int[at.length];
        int k = 0;
        for (long e : at) {
            int p = piece((int) (e >> 32), (int) e);
            if (p >= 0) out[k++] = p;
        }
        return k == out.length ? out : Arrays.copyOf(out, k);
    }

    @Override
    public long[] attachments(int piece) {
        int[] r = range(piece);
        LinkedHashSet<Long> out = new LinkedHashSet<>();
        for (long[] v : attached.get(r[0]).subMap(r[1], true, r[2], true).values()) for (long a : v) out.add(a);
        return out.stream().mapToLong(Long::longValue).toArray();
    }

    @Override
    public int pieceCount() {
        return count;
    }
}
