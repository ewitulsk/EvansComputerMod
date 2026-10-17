package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.computer.display.DisplayDevice;
import com.example.evanscomputermod.computer.display.PresentQueue;
import com.example.evanscomputermod.computer.pipeline.DisplayClock;
import com.example.evanscomputermod.computer.pipeline.DisplayPump;
import com.example.evanscomputermod.computer.pipeline.LossyCodec;
import com.example.evanscomputermod.computer.pipeline.ViewerRate;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.ClientPlayout;
import com.example.evanscomputermod.network.ClockSync;
import com.example.evanscomputermod.network.DeltaApplier;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The display pipeline's tile path (DISPLAY_PIPELINE_SPEC.md Parts A to E) without a game:
 * the pump's lanes, windowed flow control, frames a program queues for a time
 * (sent ahead, shown on the server at their time, played out by the client at
 * theirs), late joiners and flushes, lossy JPEG frames built once and refined
 * exact, and the client's playout buffer. Every packet goes through the real
 * codec; clients apply with the real {@link DeltaApplier} through
 * {@link ClientPlayout} and {@link ClientDisplayCache}.
 */
class DisplayPipelineTest {
    private static final DisplayDevice.RefreshLimits LIMITS = new DisplayDevice.RefreshLimits() {
        @Override public int defaultHz() { return 60; }
        @Override public int maxHz() { return 60; }
    };
    private static final int W = 128, H = 96;

    private final UUID computer = UUID.randomUUID();
    private final TerminalDisplay server = new TerminalDisplay(40, 10);
    private final AtomicInteger frames = new AtomicInteger();
    private final DisplayDevice dev = new DisplayDevice("test", () -> server, 640 * 400 * 4, LIMITS,
            frames::incrementAndGet, () -> {});
    private final Map<UUID, List<TerminalDeltaPacket>> got = new HashMap<>();

    @BeforeEach
    void clean() {
        ClientDisplayCache.clear();
        ClientPlayout.clear();
        ClientPlayout.fixedDelayMs = -1;
    }

    @AfterEach
    void reset() {
        ClientPlayout.fixedDelayMs = -1;
    }

    private DisplaySyncHub.Source source(int hint, boolean lossy) {
        return new DisplaySyncHub.Source() {
            @Override public TerminalDisplay display() { return server; }
            @Override public PresentQueue queue() { return dev.queue(); }
            @Override public int hint() { return hint; }
            @Override public boolean lossyAllowed() { return lossy; }
        };
    }

    /** One pass; every packet is serialised and parsed like the real one and given to that viewer's client. */
    private void sync(DisplaySyncHub hub, DisplaySyncHub.Source src, UUID... viewers) {
        got.clear();
        hub.sync(computer, src, List.of(viewers), (id, p) -> {
            got.computeIfAbsent(id, k -> new ArrayList<>()).add(p);
            receive(id, roundTrip(p));
        });
    }

    private static TerminalDeltaPacket roundTrip(TerminalDeltaPacket p) {
        ByteBuf buf = Unpooled.buffer();
        try {
            TerminalDeltaPacket.STREAM_CODEC.encode(buf, p);
            return TerminalDeltaPacket.STREAM_CODEC.decode(buf);
        } finally {
            buf.release();
        }
    }

    /** A client per viewer: its own display id (the cache is keyed by computer and display). */
    private final Map<UUID, Integer> clientDisplay = new HashMap<>();

    private int displayOf(UUID viewer) {
        return clientDisplay.computeIfAbsent(viewer, k -> 10 + clientDisplay.size());
    }

    private void receive(UUID viewer, TerminalDeltaPacket p) {
        TerminalDeltaPacket.ParsedDelta d = p.parse();
        if (d.isJpeg()) assertTrue(d.decodeJpeg(), "the JPEG decodes");
        ClientPlayout.receive(computer, displayOf(viewer), d, ClockSync.clientNowUs());
    }

    private TerminalDisplay client(UUID viewer) {
        return ClientDisplayCache.entry(computer, displayOf(viewer)).display;
    }

