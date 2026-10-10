package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import java.util.*;

import static com.example.evanscomputermod.computer.SegmentGraph.*;
import static org.junit.jupiter.api.Assertions.*;

/** Cable segments: copper/fiber/panel rules, logical links, the ring's pieces and taps. */
class SegmentGraphTest {
    /** A tiny world: packed position -> block code, everything else empty. */
    private final Map<Long, Integer> world = new HashMap<>();
    private final List<Exit<String>> exits = new ArrayList<>();

    private static long p(int x, int y, int z) {
        return FiberChords.pack(x, y, z);
    }

    private void put(int code, int x, int y, int z) {
        world.put(p(x, y, z), code);
    }

    private void line(int code, int x0, int x1, int y, int z) {
        for (int x = x0; x <= x1; x++) put(code, x, y, z);
    }

    private void nic(String name, int x, int y, int z) {
        exits.add(new Exit<>(name, 0, p(x, y, z)));
    }

    private Result<String> run(Ring ring, List<Edge<String>> edges, List<StandIn<String>> standIns) {
        return SegmentGraph.compute(exits, (dim, pos) -> world.getOrDefault(pos, NONE), ring, edges, standIns);
    }

    private Result<String> run() {
        return run(null, List.of(), List.of());
    }

    private static boolean same(Result<String> r, String a, String b) {
        return r.segment.get(a) != null && r.segment.get(a).equals(r.segment.get(b));
    }

    @Test
    void copperJoinsCopperAndALoneNicHasNoCarrier() {
        line(CABLE, 0, 5, 0, 0);
        nic("a", 0, 0, 0);
        nic("b", 5, 0, 0);
        line(CABLE, 0, 3, 0, 9);
        nic("lone", 0, 0, 9);
        var r = run();
        assertTrue(same(r, "a", "b"));
        assertTrue(r.carrier("a") && r.carrier("b"));
        assertNotNull(r.segment.get("lone"), "cabled");
        assertFalse(r.carrier("lone"), "a cable that ends nowhere has no link partner");
        assertFalse(r.optical.contains(r.segment.get("a")));
    }

    @Test
    void cableSplitSeparatesSegments() {
        line(CABLE, 0, 5, 0, 0);
        nic("a", 0, 0, 0);
        nic("b", 5, 0, 0);
        world.remove(p(3, 0, 0));
        var r = run();
        assertFalse(same(r, "a", "b"));
        assertFalse(r.carrier("a"));
    }

    @Test
    void fiberJoinsOnlyFiberAndPanels() {
        // a - copper - panel - fiber - fiber - panel - copper - b
        put(CABLE, 0, 0, 0);
        put(PANEL, 1, 0, 0);
        line(FIBER, 2, 4, 0, 0);
        put(PANEL, 5, 0, 0);
        put(CABLE, 6, 0, 0);
        nic("a", 0, 0, 0);
        nic("b", 6, 0, 0);
        // Controls: copper touching the fiber, and a NIC face on bare fiber.
        put(CABLE, 3, 1, 0);
        nic("copper", 3, 1, 0);
        nic("bare", 4, -1, 0);
        put(FIBER, 4, -1, 0);
        var r = run();
        assertTrue(same(r, "a", "b"));
        assertTrue(r.optical.contains(r.segment.get("a")));
        assertFalse(same(r, "a", "copper"), "copper touching fiber is not connected");
        assertNull(r.segment.get("bare"), "a face on bare fiber is not cabled");
        assertFalse(r.carrier("copper"));
        world.remove(p(3, 0, 0));
        r = run();
        assertFalse(same(r, "a", "b"), "a broken span splits the segment");
        assertFalse(r.carrier("a") || r.carrier("b"), "no light from the far end");
    }

    @Test
    void gatewayAndLogicalLinksAndMerges() {
        line(CABLE, 0, 2, 0, 0);
        put(GATEWAY, 3, 0, 0);
        nic("pc", 0, 0, 0);
        line(CABLE, 0, 2, 0, 5);
        nic("x", 0, 0, 5);
        line(CABLE, 0, 2, 0, 9);
        nic("y", 0, 0, 9);
        var r = run(null, List.of(new Edge<>("x", "y", false), new Edge<>("u", "proxy", true)), List.of());
        assertTrue(r.internet.contains(r.segment.get("pc")));
        assertTrue(r.carrier("pc"), "the gateway is a link partner");
        assertTrue(same(r, "x", "y"), "logical link merges two cable segments");
        assertTrue(r.carrier("x"));
        assertTrue(r.internet.contains(r.segment.get("u")));
        assertTrue(same(r, "u", "proxy"));
    }

