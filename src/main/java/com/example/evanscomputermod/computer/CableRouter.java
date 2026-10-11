package com.example.evanscomputermod.computer;

import java.util.*;
import java.util.function.LongPredicate;
import java.util.function.LongToIntFunction;

/**
 * Routes several cables through a small voxel region so that no two of them touch.
 *
 * <p>Network cable conducts to every face neighbour, so two runs that must stay separate
 * segments (an ISP router's fiber ports, its server LAN) may not even sit side by side.
 * Each run goes from its start cell to its goal cell through free cells, then continues
 * along a fixed "portal" (cells outside the region, e.g. a riser up the mast). Runs are
 * routed one after another; a run may not enter or touch any cell of the others (their
 * routes and portals) nor any cell listed as forbidden. Plain Java (positions packed like
 * {@code BlockPos.asLong} via {@link FiberChords#pack}) so it is unit-testable.
 */
public final class CableRouter {
    private CableRouter() {}

    /** One cable: start cell, goal cell, then the fixed cells after the goal (may be empty). */
    public record Run(String name, long start, long goal, List<Long> portal) {}

    private static final int[][] STEPS = {{1, 0, 0}, {-1, 0, 0}, {0, 1, 0}, {0, -1, 0}, {0, 0, 1}, {0, 0, -1}};

    public static List<Long> neighbours(long p) {
        int x = FiberChords.unpackX(p), y = FiberChords.unpackY(p), z = FiberChords.unpackZ(p);
        List<Long> out = new ArrayList<>(6);
        for (int[] s : STEPS) out.add(FiberChords.pack(x + s[0], y + s[1], z + s[2]));
        return out;
    }

    /**
     * Route every run (trying each order until all fit). Returns the full cell list of each
     * run (route then portal), in input order, or null when no order works.
     *
     * @param free cells a route may use (the portal cells need not be free)
     * @param forbidden cells no route may use (other ports' faces, the screen front...)
     * @param cost cost of entering a cell (at least 1), e.g. cheaper along a ceiling
     */
    public static List<List<Long>> routeAll(List<Run> runs, LongPredicate free, Set<Long> forbidden, LongToIntFunction cost) {
        List<int[]> orders = new ArrayList<>();
        permutations(new int[runs.size()], 0, new boolean[runs.size()], orders);
        for (int[] order : orders) {
            List<List<Long>> result = new ArrayList<>(Collections.nCopies(runs.size(), null));
            boolean ok = true;
            for (int k = 0; k < order.length && ok; k++) {
                Run run = runs.get(order[k]);
                // Keep clear of everything placed so far and of the other runs' fixed cells.
                Set<Long> avoid = new HashSet<>(forbidden);
                for (int j = 0; j < runs.size(); j++) {
                    if (j == order[k]) continue;
                    List<Long> cells = new ArrayList<>(runs.get(j).portal());
                    cells.add(runs.get(j).start());
                    cells.add(runs.get(j).goal());
                    if (result.get(j) != null) cells.addAll(result.get(j));
                    for (long c : cells) {
                        avoid.add(c);
                        avoid.addAll(neighbours(c));
                    }
                }
                List<Long> route = route(run.start(), run.goal(), free, avoid, cost);
                if (route == null) ok = false;
                else {
                    List<Long> all = new ArrayList<>(route);
                    all.addAll(run.portal());
                    result.set(order[k], all);
                }
            }
            if (ok) return result;
        }
        return null;
    }

    private static void permutations(int[] cur, int at, boolean[] used, List<int[]> out) {
        if (at == cur.length) {
            out.add(cur.clone());
            return;
        }
        for (int i = 0; i < cur.length; i++)
            if (!used[i]) {
                used[i] = true;
                cur[at] = i;
                permutations(cur, at + 1, used, out);
                used[i] = false;
            }
    }

    /** Cheapest face-connected path start..goal (inclusive) through free, non-avoided cells. */
    public static List<Long> route(long start, long goal, LongPredicate free, Set<Long> avoid, LongToIntFunction cost) {
        if (avoid.contains(start) || avoid.contains(goal)) return null;
        Map<Long, Integer> dist = new HashMap<>();
        Map<Long, Long> back = new HashMap<>();
        PriorityQueue<long[]> queue = new PriorityQueue<>(Comparator.<long[]>comparingLong(a -> a[0]).thenComparingLong(a -> a[1]));
        dist.put(start, 0);
        queue.add(new long[] {0, start});
        while (!queue.isEmpty()) {
            long[] head = queue.poll();
            long c = head[1];
            if (head[0] > dist.getOrDefault(c, Integer.MAX_VALUE)) continue;
            if (c == goal) {
                List<Long> path = new ArrayList<>();
                for (Long at = c; at != null; at = back.get(at)) path.add(at);
                Collections.reverse(path);
                return path;
            }
            for (long n : neighbours(c)) {
                if (avoid.contains(n) || !(n == goal || free.test(n))) continue;
                int d = dist.get(c) + Math.max(1, cost.applyAsInt(n));
                if (d < dist.getOrDefault(n, Integer.MAX_VALUE)) {
                    dist.put(n, d);
                    back.put(n, c);
                    queue.add(new long[] {d, n});
                }
            }
        }
        return null;
    }

    /** True when no cell of one run is equal or face-adjacent to a cell of another. */
    public static boolean separate(List<List<Long>> runs) {
        for (int i = 0; i < runs.size(); i++)
            for (int j = i + 1; j < runs.size(); j++) {
                Set<Long> near = new HashSet<>();
                for (long c : runs.get(i)) {
                    near.add(c);
                    near.addAll(neighbours(c));
                }
                for (long c : runs.get(j)) if (near.contains(c)) return false;
            }
        return true;
    }

    /** True when consecutive cells are face neighbours. */
    public static boolean connected(List<Long> run) {
        for (int i = 1; i < run.size(); i++) if (!neighbours(run.get(i - 1)).contains(run.get(i))) return false;
        return true;
    }
}
