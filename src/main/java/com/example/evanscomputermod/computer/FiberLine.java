package com.example.evanscomputermod.computer;

/**
 * A deterministic, face-connected voxel line between two block positions.
 *
 * <p>Every step moves exactly one block along one axis, so consecutive blocks share a
 * face: a Fiber Span only joins its six face neighbours. The axis taken at each step is
 * the one whose next block centre is reached earliest along the ideal straight line, so
 * the path hugs the line (a staircase where x, y and z change together). Plain Java: the
 * rasterisation is unit-tested on both Minecraft versions.
 */
public final class FiberLine {
    private FiberLine() {}

    /** Packed {x, y, z} triples, from {@code a} to {@code b} inclusive. */
    public static int[][] rasterise(int ax, int ay, int az, int bx, int by, int bz) {
        int[] d = {bx - ax, by - ay, bz - az};
        int[] n = {Math.abs(d[0]), Math.abs(d[1]), Math.abs(d[2])};
        int[] s = {Integer.signum(d[0]), Integer.signum(d[1]), Integer.signum(d[2])};
        int total = n[0] + n[1] + n[2];
        int[][] out = new int[total + 1][];
        int[] p = {ax, ay, az};
        int[] taken = new int[3];
        out[0] = p.clone();
        for (int i = 1; i <= total; i++) {
            int axis = -1;
            long bestNum = 0, bestDen = 1;
            // Axis order x, z, y on ties: horizontal runs first, then the vertical step.
            for (int a : new int[] {0, 2, 1}) {
                if (taken[a] >= n[a]) continue;
                // Parameter of the next crossing: (taken + 0.5) / n, compared exactly.
                long num = 2L * taken[a] + 1, den = 2L * n[a];
                if (axis < 0 || num * bestDen < bestNum * den) {
                    axis = a;
                    bestNum = num;
                    bestDen = den;
                }
            }
            taken[axis]++;
            p[axis] += s[axis];
            out[i] = p.clone();
        }
        return out;
    }

    /** True when every consecutive pair differs by exactly one block on exactly one axis. */
    public static boolean faceConnected(int[][] path) {
        for (int i = 1; i < path.length; i++) {
            int diff = 0;
            for (int a = 0; a < 3; a++) diff += Math.abs(path[i][a] - path[i - 1][a]);
            if (diff != 1) return false;
        }
        return true;
    }
}