    // ---- the ring ----

    /** Two sites 120 blocks apart: chord 0 (0,10,0)->(120,10,0), chord 1 (120,10,6)->(0,10,6). */
    private static FiberChords ring(int length) {
        List<int[]> next = List.of(new int[] {0, 10, 0}, new int[] {length, 10, 6});
        List<int[]> prev = List.of(new int[] {0, 10, 6}, new int[] {length, 10, 0});
        return new FiberChords(next, prev);
    }

    /** Each ISP: router NIC on a riser under the panel below its chord end. */
    private void isp(String nic, int x, int z) {
        put(PANEL, x, 9, z);
        put(CABLE, x, 8, z);
        nic(nic, x, 8, z);
    }

    private static List<Long> panels(int length) {
        return List.of(p(0, 9, 0), p(length, 9, 0), p(length, 9, 6), p(0, 9, 6));
    }

    @Test
    void ringChordConductsBetweenPanelsWithoutReadingItsBlocks() {
        FiberChords chords = ring(120);
        isp("a3", 0, 0);
        isp("b2", 120, 0);
        var pieces = new RingPieces(chords, 0, Set.of(), Map.of(), panels(120));
        var r = run(pieces, List.of(), List.of());
        assertTrue(same(r, "a3", "b2"));
        assertTrue(r.carrier("a3") && r.carrier("b2"));
        assertTrue(r.optical.contains(r.segment.get("a3")));
        assertEquals(r.segment.get("a3"), r.pieceSegment[pieces.piece(0, 60)]);
        // The other chord exists but nobody is on it.
        assertEquals(-1, r.pieceSegment[pieces.piece(1, 60)]);
    }

    @Test
    void costDoesNotDependOnChordLength() {
        int[] reads = new int[2];
        int k = 0;
        for (int length : new int[] {120, 30000}) {
            world.clear();
            exits.clear();
            isp("a3", 0, 0);
            isp("b2", length, 0);
            var pieces = new RingPieces(ring(length), 0, Set.of(), Map.of(), panels(length));
            var r = run(pieces, List.of(), List.of());
            assertTrue(same(r, "a3", "b2"));
            reads[k++] = r.blocksVisited;
        }
        assertEquals(reads[0], reads[1], "blocks read must not grow with the chord");
        assertTrue(reads[1] < 40, "reads " + reads[1]);
    }

    @Test
    void breakSplitsThePieceAndTapsStayWithTheirSide() {
        FiberChords chords = ring(120);
        isp("a3", 0, 0);
        isp("b2", 120, 0);
        // A tap: a panel on top of path block 30, cabled to a player router.
        put(PANEL, 30, 11, 0);
        put(CABLE, 30, 12, 0);
        nic("tap", 30, 12, 0);
        List<Long> attached = new ArrayList<>(panels(120));
        attached.add(p(30, 11, 0));
        var whole = new RingPieces(chords, 0, Set.of(), Map.of(), attached);
        var r = run(whole, List.of(), List.of());
        assertTrue(same(r, "a3", "tap") && same(r, "tap", "b2"), "the tap shares the chord with both villages");
        assertEquals(3, r.members.get(r.segment.get("tap")).size());

        // Break between the tap and village B (block 50).
        var cut = new RingPieces(chords, 0, Set.of(p(50, 10, 0)), Map.of(), attached);
        assertEquals(2, cut.piecesOf(0).length);
        assertEquals(-1, cut.piece(0, 50));
        r = run(cut, List.of(), List.of());
        assertTrue(same(r, "a3", "tap"), "the tap keeps village A's side");
        assertFalse(same(r, "tap", "b2"));
        assertTrue(r.carrier("tap") && r.carrier("a3"));
        assertFalse(r.carrier("b2"), "village B is alone on its piece: no light");

        // Break between village A and the tap instead.
        cut = new RingPieces(chords, 0, Set.of(p(10, 10, 0)), Map.of(), attached);
        r = run(cut, List.of(), List.of());
        assertTrue(same(r, "tap", "b2"));
        assertFalse(same(r, "a3", "tap"));
    }

