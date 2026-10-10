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
