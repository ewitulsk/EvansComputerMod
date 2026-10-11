package com.example.evanscomputermod.computer;

import java.util.*;

/**
 * Cable segments: which NICs share a wire. Pure Java (no Minecraft types), so the rules
 * are unit-tested on both game versions.
 *
 * <p>A flood fill starts at every NIC's exit block (the block on the computer's face).
 * Copper (network cable, patch panels, the internet gateway) joins copper; Fiber Span
 * joins only fiber and patch panels, so a copper cable or a computer face touching a span
 * is not connected (the patch panel is the copper/fiber transition).
 *
 * <p>The generated fiber ring is not walked block by block: each chord is split into
 * <em>pieces</em> (runs of path between breaks, {@link Ring}). Reaching any block of a
 * piece reaches the whole piece at once, and continues from the blocks attached to it
 * (the two ISP patch panels at its ends and any panel or span a player put against it),
 * so the cost does not depend on chord length.
 *
 * <p>Logical edges (lab leads, the internet uplink) and stand-ins (a NIC treated as
 * plugged into a piece before its building exists) are merged with union-find.
 */
public final class SegmentGraph {
    private SegmentGraph() {}

    /** Block codes (also the values of the saved last-known snapshot). */
    public static final int NONE = 0, CABLE = 1, GATEWAY = 2, FIBER = 3, PANEL = 4;

    /** Do two face-neighbouring blocks conduct into each other? */
    public static boolean joins(int a, int b) {
        if (a == NONE || b == NONE) return false;
        if (a == FIBER) return b == FIBER || b == PANEL;
        if (b == FIBER) return a == PANEL;
        return true;
    }

    /** A NIC whose exit block is copper (not fiber, not empty) is cabled. */
    public static boolean attaches(int code) {
        return code != NONE && code != FIBER;
    }

    /** Block code at a position (loaded world, or the last-known snapshot). */
    public interface Blocks {
        int code(int dim, long pos);
    }

    /** The generated ring, split into pieces. */
    public interface Ring {
        /** The dimension the ring lives in. */
        int dim();

        /** Pieces conducting at {@code pos}; empty when it is not a live ring block. */
        int[] piecesAt(long pos);

        /** Blocks attached to a piece (packed positions): its end panels and taps. */
        long[] attachments(int piece);

        int pieceCount();
    }

    public record Exit<K>(K nic, int dim, long pos) {}

    /** A logical link between two NICs; {@code internet} marks the segment as online. */
    public record Edge<K>(K a, K b, boolean internet) {}

    /** {@code nic} counts as plugged into {@code piece}. */
    public record StandIn<K>(K nic, int piece) {}

    public static final class Result<K> {
        /** NIC -> segment id (dense, 0..). NICs on no segment are absent. */
        public final Map<K, Integer> segment;
        public final Map<Integer, List<K>> members;
        /** Segments that reach the internet gateway or an internet logical link. */
        public final Set<Integer> internet;
        /** Segments that contain fiber (player spans or ring pieces). */
        public final Set<Integer> optical;
        /** Ring piece -> segment id, -1 when no NIC reaches it. */
        public final int[] pieceSegment;
        /** Blocks read by the flood fill (cost measure). */
        public final int blocksVisited;

        Result(Map<K, Integer> segment, Map<Integer, List<K>> members, Set<Integer> internet, Set<Integer> optical,
                int[] pieceSegment, int blocksVisited) {
            this.segment = segment;
            this.members = members;
            this.internet = internet;
            this.optical = optical;
            this.pieceSegment = pieceSegment;
            this.blocksVisited = blocksVisited;
        }

        /**
         * Physical link state: a NIC has carrier when its segment has a partner on it
         * (another NIC, or the internet gateway). A cable or fiber that ends nowhere, or a
         * fiber cut between this NIC and every other, gives no link.
         */
        public boolean carrier(K nic) {
            Integer s = segment.get(nic);
            return s != null && (internet.contains(s) || members.getOrDefault(s, List.of()).size() >= 2);
        }
    }

    private static final int[][] STEPS = {{0, 1, 0}, {0, -1, 0}, {0, 0, -1}, {0, 0, 1}, {1, 0, 0}, {-1, 0, 0}};

    /** Packed neighbour of {@code pos} (BlockPos.asLong layout, see FiberChords.pack). */
    public static long neighbour(long pos, int dir) {
        int[] s = STEPS[dir];
        return FiberChords.pack(FiberChords.unpackX(pos) + s[0], FiberChords.unpackY(pos) + s[1],
                FiberChords.unpackZ(pos) + s[2]);
    }