    private void ack(DisplaySyncHub hub, UUID viewer) {
        List<TerminalDeltaPacket> ps = got.get(viewer);
        assertNotNull(ps, "nothing was sent to " + viewer);
        TerminalDeltaPacket last = ps.get(ps.size() - 1);
        hub.onClientReady(viewer, last.parse().generation(), ClockSync.clientNowUs());
    }

    /** A frame of solid colour {@code c} (RGB565) into the back buffer. */
    private void draw(int pid, int c) {
        byte[] px = new byte[W * H * 2];
        for (int i = 0; i < W * H; i++) {
            px[i * 2] = (byte) c;
            px[i * 2 + 1] = (byte) (c >> 8);
        }
        assertEquals(DisplayDevice.OK, dev.blit(pid, 0, 0, W, H, px, TerminalDisplay.PIXEL_FORMAT_RGB565));
    }

    private static int pixel(TerminalDisplay d) {
        synchronized (d) {
            byte[] px = d.getPixelData();
            return (px[0] & 0xFF) | (px[1] & 0xFF) << 8;
        }
    }

    private static void sleep(long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException e) {
            throw new AssertionError(e);
        }
    }

    /** Wait until {@code cond} or fail after {@code ms}. */
    private static void await(String what, long ms, java.util.function.BooleanSupplier cond) {
        long end = System.nanoTime() + ms * 1_000_000L;
        while (!cond.getAsBoolean()) {
            if (System.nanoTime() > end) fail("timed out: " + what);
            sleep(5);
        }
    }

    // ------------------------------------------------------------ the pump

    @Test
    void aLaneRunsOnePassAtATimeAndFoldsRequestsIntoOneMore() throws Exception {
        AtomicInteger running = new AtomicInteger(), max = new AtomicInteger(), runs = new AtomicInteger();
        CountDownLatch first = new CountDownLatch(1), release = new CountDownLatch(1);
        DisplayPump.Lane lane = new DisplayPump.Lane("test", () -> {
            int n = running.incrementAndGet();
            max.accumulateAndGet(n, Math::max);
            runs.incrementAndGet();
            first.countDown();
            try {
                release.await(2, TimeUnit.SECONDS);
            } catch (InterruptedException ignored) {
            }
            running.decrementAndGet();
        });
        lane.request();
        assertTrue(first.await(2, TimeUnit.SECONDS));
        for (int i = 0; i < 50; i++) lane.request(); // while the pass runs
        release.countDown();
        await("the lane goes idle", 2000, () -> !lane.busy());
        assertEquals(1, max.get(), "never two passes at once");
        assertEquals(2, runs.get(), "fifty requests during a pass make exactly one more pass");
        assertTrue(DisplayPump.passes() >= 2);
    }

    // ------------------------------------------------------------ Part B: the window

    @Test
    void aViewerHasSeveralFramesInFlightAndATimeoutGivesItAKeyframe() {
        DisplaySyncHub hub = new DisplaySyncHub(0, 8, 512 * 1024);
        UUID a = UUID.randomUUID();
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, 0);
        sync(hub, DisplaySyncHub.exact(server), a);
        assertEquals(TerminalDeltaPacket.PACKET_TYPE_KEYFRAME, got.get(a).get(0).parse().packetType());
        // Nothing acked: frames still go out, up to the window.
        int sent = 0;
        for (int i = 0; i < 12; i++) {
            draw(1, 0x1000 + i);
            sync(hub, DisplaySyncHub.exact(server), a);
            if (got.get(a) != null) sent += got.get(a).size();
        }
        assertEquals(7, sent, "the keyframe plus 7 more fill a window of 8; then changes coalesce");
        assertNotEquals(pixel(server), pixel(client(a)), "the last frames waited");
        // A second without acks: the viewer is started over with a keyframe, not the stale chain.
        sleep(ClientSyncState.READY_TIMEOUT_MS + 100);
        sync(hub, DisplaySyncHub.exact(server), a);
        assertEquals(TerminalDeltaPacket.PACKET_TYPE_KEYFRAME, got.get(a).get(0).parse().packetType());
        assertEquals(pixel(server), pixel(client(a)));
        double[] stats = hub.viewerStats(a);
        assertNotNull(stats);
        assertEquals(1.0, stats[4], "one timeout counted");
    }

    @Test
    void acksFeedTheViewersRateEstimate() {
        ViewerRate r = new ViewerRate();
        long t = 1_000_000;
        for (int i = 1; i <= 20; i++) {
            r.sent(i, 10_000, t);
            r.acked(i, t + 40_000 + (i % 2) * 10_000, t + 80_000);
            t += 33_000;
        }
        assertEquals(80.0, r.rttMs(), 1.0, "the round trip");
        assertTrue(r.jitterMs() > 4 && r.jitterMs() < 11, "jitter from the receive times: " + r.jitterMs());
        assertTrue(r.deliveredKbps(t + 80_000) > 1000, "about 20 x 10 kB in the last second");
    }

    // ------------------------------------------------------------ Part C: timed frames

    @Test
    void queuedFramesGoOutAheadAndAreShownAtTheirTime() {
        DisplaySyncHub hub = new DisplaySyncHub(0, 8, 512 * 1024);
        UUID a = UUID.randomUUID();
        ClientPlayout.fixedDelayMs = 0; // the client shows each frame at its time (no jitter buffer, for the check)
        assertEquals(DisplayDevice.OK, dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, 0));
        assertEquals(DisplayDevice.E_SINGLE_BUFFERED, dev.presentAt(1, 0, DisplayClock.nowUs() + 100_000),
                "single-buffered displays can't queue");
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, DisplayDevice.FLAG_DOUBLE_BUFFER);
        draw(1, 0x0101);
        dev.present(1, 0);
        sync(hub, source(1, false), a);
        ack(hub, a);
        long t0 = DisplayClock.nowUs();
        for (int i = 0; i < 3; i++) {
            draw(1, 0x2000 + i);
            assertEquals(i + 1, dev.presentAt(1, 0, t0 + 250_000 + i * 50_000), "frames queued");
        }
        sync(hub, source(1, false), a);
        List<TerminalDeltaPacket> ps = got.get(a);
        assertEquals(3, ps.size(), "all three sent at once, ahead of their time");
        for (TerminalDeltaPacket p : ps) {
            assertTrue(p.timed(), "timed");
            assertTrue((p.flags() & TerminalDeltaPacket.FLAG_GFX_ONLY) != 0, "graphics only (text is never late)");
        }
        assertEquals(t0 + 250_000, ps.get(0).ptsUs(), 1000);
        assertEquals(0x0101, pixel(server), "the server still shows the old frame: nothing is due");
        assertEquals(3, ClientPlayout.queued(computer, displayOf(a)), "the client holds them");
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(0x0101, pixel(client(a)), "and doesn't show them early");
        await("the server shows the last frame at its time", 2000, () -> pixel(server) == 0x2002);
        long late = DisplayClock.nowUs() - (t0 + 350_000);
        assertTrue(late < 100_000, "shown on time on the server (" + late / 1000 + " ms late)");
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(0x2002, pixel(client(a)), "the client shows the newest due frame");
        assertEquals(0, ClientPlayout.queued(computer, displayOf(a)));
        assertEquals(3, dev.queue().framesQueued(), "three frames went through the queue");
    }

    @Test
    void aLateJoinerGetsWhatIsShownNowAndTheFramesStillQueued() {
        DisplaySyncHub hub = new DisplaySyncHub(0, 8, 512 * 1024);
        UUID a = UUID.randomUUID(), b = UUID.randomUUID();
        ClientPlayout.fixedDelayMs = 0;
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, DisplayDevice.FLAG_DOUBLE_BUFFER);
        draw(1, 0x0303);
        dev.present(1, 0);
        sync(hub, source(1, false), a);
        ack(hub, a);
        long t0 = DisplayClock.nowUs();
        draw(1, 0x4444);
        dev.presentAt(1, 0, t0 + 300_000);
        sync(hub, source(1, false), a);
        ack(hub, a);
        // B arrives while the frame waits: a keyframe of the screen now, then the queued frame.
        sync(hub, source(1, false), a, b);
        List<TerminalDeltaPacket> pb = got.get(b);
        assertEquals(2, pb.size());
        assertEquals(TerminalDeltaPacket.PACKET_TYPE_KEYFRAME, pb.get(0).parse().packetType());
        assertFalse(pb.get(0).timed(), "the keyframe is shown at once");
        assertTrue(pb.get(1).timed(), "the queued frame keeps its time");
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(0x0303, pixel(client(b)), "B shows the screen as it is now");
        await("the frame's time", 2000, () -> pixel(server) == 0x4444);
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(0x4444, pixel(client(a)));
        assertEquals(0x4444, pixel(client(b)));
        assertEquals(1, hub.timedBuilds(), "one build for both viewers");
    }

    @Test
    void aFlushDropsTheQueueOnTheServerAndTheClient() {
        DisplaySyncHub hub = new DisplaySyncHub(0, 8, 512 * 1024);
        UUID a = UUID.randomUUID();
        ClientPlayout.fixedDelayMs = 0;
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, DisplayDevice.FLAG_DOUBLE_BUFFER);
        draw(1, 0x0505);
        dev.present(1, 0);
        sync(hub, source(1, false), a);
        ack(hub, a);
        long t0 = DisplayClock.nowUs();
        for (int i = 0; i < 4; i++) {
            draw(1, 0x6000 + i);
            dev.presentAt(1, 0, t0 + 200_000 + i * 40_000);
        }
        sync(hub, source(1, false), a);
        ack(hub, a);
        assertEquals(4, ClientPlayout.queued(computer, displayOf(a)));
        // Seek: the program flushes and shows a new frame now.
        assertEquals(4, dev.queueFlush(1));
        draw(1, 0x7777);
        dev.present(1, 0);
        sync(hub, source(1, false), a);
        List<TerminalDeltaPacket> ps = got.get(a);
        assertTrue(ps.get(0).flush(), "a flush marker first");
        assertEquals(1, hub.flushMarkers());
        assertEquals(0, ClientPlayout.queued(computer, displayOf(a)), "the client dropped the frames it held");
        assertEquals(0x7777, pixel(client(a)), "the frame after the seek is shown at once");
        sleep(350);
        dev.applyDue();
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(0x7777, pixel(server), "no stale frame appears on the server after the flush");
        assertEquals(0x7777, pixel(client(a)), "nor on the client");
    }

    @Test
    void presentAtRespectsTheQueueLimits() {
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, DisplayDevice.FLAG_DOUBLE_BUFFER);
        long t0 = DisplayClock.nowUs();
        for (int i = 0; i < 12; i++) assertEquals(i + 1, dev.presentAt(1, 0, t0 + 400_000));
        assertEquals(DisplayDevice.E_AGAIN, dev.presentAt(1, 0, t0 + 400_000), "13th frame: the queue is full");
        assertEquals(12, dev.queueFlush(1), "the flush drops all twelve");
        assertEquals(DisplayDevice.E_AGAIN, dev.presentAt(1, 0, DisplayClock.nowUs() + 900_000), "more than 500 ms ahead");
        long state = dev.queueState();
        assertEquals(0, state >>> 32, "flushed");
        // With the wait flag the call waits for the time to come within reach instead.
        long before = System.nanoTime();
        assertTrue(dev.presentAt(1, DisplayDevice.PRESENT_AT_WAIT, DisplayClock.nowUs() + 600_000) > 0);
        assertTrue(System.nanoTime() - before > 50_000_000L, "it waited until 500 ms ahead");
        assertEquals(DisplayDevice.OK, dev.setHint(1, DisplayDevice.HINT_MOTION));
        assertEquals(DisplayDevice.E_INVALID, dev.setHint(1, 7));
    }

    // ------------------------------------------------------------ Part D: lossy frames

    /** A frame where every pixel differs from the last (moving noise), RGB565. */
    private void drawNoise(int pid, int seed) {
        byte[] px = new byte[W * H * 2];
        java.util.Random r = new java.util.Random(seed);
        for (int i = 0; i < W * H; i++) {
            int x = i % W, y = i / W;
            int v = ((x + seed) * 7 & 0x1F) << 11 | ((y * 3 + seed) & 0x3F) << 5 | (r.nextInt(4) + 8);
            px[i * 2] = (byte) v;
            px[i * 2 + 1] = (byte) (v >> 8);
        }
        assertEquals(DisplayDevice.OK, dev.blit(pid, 0, 0, W, H, px, TerminalDisplay.PIXEL_FORMAT_RGB565));
    }

    @Test
    void movingPicturesGoAsJpegBuiltOnceAndStillPicturesAreRefinedExact() {
        DisplaySyncHub hub = new DisplaySyncHub(0, 8, 512 * 1024);
        UUID a = UUID.randomUUID(), b = UUID.randomUUID();
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, DisplayDevice.FLAG_DOUBLE_BUFFER);
        dev.setHint(1, DisplayDevice.HINT_MOTION);
        drawNoise(1, 0);
        dev.present(1, 0);
        sync(hub, source(DisplayDevice.HINT_MOTION, true), a, b);
        ack(hub, a);
        ack(hub, b);
        for (int i = 1; i <= 5; i++) {
            drawNoise(1, i);
            dev.present(1, 0);
            sync(hub, source(DisplayDevice.HINT_MOTION, true), a, b);
            assertTrue(got.get(a).get(0).jpeg(), "frame " + i + " is a JPEG");
            assertSame(got.get(a).get(0), got.get(b).get(0), "the same JPEG goes to both viewers");
            ack(hub, a);
            ack(hub, b);
        }
        assertEquals(5, hub.jpegBuilds(), "one encode per frame, not per viewer");
        assertTrue(hub.lossyTiles() > 0);
        byte[] exact = server.getPixelData().clone();
        assertFalse(java.util.Arrays.equals(exact, client(a).getPixelData()), "the client holds an approximation");
        // The picture stands still for 250 ms: one exact frame follows.
        sleep(DisplaySyncHub.REFINE_AFTER_US / 1000 + 50);
        sync(hub, source(DisplayDevice.HINT_MOTION, true), a, b);
        assertNotNull(got.get(a), "a refine was sent");
        assertFalse(got.get(a).get(0).jpeg(), "the refine is exact");
        assertEquals(1, hub.refines());
        assertEquals(0, hub.lossyTiles());
        assertArrayEquals(exact, client(a).getPixelData(), "the paused picture is exact on the client");
        assertArrayEquals(exact, client(b).getPixelData());
        // Hint exact: never lossy.
        drawNoise(1, 9);
        dev.present(1, 0);
        ack(hub, a);
        sync(hub, source(DisplayDevice.HINT_EXACT, true), a);
        assertFalse(got.get(a).get(0).jpeg(), "exact hint: exact tiles");
        // Tier 1 (lossy not allowed by the source): exact too.
        drawNoise(1, 10);
        dev.present(1, 0);
        ack(hub, a);
        sync(hub, source(DisplayDevice.HINT_MOTION, false), a);
        assertFalse(got.get(a).get(0).jpeg(), "a source that doesn't allow lossy stays exact");
    }

    @Test
    void autoModeGoesLossyOnlyForPicturesThatKeepChanging() {
        DisplaySyncHub hub = new DisplaySyncHub(0, 64, 64L << 20);
        UUID a = UUID.randomUUID();
        dev.init(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGB565, DisplayDevice.FLAG_DOUBLE_BUFFER);
        drawNoise(1, 0);
        dev.present(1, 0);
        sync(hub, source(DisplayDevice.HINT_AUTO, true), a);
        int jpegs = 0;
        for (int i = 1; i <= 12; i++) {
            ack(hub, a);
            drawNoise(1, i);
            dev.present(1, 0);
            sync(hub, source(DisplayDevice.HINT_AUTO, true), a);
            if (got.get(a).get(0).jpeg()) jpegs++;
            else assertTrue(i < 8, "frame " + i + " should have gone lossy by now");
        }
        assertTrue(jpegs >= 4, "after 8 heavy frames the stream is lossy (" + jpegs + ")");
        // A small change (a cursor blinking) stays exact even in lossy mode.
        ack(hub, a);
        byte[] one = new byte[2];
        dev.blit(1, 3, 3, 1, 1, one, TerminalDisplay.PIXEL_FORMAT_RGB565);
        dev.present(1, 0);
        sync(hub, source(DisplayDevice.HINT_AUTO, true), a);
        assertFalse(got.get(a).get(0).jpeg(), "a one-tile change is exact");
    }

    @Test
    void jpegFramesRoundTripCloseToTheOriginalAtEveryRung() throws Exception {
        LossyCodec c = new LossyCodec();
        byte[] px = new byte[W * H * 4];
        for (int i = 0; i < W * H; i++) {
            px[i * 4] = (byte) (i % W * 4);
            px[i * 4 + 1] = (byte) (i / W * 5);
            px[i * 4 + 2] = (byte) 128;
            px[i * 4 + 3] = (byte) 255;
        }
        int[] size = new int[LossyCodec.RUNGS.length];
        for (int r = 0; r < LossyCodec.RUNGS.length; r++) {
            byte[] j = c.encode(px, W, H, TerminalDisplay.PIXEL_FORMAT_RGBA8888, r);
            size[r] = j.length;
            byte[] out = new byte[px.length];
            assertTrue(c.decode(j, W, H, TerminalDisplay.PIXEL_FORMAT_RGBA8888, out), "rung " + r + " decodes");
            long err = 0;
            for (int i = 0; i < px.length; i++) if (i % 4 != 3) err += Math.abs((px[i] & 0xFF) - (out[i] & 0xFF));
            double mean = err / (W * H * 3.0);
            assertTrue(mean < (r == LossyCodec.HALF_RES_RUNG ? 12 : 6), "rung " + r + ": mean error " + mean);
        }
        assertTrue(size[0] >= size[3], "the lowest quality is no bigger than the best: " + java.util.Arrays.toString(size));
        assertTrue(size[4] < size[3], "half resolution is smaller still: " + java.util.Arrays.toString(size));
        assertFalse(LossyCodec.supports(TerminalDisplay.PIXEL_FORMAT_INDEXED8), "palette displays stay exact");
    }

    // ------------------------------------------------------------ Part E1: the playout buffer

    @Test
    void untimedUpdatesAreNeverDelayedAndTimedOnesWaitForTheirSlot() {
        UUID id = UUID.randomUUID();
        TerminalDisplay src = new TerminalDisplay(10, 4);
        src.setGfxFrame(1, 32, 16, TerminalDisplay.PIXEL_FORMAT_RGBA8888, null, new byte[32 * 16 * 4]);
        java.util.zip.Deflater z = new java.util.zip.Deflater();
        ClientPlayout.receive(id, 0, TerminalDeltaPacket.createKeyframe(id, (byte) 0, src, 1, z).parse(), ClockSync.clientNowUs());
        TerminalDisplay cl = ClientDisplayCache.entry(id, 0).display;
        assertEquals(32, cl.getGfxWidth(), "a keyframe is applied at once");
        // A timed frame due in 200 ms (server clock = client clock here: the same JVM).
        long now = DisplayClock.nowUs();
        FramebufferDiffTracker t = new FramebufferDiffTracker();
        t.initShadow(src);
        byte[] px = new byte[32 * 16 * 4];
        java.util.Arrays.fill(px, (byte) 9);
        var gfx = t.computeGfxDelta(32, 16, px, src.getPalette(), 1, TerminalDisplay.PIXEL_FORMAT_RGBA8888);
        var timed = TerminalDeltaPacket.createDelta(id, (byte) 0, 2, 1, null, gfx, 10, TerminalDeltaPacket.FLAG_GFX_ONLY, z)
                .withTime(now + 200_000, now);
        ClientPlayout.fixedDelayMs = 0;
        ClientPlayout.receive(id, 0, timed.parse(), ClockSync.clientNowUs());
        // Text typed meanwhile: shown at once, not behind the frame.
        src.getCellData()[0] = 'x';
        var text = TerminalDeltaPacket.createDelta(id, (byte) 0, 3, 1, t.computeTextDelta(src), null, 10, z);
        ClientPlayout.receive(id, 0, text.parse(), ClockSync.clientNowUs());
        assertEquals('x', cl.getCellData()[0], "untimed text is applied at once");
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(0, cl.getPixelData()[0], "the frame waits for its time");
        sleep(250);
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(9, cl.getPixelData()[0], "and is shown then");
        assertEquals('x', cl.getCellData()[0], "the text stays (a timed frame carries no text)");
        assertEquals(TerminalDisplay.PIXEL_FORMAT_RGBA8888, cl.getPixelFormat(), "a text-only packet keeps the format");
    }

    @Test
    void lateFramesShowAtOnceUnlessANewerOneIsDueAndTheDelayAdapts() {
        UUID id = UUID.randomUUID();
        var c = ClientPlayout.computer(id);
        TerminalDisplay src = new TerminalDisplay(10, 4);
        src.setGfxFrame(1, 16, 16, TerminalDisplay.PIXEL_FORMAT_RGBA8888, null, new byte[16 * 16 * 4]);
        java.util.zip.Deflater z = new java.util.zip.Deflater();
        ClientPlayout.receive(id, 0, TerminalDeltaPacket.createKeyframe(id, (byte) 0, src, 1, z).parse(), ClockSync.clientNowUs());
        FramebufferDiffTracker t = new FramebufferDiffTracker();
        t.initShadow(src);
        long now = DisplayClock.nowUs();
        // Two frames whose times have passed (they arrived late): only the newer is shown.
        // (A fixed delay first, so the second frame's slot isn't moved by the first one's lateness.)
        ClientPlayout.fixedDelayMs = 0;
        for (int i = 0; i < 2; i++) {
            byte[] px = new byte[16 * 16 * 4];
            java.util.Arrays.fill(px, (byte) (20 + i));
            var gfx = t.computeGfxDelta(16, 16, px, src.getPalette(), 1, TerminalDisplay.PIXEL_FORMAT_RGBA8888);
            t.commitShadowGfx(16, 16, px, src.getPalette(), 1, TerminalDisplay.PIXEL_FORMAT_RGBA8888);
            var p = TerminalDeltaPacket.createDelta(id, (byte) 0, 2 + i, 1, null, gfx, 10, TerminalDeltaPacket.FLAG_GFX_ONLY, z)
                    .withTime(now - 300_000 + i * 10_000, now); // sent now for a time already gone: late
            ClientPlayout.receive(id, 0, p.parse(), ClockSync.clientNowUs());
        }
        ClientPlayout.tick(ClockSync.clientNowUs());
        assertEquals(21, ClientDisplayCache.entry(id, 0).display.getPixelData()[0], "the newest due frame is shown");
        long[] k = c.counters();
        assertEquals(2, k[2], "both arrived late");
        assertEquals(1, k[3], "the older one was dropped (never on screen)");
        // Adaptive: a late frame raises the delay at once, by its lateness.
        ClientPlayout.fixedDelayMs = -1;
        double before = c.delayMs();
        byte[] px = new byte[16 * 16 * 4];
        java.util.Arrays.fill(px, (byte) 30);
        var gfx = t.computeGfxDelta(16, 16, px, src.getPalette(), 1, TerminalDisplay.PIXEL_FORMAT_RGBA8888);
        var p = TerminalDeltaPacket.createDelta(id, (byte) 0, 4, 1, null, gfx, 10, TerminalDeltaPacket.FLAG_GFX_ONLY, z)
                .withTime(now - 150_000, now);
        ClientPlayout.receive(id, 0, p.parse(), ClockSync.clientNowUs());
        assertTrue(c.delayMs() > before + 100, "lateness raises the playout delay: " + before + " -> " + c.delayMs());
        assertTrue(c.delayMs() <= ClientPlayout.maxPlayoutMs, "within the server's cap");
        assertTrue(ClientPlayout.audioDelayUs(id, ClockSync.clientNowUs()) >= 0, "the computer streams: its audio follows the delay");
        assertEquals(-1, ClientPlayout.audioDelayUs(UUID.randomUUID(), ClockSync.clientNowUs()), "others keep low-latency audio");
    }

    @Test
    void theClockEstimateFollowsTheLeastDelayedMessage() {
        ClockSync s = new ClockSync();
        // Server clock is 5 s ahead; network delay 20..80 ms.
        for (int i = 0; i < 50; i++) {
            long recv = 1_000_000L + i * 33_000L;
            long delay = 20_000 + (i * 7919 % 60_000);
            s.note(recv - delay + 5_000_000L, recv);
        }
        assertEquals(5_000_000L - 20_000L, s.offsetUs(), 5_000, "offset = clock difference minus the least delay");
        assertEquals(1_000_000L, s.toClient(6_000_000L - 20_000L), 5_000);
    }
}
