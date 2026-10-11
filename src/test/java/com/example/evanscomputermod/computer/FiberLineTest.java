package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import java.util.*;

import static org.junit.jupiter.api.Assertions.*;

/** The generated fiber ring: face-connected paths, chunk index and cut/repair bookkeeping. */
class FiberLineTest {
    @Test
    void diagonalRisingLineIsFaceConnectedAndHugsTheLine() {
        int[][] p = FiberLine.rasterise(4, 80, 4, 3094, 112, -1530);
        assertArrayEquals(new int[] {4, 80, 4}, p[0]);
        assertArrayEquals(new int[] {3094, 112, -1530}, p[p.length - 1]);
        assertEquals(3090 + 32 + 1534 + 1, p.length, "one block per unit step");
        assertTrue(FiberLine.faceConnected(p));
        // Every block stays within one block (per axis) of the ideal straight line.
        double dx = 3090, dy = 32, dz = -1534;
        for (int[] b : p) {
            double t = ((b[0] - 4) * dx + (b[1] - 80) * dy + (b[2] - 4) * dz) / (dx * dx + dy * dy + dz * dz);
            assertTrue(Math.abs(4 + t * dx - b[0]) <= 1.5, "x drift at " + Arrays.toString(b));
            assertTrue(Math.abs(80 + t * dy - b[1]) <= 1.5, "y drift at " + Arrays.toString(b));
            assertTrue(Math.abs(4 + t * dz - b[2]) <= 1.5, "z drift at " + Arrays.toString(b));
        }
    }

    @Test
    void controlDiagonalNaiveLineIsNotFaceConnected() {
        // The old generator stepped x and z together; the check must reject that.
        int[][] naive = {{0, 0, 0}, {1, 0, 1}, {2, 0, 2}};
        assertFalse(FiberLine.faceConnected(naive));
        assertTrue(FiberLine.faceConnected(FiberLine.rasterise(0, 0, 0, 2, 0, 2)));
    }

    @Test
    void degenerateAndAxisLines() {
        assertEquals(1, FiberLine.rasterise(5, 5, 5, 5, 5, 5).length);
        int[][] down = FiberLine.rasterise(0, 10, 0, 0, 0, 0);
        assertEquals(11, down.length);
        assertTrue(FiberLine.faceConnected(down));
    }

    @Test
    void packingMatchesMinecraftLayout() {
        for (int[] v : new int[][] {{0, 0, 0}, {-1, -64, -1}, {5000, 319, -4999}, {-30000000 + 1, 100, 29999999}}) {
            long p = FiberChords.pack(v[0], v[1], v[2]);
            assertEquals(v[0], FiberChords.unpackX(p));
            assertEquals(v[1], FiberChords.unpackY(p));
            assertEquals(v[2], FiberChords.unpackZ(p));
        }
    }

    @Test
    void panelSidesFaceTheirNeighbours() {
        // Ring tangent along x: previous village to the west, next to the east.
        assertArrayEquals(new int[] {FiberChords.WEST, FiberChords.EAST}, FiberChords.sides(0, 0, -3000, 400, 3000, 400));
        // Tangent along z.
        assertArrayEquals(new int[] {FiberChords.SOUTH, FiberChords.NORTH}, FiberChords.sides(0, 0, 300, 3000, 300, -3000));
        // Both neighbours on the same side: two different sides, each facing its target.
        int[] s = FiberChords.sides(0, 0, 3000, 200, 3000, -200);
        assertNotEquals(s[0], s[1]);
        for (int side : s) assertTrue(FiberChords.step(side)[0] >= 0, "faces east-ish: " + side);
    }

    @Test
    void twoPanelRingKeepsNeighbouringChordsApart() {
        int n = 10;
        int[][] site = new int[n][];
        for (int i = 0; i < n; i++) {
            double a = 0.7 + i * Math.PI / 5;
            site[i] = new int[] {(int) Math.round(5000 * Math.cos(a)) >> 4 << 4, 80 + i % 3, (int) Math.round(5000 * Math.sin(a)) >> 4 << 4};
        }
        List<int[]> next = new ArrayList<>(), prev = new ArrayList<>();
        for (int i = 0; i < n; i++) {
            int[] p = site[(i + n - 1) % n], q = site[(i + 1) % n];
            int[] sides = FiberChords.sides(site[i][0], site[i][2], p[0], p[2], q[0], q[2]);
            int[] sp = FiberChords.step(sides[0]), sn = FiberChords.step(sides[1]);
            prev.add(new int[] {site[i][0] + sp[0], site[i][1] + 1, site[i][2] + sp[1]});
            next.add(new int[] {site[i][0] + sn[0], site[i][1] + 1, site[i][2] + sn[1]});
        }
        FiberChords ring = new FiberChords(next, prev);
        Set<Long> seen = new HashSet<>();
        for (int c = 0; c < n; c++) {
            int[][] path = ring.path(c);
            assertTrue(FiberLine.faceConnected(path));
            assertArrayEquals(next.get(c), path[0]);
            assertArrayEquals(prev.get((c + 1) % n), path[path.length - 1]);
            for (int[] b : path) assertTrue(seen.add(FiberChords.pack(b[0], b[1], b[2])), "chords share a block at " + Arrays.toString(b));
            // A chord never passes over its own mast top or the other panel's endpoint.
            int[] mast = site[c];
            for (int[] b : path) assertFalse(b[0] == mast[0] && b[2] == mast[2], "chord " + c + " crosses its mast");
            assertEquals(1 << c, ring.chordsAt(FiberChords.pack(path[0][0], path[0][1], path[0][2])));
            assertEquals(1, ring.arms(FiberChords.pack(path[0][0], path[0][1], path[0][2])) & 1, "endpoint joins its panel below");
        }
    }