    public static <K> Result<K> compute(Collection<Exit<K>> exits, Blocks blocks, Ring ring,
            Collection<Edge<K>> edges, Collection<StandIn<K>> standIns) {
        Uf uf = new Uf();
        Map<K, Integer> comp = new HashMap<>();
        BitSet gateway = new BitSet(), optical = new BitSet(), online = new BitSet();
        int pieces = ring == null ? 0 : ring.pieceCount();
        int[] pieceComp = new int[pieces];
        Arrays.fill(pieceComp, -1);
        Map<Integer, Map<Long, List<K>>> byPos = new HashMap<>();
        for (Exit<K> e : exits)
            byPos.computeIfAbsent(e.dim(), d -> new HashMap<>()).computeIfAbsent(e.pos(), p -> new ArrayList<>()).add(e.nic());
        Map<Integer, Map<Long, Integer>> visited = new HashMap<>();
        int read = 0;
        ArrayDeque<long[]> queue = new ArrayDeque<>();
        for (Exit<K> e : exits) {
            if (comp.containsKey(e.nic())) continue;
            Map<Long, Integer> seen = visited.computeIfAbsent(e.dim(), d -> new HashMap<>());
            if (seen.containsKey(e.pos())) continue;
            int start = blocks.code(e.dim(), e.pos());
            read++;
            if (!attaches(start)) continue;
            int c = uf.make();
            Map<Long, List<K>> here = byPos.get(e.dim());
            boolean ringDim = ring != null && ring.dim() == e.dim();
            seen.put(e.pos(), start);
            queue.add(new long[] {e.pos(), start});
            while (!queue.isEmpty()) {
                long[] cur = queue.poll();
                long pos = cur[0];
                int code = (int) cur[1];
                if (code == GATEWAY) gateway.set(c);
                if (code == FIBER) optical.set(c);
                if (attaches(code))
                    for (K nic : here.getOrDefault(pos, List.of())) comp.putIfAbsent(nic, c);
                for (int dir = 0; dir < 6; dir++) {
                    long n = neighbour(pos, dir);
                    if (seen.containsKey(n)) continue;
                    if (ringDim) {
                        int[] at = ring.piecesAt(n);
                        if (at.length > 0) {
                            // A ring block: fiber, so only fiber and panels join it. Entering
                            // it reaches its whole piece and every block attached to it.
                            if (!joins(code, FIBER)) continue;
                            for (int p : at) {
                                if (pieceComp[p] >= 0) {
                                    uf.union(c, pieceComp[p]);
                                    continue;
                                }
                                pieceComp[p] = c;
                                optical.set(c);
                                for (long a : ring.attachments(p)) {
                                    if (seen.containsKey(a)) continue;
                                    int ac = blocks.code(e.dim(), a);
                                    read++;
                                    if (!joins(FIBER, ac)) continue;
                                    seen.put(a, ac);
                                    queue.add(new long[] {a, ac});
                                }
                            }
                            continue;
                        }
                    }
                    int next = blocks.code(e.dim(), n);
                    read++;
                    if (joins(code, next)) {
                        seen.put(n, next);
                        queue.add(new long[] {n, next});
                    }
                }
            }
        }
        for (Edge<K> l : edges) {
            int a = comp.computeIfAbsent(l.a(), k -> uf.make());
            if (l.b() != null) uf.union(a, comp.computeIfAbsent(l.b(), k -> uf.make()));
            if (l.internet()) online.set(a);
        }
        for (StandIn<K> s : standIns) {
            if (s.piece() < 0 || s.piece() >= pieces) continue;
            int a = comp.computeIfAbsent(s.nic(), k -> uf.make());
            if (pieceComp[s.piece()] < 0) {
                pieceComp[s.piece()] = uf.make();
                optical.set(pieceComp[s.piece()]);
            }
            uf.union(a, pieceComp[s.piece()]);
        }
        // Dense ids per root, in order of first NIC seen.
        Map<Integer, Integer> dense = new HashMap<>();
        Map<K, Integer> segment = new HashMap<>();
        Map<Integer, List<K>> members = new HashMap<>();
        Set<Integer> internet = new HashSet<>(), fiber = new HashSet<>();
        for (var en : comp.entrySet()) {
            int root = uf.find(en.getValue());
            int id = dense.computeIfAbsent(root, r -> dense.size());
            segment.put(en.getKey(), id);
            members.computeIfAbsent(id, k -> new ArrayList<>()).add(en.getKey());
        }
        int[] pieceSegment = new int[pieces];
        for (int p = 0; p < pieces; p++)
            pieceSegment[p] = pieceComp[p] < 0 ? -1 : dense.getOrDefault(uf.find(pieceComp[p]), -1);
        for (int x = 0; x < uf.size(); x++) {
            Integer id = dense.get(uf.find(x));
            if (id == null) continue;
            if (gateway.get(x) || online.get(x)) internet.add(id);
            if (optical.get(x)) fiber.add(id);
        }
        Map<Integer, List<K>> frozen = new HashMap<>();
        members.forEach((k, v) -> frozen.put(k, List.copyOf(v)));
        return new Result<>(segment, frozen, internet, fiber, pieceSegment, read);
    }

    /** Union-find with path halving over a growable int array. */
    private static final class Uf {
        private int[] parent = new int[64];
        private int size;

        int make() {
            if (size == parent.length) parent = Arrays.copyOf(parent, size * 2);
            parent[size] = size;
            return size++;
        }

        int size() {
            return size;
        }

        int find(int x) {
            while (parent[x] != x) {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            return x;
        }

        void union(int a, int b) {
            a = find(a);
            b = find(b);
            if (a != b) parent[Math.max(a, b)] = Math.min(a, b);
        }
    }
}