    @Test
    void controlsPanelOffThePathAndCopperOnThePath() {
        FiberChords chords = ring(120);
        isp("a3", 0, 0);
        isp("b2", 120, 0);
        // Panel two blocks above the path: not against it.
        put(PANEL, 40, 12, 0);
        put(CABLE, 40, 13, 0);
        nic("far", 40, 13, 0);
        // Copper cable directly on top of a path block.
        put(CABLE, 70, 11, 0);
        put(CABLE, 70, 12, 0);
        nic("copper", 70, 12, 0);
        List<Long> attached = new ArrayList<>(panels(120));
        attached.add(p(40, 12, 0)); // even if it were indexed, it touches no path block
        attached.add(p(70, 11, 0));
        var r = run(new RingPieces(chords, 0, Set.of(), Map.of(), attached), List.of(), List.of());
        assertTrue(same(r, "a3", "b2"));
        assertFalse(same(r, "far", "a3"), "a panel not against the path does not tap it");
        assertFalse(same(r, "copper", "a3"), "copper touching the ring is not connected");
        assertTrue(RingPieces.touchesPath(chords, p(70, 11, 0)));
        assertFalse(RingPieces.touchesPath(chords, p(40, 12, 0)));
    }

    @Test
    void adminCutAndEndpointBreakAndStandIns() {
        FiberChords chords = ring(120);
        isp("a3", 0, 0);
        var cut = new RingPieces(chords, 0, Set.of(), Map.of(0, new int[] {chords.midpoint(0)}), panels(120));
        // Village B never generated: its port stands in as plugged into the chord's last block.
        var standIn = List.of(new StandIn<>("b2", cut.piece(0, chords.path(0).length - 1)));
        var r = run(cut, List.of(), standIn);
        assertFalse(same(r, "a3", "b2"), "admin cut splits the chord");
        assertFalse(r.carrier("b2"));
        var whole = new RingPieces(chords, 0, Set.of(), Map.of(), panels(120));
        r = run(whole, List.of(), List.of(new StandIn<>("b2", whole.piece(0, chords.path(0).length - 1))));
        assertTrue(same(r, "a3", "b2"), "stand-in reaches across the intact chord");
        assertTrue(r.carrier("a3"));
        // Both ends stand in (nothing generated): still one segment.
        exits.clear();
        r = run(whole, List.of(), List.of(new StandIn<>("a3", whole.piece(0, 0)),
                new StandIn<>("b2", whole.piece(0, chords.path(0).length - 1))));
        assertTrue(same(r, "a3", "b2") && r.carrier("a3"));
        // The endpoint block itself removed: the panel no longer touches the piece.
        var noEnd = new RingPieces(chords, 0, Set.of(p(0, 10, 0)), Map.of(), panels(120));
        assertEquals(-1, noEnd.piece(0, 0));
        exits.clear();
        isp("a3", 0, 0);
        isp("b2", 120, 0);
        r = run(noEnd, List.of(), List.of());
        assertFalse(same(r, "a3", "b2"));
    }

    @Test
    void pieceNumberingAndRanges() {
        FiberChords chords = ring(120);
        int last = chords.path(0).length - 1;
        var r = new RingPieces(chords, 0, Set.of(p(10, 10, 0), p(11, 10, 0), p(0, 10, 0)), Map.of(0, new int[] {60}), List.of());
        int[] pieces = r.piecesOf(0);
        assertEquals(3, pieces.length); // [1..9], [12..59], [61..last]
        assertArrayEquals(new int[] {0, 1, 9}, r.range(pieces[0]));
        assertArrayEquals(new int[] {0, 12, 59}, r.range(pieces[1]));
        assertArrayEquals(new int[] {0, 61, last}, r.range(pieces[2]));
        assertEquals(pieces[1], r.piece(0, 30));
        assertEquals(pieces[2], r.piece(0, last));
        assertEquals(-1, r.piece(0, 11));
        assertEquals(1, r.piecesOf(1).length);
        assertEquals(4, r.pieceCount());
        assertArrayEquals(new int[] {pieces[1]}, r.piecesAt(p(30, 10, 0)));
        assertEquals(0, r.piecesAt(p(10, 10, 0)).length, "a break conducts nothing");
    }
}