    @Test
    void cableRouterKeepsRunsApart() {
        // A 5 x 1 x 5 room; two runs must cross it without touching; a third is forbidden a cell.
        java.util.function.LongPredicate free = p -> {
            int x = FiberChords.unpackX(p), y = FiberChords.unpackY(p), z = FiberChords.unpackZ(p);
            return x >= 0 && x < 5 && y >= 0 && y < 2 && z >= 0 && z < 5;
        };
        var runs = List.of(
                new CableRouter.Run("a", FiberChords.pack(0, 0, 0), FiberChords.pack(4, 0, 0), List.of(FiberChords.pack(5, 0, 0))),
                new CableRouter.Run("b", FiberChords.pack(0, 0, 4), FiberChords.pack(4, 0, 4), List.of()));
        var routed = CableRouter.routeAll(runs, free, Set.of(FiberChords.pack(2, 0, 0)), p -> 1);
        assertNotNull(routed);
        assertTrue(CableRouter.separate(routed));
        for (var r : routed) assertTrue(CableRouter.connected(r));
        assertFalse(routed.get(0).contains(FiberChords.pack(2, 0, 0)), "forbidden cell avoided");
        assertEquals(FiberChords.pack(5, 0, 0), routed.get(0).get(routed.get(0).size() - 1), "portal appended");
        // Control: runs that cannot be separated fail instead of touching.
        var tight = List.of(
                new CableRouter.Run("a", FiberChords.pack(0, 0, 0), FiberChords.pack(4, 0, 0), List.of()),
                new CableRouter.Run("b", FiberChords.pack(0, 0, 1), FiberChords.pack(4, 0, 1), List.of()));
        java.util.function.LongPredicate narrow = p -> FiberChords.unpackZ(p) <= 1 && FiberChords.unpackY(p) == 0
                && FiberChords.unpackX(p) >= 0 && FiberChords.unpackX(p) < 5;
        assertNull(CableRouter.routeAll(tight, narrow, Set.of(), p -> 1));
    }

    @Test
    void ringIndexAndCutRepairBookkeeping() {
        List<int[]> ends = new ArrayList<>();
        for (int i = 0; i < 10; i++) {
            double a = i * Math.PI / 5;
            ends.add(new int[] {(int) Math.round(5000 * Math.cos(a)) >> 4 << 4, 90 + i, (int) Math.round(5000 * Math.sin(a)) >> 4 << 4});
        }
        FiberChords ring = new FiberChords(ends);
        assertEquals(10, ring.chordCount());
        for (int c = 0; c < 10; c++) {
            int[][] path = ring.path(c);
            assertTrue(FiberLine.faceConnected(path), "chord " + c);
            assertArrayEquals(ends.get(c), path[0]);
            assertArrayEquals(ends.get((c + 1) % 10), path[path.length - 1]);
        }
        // Shared endpoint belongs to both adjacent chords.
        int[] e = ends.get(3);
        assertEquals((1 << 2) | (1 << 3), ring.chordsAt(FiberChords.pack(e[0], e[1], e[2])));
        // Every path block is listed exactly once in its chunk.
        int[] mid = ring.path(4)[ring.path(4).length / 2];
        long[] entries = ring.inChunk(mid[0] >> 4, mid[2] >> 4);
        assertTrue(Arrays.stream(entries).anyMatch(x -> (x >> 32) == 4 && ring.path(4)[(int) x][0] == mid[0] && ring.path(4)[(int) x][2] == mid[2]));
        assertEquals(0, ring.inChunk(0, 0).length, "spawn chunk is off the ring");

        // Canonical arms: exactly the directions to the two path neighbours; DOWN at endpoints.
        int[][] four = ring.path(4);
        int mi = four.length / 2;
        int expected = (1 << FiberChords.direction(four[mi - 1][0] - four[mi][0], four[mi - 1][1] - four[mi][1], four[mi - 1][2] - four[mi][2]))
                | (1 << FiberChords.direction(four[mi + 1][0] - four[mi][0], four[mi + 1][1] - four[mi][1], four[mi + 1][2] - four[mi][2]));
        assertEquals(expected, ring.arms(FiberChords.pack(four[mi][0], four[mi][1], four[mi][2])));
        assertEquals(2, Integer.bitCount(expected));
        int endArms = ring.arms(FiberChords.pack(e[0], e[1], e[2]));
        assertEquals(1, endArms & 1, "endpoint joins the patch panel below");
        assertEquals(0, ring.arms(FiberChords.pack(mid[0], mid[1] + 3, mid[2])));

        Set<Long> broken = new HashSet<>();
        long cut = FiberChords.pack(mid[0], mid[1], mid[2]);
        assertEquals(1 << 4, ring.chordsAt(cut));
        broken.add(cut);
        assertFalse(ring.intact(4, broken));
        assertTrue(ring.intact(3, broken), "control: an adjacent chord stays intact");
        assertTrue(ring.intact(5, broken));
        long offPath = FiberChords.pack(mid[0], mid[1] + 3, mid[2]);
        assertEquals(0, ring.chordsAt(offPath), "a block above the path is not fiber");
        broken.remove(cut);
        assertTrue(ring.intact(4, broken), "replacing the block repairs the chord");
    }
}
