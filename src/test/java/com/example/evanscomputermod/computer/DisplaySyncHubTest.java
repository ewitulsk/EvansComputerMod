package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.network.DeltaApplier;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

import static org.junit.jupiter.api.Assertions.*;

/**
 * One display streamed to several viewers: each frame's delta is built once
 * and shared, viewers that fell behind catch up from the history, and every
 * client ends up showing exactly what the server shows.
 */
class DisplaySyncHubTest {

    private static final UUID COMPUTER = UUID.randomUUID();
    private static final UUID A = UUID.randomUUID();
    private static final UUID B = UUID.randomUUID();

    private final TerminalDisplay server = new TerminalDisplay(40, 10);
    private DisplaySyncHub hub = new DisplaySyncHub(1);
    /** Each viewer's client display and the packets it got in the last pass. */
    private final Map<UUID, TerminalDisplay> clients = new HashMap<>();
    private final Map<UUID, List<TerminalDeltaPacket>> got = new HashMap<>();

    DisplaySyncHubTest() {
        byte[] pixels = new byte[64 * 32];
        int[] palette = new int[256];
        for (int i = 0; i < 256; i++) palette[i] = 0xFF000000 | i * 0x010101;
        server.setGfxFrame(2, 64, 32, TerminalDisplay.PIXEL_FORMAT_INDEXED8, palette, pixels);
    }

    /** One sync pass for {@code viewers}; packets are applied to their clients but not acked. */
    private void sync(UUID... viewers) {
        got.clear();
        hub.sync(COMPUTER, server, List.of(viewers), (id, p) -> {
            assertEquals(COMPUTER, p.computerId());
            assertEquals(1, p.displayId());
            got.computeIfAbsent(id, k -> new ArrayList<>()).add(p);
            TerminalDeltaPacket.ParsedDelta d = p.parse();
            TerminalDisplay c = clients.computeIfAbsent(id, k -> new TerminalDisplay(1, 1));
            if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) DeltaApplier.applyKeyframe(c, d);
            else DeltaApplier.applyDelta(c, d);
        });
    }

    /** The viewer acks the last packet it got. */
    private void ack(UUID viewer) {
        List<TerminalDeltaPacket> ps = got.get(viewer);
        assertNotNull(ps, "nothing was sent to " + viewer);
        hub.onClientReady(viewer, ps.get(ps.size() - 1).parse().generation());
    }

    private void type(int col, int row, char c) {
        server.getCellData()[(row * server.getWidth() + col) * TerminalDisplay.CELL_SIZE] = (byte) c;
    }

    private void draw(int x, int y, int colour) {
        byte[] px = server.getPixelData().clone();
        px[y * 64 + x] = (byte) colour;
        server.setGfxFrame(2, 64, 32, TerminalDisplay.PIXEL_FORMAT_INDEXED8, null, px);
    }

    private void assertShows(UUID viewer) {
        TerminalDisplay c = clients.get(viewer);
        assertArrayEquals(server.getCellData(), c.getCellData(), "text of " + viewer);
        assertArrayEquals(server.getPixelData(), c.getPixelData(), "pixels of " + viewer);
        assertEquals(server.getCursorX(), c.getCursorX());
    }

    @Test
    void twoViewersCauseOneDeltaBuildPerFrame() {
        sync(A, B);
        assertEquals(1, hub.keyframeBuilds(), "one keyframe for both");
        assertSame(got.get(A).get(0), got.get(B).get(0));
        ack(A);
        ack(B);

        type(3, 2, 'X');
        draw(40, 20, 7);
        server.setCursorX(4);
        sync(A, B);
        assertEquals(1, hub.deltaBuilds(), "two viewers, one delta build");
        assertEquals(1, got.get(A).size());
        assertSame(got.get(A).get(0), got.get(B).get(0), "the same packet goes to both");
        assertEquals(TerminalDeltaPacket.PACKET_TYPE_DELTA, got.get(A).get(0).parse().packetType());
        assertShows(A);
        assertShows(B);

        // Nothing changed: nothing is built or sent.
        ack(A);
        ack(B);
        sync(A, B);
        assertEquals(1, hub.deltaBuilds());
        assertTrue(got.isEmpty());
    }

    /** With a window of one frame (the old flow control), a viewer still waiting for its ack falls behind. */
    @Test
    void aViewerThatFellBehindCatchesUpFromTheHistory() {
        hub = new DisplaySyncHub(1, 1, 1L << 20);
        sync(A, B);
        ack(A); // B's ack for the keyframe is still on its way

        type(0, 0, 'a');
        sync(A, B);
        assertEquals(1, hub.deltaBuilds());
        assertNull(got.get(B), "B is still waiting to ack");
        ack(A);
        hub.onClientReady(B, 1); // B's keyframe ack arrives

        type(1, 0, 'b');
        draw(1, 1, 9);
        sync(A, B);
        assertEquals(2, hub.deltaBuilds(), "still one build per frame");
        assertEquals(1, got.get(A).size());
        assertEquals(1, got.get(B).size(), "B gets the frame it missed (one frame in its window)");
        TerminalDeltaPacket missed = got.get(B).get(0);
        TerminalDeltaPacket latest = got.get(A).get(0);
        ack(A);
        ack(B);
        sync(A, B);
        assertEquals(1, got.get(B).size(), "then this one, from the history");
        assertSame(latest, got.get(B).get(0), "the same packet A got");
        assertNotSame(missed, latest);
        assertEquals(1, hub.keyframeBuilds(), "no keyframe needed to catch up");
        assertShows(A);
        assertShows(B);
    }

    @Test
    void aViewerBeyondTheHistoryGetsAKeyframe() {
        hub = new DisplaySyncHub(1, 1, 1L << 20);
        sync(A, B);
        ack(A);
        for (int i = 0; i < DisplaySyncHub.HISTORY_MAX + 2; i++) {
            type(i % 40, 1, (char) ('a' + i % 26));
            sync(A, B);
            ack(A);
        }
        hub.onClientReady(B, 1);
        type(0, 9, 'z');
        sync(A, B);
        assertEquals(1, got.get(B).size());
        assertEquals(TerminalDeltaPacket.PACKET_TYPE_KEYFRAME, got.get(B).get(0).parse().packetType());
        assertShows(A);
        assertShows(B);
    }

    @Test
    void aChangeOfShapeIsAKeyframeStep() {
        sync(A);
        ack(A);
        server.setGfxFrame(2, 32, 16, TerminalDisplay.PIXEL_FORMAT_RGBA8888, null, new byte[32 * 16 * 4]);
        sync(A);
        assertEquals(TerminalDeltaPacket.PACKET_TYPE_KEYFRAME, got.get(A).get(0).parse().packetType());
        assertEquals(32, clients.get(A).getGfxWidth());
        assertEquals(TerminalDisplay.PIXEL_FORMAT_RGBA8888, clients.get(A).getPixelFormat());
    }

    /**
     * A rectangle write ({@code setGfxRect}: compositor damage, single-buffered
     * blits) is diffed by the tiles it touched only, and clients still match.
     * A second hub on the same display (another view of it) keeps its own
     * place in the change history.
     */
    @Test
    void rectangleWritesAreDiffedByTheirTilesOnly() {
        sync(A);
        ack(A);
        byte[] frame = server.getPixelData().clone();
        frame[5 * 64 + 40] = 9;
        frame[6 * 64 + 41] = 9;
        assertTrue(server.setGfxRect(2, 64, 32, TerminalDisplay.PIXEL_FORMAT_INDEXED8, frame, 40, 5, 2, 2));
        sync(A);
        assertEquals(1, hub.lastTilesCompared(), "one 16x16 tile compared out of 8");
        assertShows(A);
        ack(A);
        DisplaySyncHub other = new DisplaySyncHub(1);
        other.sync(COMPUTER, server, List.of(B), (id, p) -> {});
        frame[20 * 64 + 3] = 4;
        assertTrue(server.setGfxRect(2, 64, 32, TerminalDisplay.PIXEL_FORMAT_INDEXED8, frame, 3, 20, 1, 1));
        sync(A);
        assertEquals(1, hub.lastTilesCompared());
        assertShows(A);
        // A whole-frame write still compares every tile.
        ack(A);
        draw(1, 1, 7);
        sync(A);
        assertEquals(-1, hub.lastTilesCompared(), "a full frame is compared in full");
        assertShows(A);
    }
}
