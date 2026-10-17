package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.computer.display.PresentQueue;
import com.example.evanscomputermod.computer.pipeline.DisplayClock;
import com.example.evanscomputermod.computer.pipeline.LossyCodec;
import com.example.evanscomputermod.computer.pipeline.StreamBudget;
import com.example.evanscomputermod.network.TerminalDeltaPacket;

import org.jetbrains.annotations.Nullable;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.Collection;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.BiConsumer;
import java.util.function.LongConsumer;
import java.util.zip.Deflater;

/**
 * Streams one display to every player watching it, building each frame's
 * packet once (docs/gui/DISPLAY_PIPELINE.md "Pipeline today").
 *
 * <p>The hub keeps one shadow of the display (the <i>head</i>, at generation
 * {@code headGen}). A sync pass diffs the display against the head once,
 * compresses the result once and commits it as the next generation; every
 * viewer whose last frame is the previous head gets that same packet. A
 * viewer that fell behind is sent the packets it missed, in order, from a
 * short history, and only gets a keyframe when the history no longer
 * reaches back to its frame. Keyframes are built at most once per pass and
 * shared too.
 *
 * <p><b>Flow control</b> (DISPLAY_PIPELINE_SPEC.md Part B1) is a window per
 * viewer ({@link ClientSyncState}): up to {@code display.windowFrames}
 * frames and {@code display.windowBytes} unacknowledged; past that changes
 * coalesce. A frame unacked for a second restarts the viewer with a keyframe.
 * Acks carry the client's receive time, which feeds each viewer's
 * {@link com.example.evanscomputermod.computer.pipeline.ViewerRate}.
 *
 * <p><b>Timed frames</b> (Part C4): when the display's {@link PresentQueue}
 * holds frames a program queued with {@code gfx_present_at}, every new one is
 * diffed against the previous frame in presentation order and sent right
 * away, graphics only, tagged with its time, so clients buffer ahead. While
 * frames the server hasn't shown yet are out, the untimed passes send text
 * only (the graphics plane belongs to the queue). A late joiner gets a
 * keyframe of what the display shows now plus the frames still queued. A
 * flush ({@code gfx_queue_flush}) sends a flush marker so clients drop what
 * they hold beyond the frame shown last.
 *
 * <p><b>Lossy frames</b> (Part D): with a {@link Source} that allows it, a
 * graphics plane that keeps changing (hint motion, or auto: at least half the
 * tiles changed in 8 of the last 10 frames) goes as one JPEG, built once and
 * shared. The quality rung fits the slowest viewer's share of
 * {@code display.maxStreamKbps}; a viewer that still can't keep up gets
 * every 2nd or 3rd frame. When a lossy picture stops changing for 250 ms,
 * the tiles it covered go out exact once ("refine").
 *
 * <p>Diffing and encoding hold the display's monitor (programs and the
 * kernel write it under the same monitor); packets go to {@code send} after
 * it is released. The methods are synchronized; passes normally run on the
 * display pump.
 */
public final class DisplaySyncHub {

    /** Most packets kept for viewers that fell behind. */
    static final int HISTORY_MAX = 32;
    /** Most compressed bytes kept for them (a keyframe is cheaper beyond this). */
    static final long HISTORY_MAX_BYTES = 4L << 20;
    /** Auto mode: frames looked back on, and how many must change half the tiles to go lossy. */
    static final int AUTO_LOOKBACK = 10, AUTO_LOSSY = 8;
    /** Auto mode goes back to exact after this long without a heavy change (µs). */
    static final long AUTO_EXIT_US = 1_000_000;
    /** A lossy picture that stops changing this long is sent exact once (µs). */
    static final long REFINE_AFTER_US = 250_000;

    /** What a pass streams: the display, its presentation queue, and whether lossy frames are allowed. */
    public interface Source {
        TerminalDisplay display();

        /** The display's presentation queue (null: none). */
        @Nullable
        default PresentQueue queue() {
            return null;
        }

        /** {@code gfx_set_hint}: 0 auto, 1 exact, 2 motion. */
        default int hint() {
            return 1;
        }

        /** May this display go lossy at all (server config, not a Tier 1 panel)? */
        default boolean lossyAllowed() {
            return false;
        }
    }

    /** An exact-only source for a display (tests, the mirrors). */
    public static Source exact(TerminalDisplay d) {
        return () -> d;
    }

    private final byte displayId;
    private final int windowFrames;
    private final long windowBytes;
    private final FramebufferDiffTracker head = new FramebufferDiffTracker();
    private final Deflater deflater = new Deflater(Deflater.BEST_SPEED);
    private final Map<UUID, ClientSyncState> viewers = new HashMap<>();
    private final ArrayDeque<Step> history = new ArrayDeque<>();
    private long historyBytes;
    /** The display the head shadows; a different one restarts the stream with a keyframe. */
    @Nullable
    private TerminalDisplay bound;
    private boolean started;
    private long headGen;
    /** The display's sync version the head shadows (see TerminalDisplay#changedTilesSince). */
    private int headVersion;
    private int lastTilesCompared;
    private long deltaBuilds;
    private long keyframeBuilds;
    private long packetsSent;
    /** Bytes handed to viewers (see {@link #PACKET_OVERHEAD}), in total and per viewer. */
    private long bytesSent;
    private final Map<UUID, Long> viewerBytes = new HashMap<>();
    /**
     * Bytes counted per packet on top of its payload, what the game's custom
     * payload packet adds: the packet id, the payload type
     * ({@code evanscomputermod:terminal_delta}), the computer UUID, the
     * display id, the flags and the length prefix.
     */
    public static final int PACKET_OVERHEAD = 53;

    // Timed frames.
    /** Last queued frame this hub built a step for, and the queue's flush count it saw. */
    private long timedSeq;
    private int flushEpoch;
    /** Steps of frames still queued on the server (sent to a late joiner after its keyframe). */
    private final ArrayDeque<Step> timedPending = new ArrayDeque<>();
    private long timedBuilds, flushMarkers;

    // Lossy frames.
    @Nullable
    private LossyCodec codec;
    private final boolean[] heavy = new boolean[AUTO_LOOKBACK];
    private int heavyAt;
    private long lastHeavyUs = Long.MIN_VALUE / 2;
    /** Tiles clients hold lossy, and when the last lossy frame went out. */
    private final BitSet lossyTiles = new BitSet();
    private long lastLossyUs;
    private long lastGfxFrameUs;
    private double frameIntervalUs = 33_333;
    /** Smoothed JPEG bytes per rung (0 = not measured yet). */
    private final double[] rungBytes = new double[LossyCodec.RUNGS.length];
    private int rung;
    private long jpegBuilds, refines, framesSkipped;
    /** Asks the host for another pass in so many ns (refine timers). */
    @Nullable
    private LongConsumer waker;

    /** One generation step of the stream: the packet that takes a client from {@code from} to {@code to}. */
    private record Step(long from, long to, TerminalDeltaPacket packet, boolean keyframe, long seq, boolean wholeLossy) {}

    public DisplaySyncHub(int displayId) {
        this(displayId, com.example.evanscomputermod.EcmConfig.displayWindowFrames(),
                com.example.evanscomputermod.EcmConfig.displayWindowBytes());
    }

    /** A hub with its own window (tests). */
    public DisplaySyncHub(int displayId, int windowFrames, long windowBytes) {
        this.displayId = (byte) displayId;
        this.windowFrames = Math.max(1, windowFrames);
        this.windowBytes = Math.max(1, windowBytes);
    }

    /** Where the hub asks for a pass later (the host's lane timer). */
    public synchronized void setWaker(@Nullable LongConsumer waker) {
        this.waker = waker;
    }

    /** {@link #sync(UUID, Source, Collection, BiConsumer)} of an exact display with no queue. */
    public void sync(UUID computerId, @Nullable TerminalDisplay display, Collection<UUID> viewerIds,
                     BiConsumer<UUID, TerminalDeltaPacket> send) {
        if (display == null) return;
        sync(computerId, exact(display), viewerIds, send);
    }

    /**
     * Bring the given viewers up to date with the source's display: a
     * keyframe for those due one, the missed packets for viewers behind the
     * head that have room in their window, nothing for viewers whose window
     * is full. Timed frames are built as soon as they are queued. {@code send}
     * is called once per packet, in order, after all locks are released.
     */
    public void sync(UUID computerId, Source src, Collection<UUID> viewerIds,
                     BiConsumer<UUID, TerminalDeltaPacket> send) {
        TerminalDisplay display = src.display();
        if (display == null || viewerIds.isEmpty()) return;
        long t0 = System.nanoTime();
        try {
            syncPass(computerId, src, display, viewerIds, send);
        } finally {
            // Display work should never run on the server thread any more (Part A); count it if it does.
            if (com.example.evanscomputermod.computer.pipeline.DisplayPump.isServerThread(Thread.currentThread())) {
                com.example.evanscomputermod.computer.pipeline.DisplayPump.noteServerThread(System.nanoTime() - t0);
            }
        }
    }

    private void syncPass(UUID computerId, Source src, TerminalDisplay display, Collection<UUID> viewerIds,
                          BiConsumer<UUID, TerminalDeltaPacket> send) {
        List<UUID> to = new ArrayList<>();
        List<TerminalDeltaPacket> out = new ArrayList<>();
        long nowMs = System.currentTimeMillis();
        long nowUs = DisplayClock.nowUs();
        long wakeIn = -1;
        synchronized (this) {
            if (display != bound) {
                bound = display;
                started = false;
                history.clear();
                historyBytes = 0;
                timedPending.clear();
                for (ClientSyncState st : viewers.values()) st.needsKeyframe = true;
            }
            boolean wantFrame = false;
            for (UUID id : viewerIds) {
                ClientSyncState st = viewers.computeIfAbsent(id, k -> new ClientSyncState());
                st.checkTimeout(nowMs);
                if (st.shouldSendKeyframe() || st.canSend(windowFrames, windowBytes)) wantFrame = true;
            }
            lastSource = src;
            PresentQueue q = src.queue();
            synchronized (display) {
                PresentQueue.State qs = q == null ? null : q.state();
                if (qs != null && qs.target() != display) qs = null;
                // Queued frames are built whatever the windows say: they must exist before the server shows them.
                advanceTimed(computerId, display, qs);
                // Untimed changes coalesce while no viewer can take a frame.
                Step step = wantFrame ? advance(computerId, display, src, qs, nowUs) : null;
                TerminalDeltaPacket keyframe = step != null && step.keyframe ? step.packet : null;
                long keyframeGen = step != null && step.keyframe ? step.to : -1;
                for (UUID id : viewerIds) {
                    ClientSyncState st = viewers.get(id);
                    List<Step> chain = null;
                    if (!st.shouldSendKeyframe()) {
                        if (st.frameGen >= headGen || !st.canSend(windowFrames, windowBytes)) continue;
                        chain = chainFrom(st.frameGen);
                    }
                    if (chain == null) {
                        if (keyframe == null) {
                            keyframeGen = keyframeGeneration();
                            keyframe = buildKeyframe(computerId, display, keyframeGen);
                        }
                        to.add(id);
                        out.add(keyframe);
                        st.sent(keyframeGen, wire(keyframe), nowMs, nowUs);
                        // Frames queued and not shown yet follow the keyframe (the keyframe is "now").
                        for (Step s : timedPending) {
                            to.add(id);
                            out.add(s.packet);
                            st.sent(s.to, wire(s.packet), nowMs, nowUs);
                        }
                        st.markKeyframeSent(headGen);
                    } else {
                        long at = st.frameGen;
                        int decimate = decimation(id, st, nowUs);
                        for (int i = 0; i < chain.size(); i++) {
                            Step s = chain.get(i);
                            if (!st.canSend(windowFrames, windowBytes)) break;
                            at = s.to;
                            if (s.wholeLossy && skippable(chain, i, decimate)) {
                                framesSkipped++;
                                continue;
                            }
                            to.add(id);
                            out.add(s.packet);
                            st.sent(s.to, wire(s.packet), nowMs, nowUs);
                        }
                        st.markSent(at);
                    }
                }
                if (resyncPending) {
                    resyncPending = false;
                    wakeIn = 1_000_000; // queued frames left for the next pass
                } else if (!lossyTiles.isEmpty() && refineTimerFor != lastLossyUs) {
                    refineTimerFor = lastLossyUs;
                    wakeIn = Math.max(1_000_000, (lastLossyUs + REFINE_AFTER_US - nowUs) * 1000 + 5_000_000);
                }
            }
            packetsSent += out.size();
            for (int i = 0; i < out.size(); i++) {
                long n = wire(out.get(i));
                bytesSent += n;
                viewerBytes.merge(to.get(i), n, Long::sum);
                if (out.get(i).jpeg()) StreamBudget.note(to.get(i), this, nowUs);
            }
        }
        LongConsumer w = waker;
        if (wakeIn > 0 && w != null) w.accept(wakeIn);
        for (int i = 0; i < to.size(); i++) send.accept(to.get(i), out.get(i));
    }

    private static long wire(TerminalDeltaPacket p) {
        return p.compressedPayload().length + PACKET_OVERHEAD + (p.timed() ? 12 : 0);
    }

    /**
     * Skip a whole lossy frame for a viewer when a later whole lossy frame in
     * the same catch-up replaces it: untimed ones always (they are stale),
     * timed ones per the viewer's decimation (it gets every 2nd or 3rd).
     */
    private static boolean skippable(List<Step> chain, int i, int decimate) {
        boolean later = false;
        for (int j = i + 1; j < chain.size(); j++) {
            if (chain.get(j).wholeLossy) {
                later = true;
                break;
            }
            if (!chain.get(j).packet.jpeg() && chain.get(j).packet.compressedPayload().length > 0
                    && (chain.get(j).packet.flags() & TerminalDeltaPacket.FLAG_TEXT_ONLY) == 0) {
                return false; // an exact graphics step in between builds on this frame
            }
        }
        if (!later) return false;
        if (!chain.get(i).packet.timed()) return true;
        return decimate > 1 && chain.get(i).seq % decimate != 0;
    }

    /** Every Nth frame for a viewer whose link can't carry the stream at the lowest rung. */
    private int decimation(UUID id, ClientSyncState st, long nowUs) {
        double need = rungKbps(rung);
        double cap = capacityKbps(id, st, nowUs);
        if (need <= 0 || cap >= need) return 1;
        return need / cap > 2 ? 3 : 2;
    }

    /** What a viewer can take for this display, in kbit/s: its share of the budget, less when its link is full. */
    private double capacityKbps(UUID id, ClientSyncState st, long nowUs) {
        double cap = StreamBudget.shareKbps(id, this, nowUs);
        boolean saturated = !st.canSend(windowFrames, windowBytes) || st.timeouts > 0;
        if (saturated && st.rate.samples() > 4) cap = Math.min(cap, Math.max(200, st.rate.deliveredKbps(nowUs) * 1.1));
        return cap;
    }

    private double rungKbps(int r) {
        double bytes = rungBytes[r];
        if (bytes <= 0) return 0;
        return bytes * 8 / 1000.0 / Math.max(0.005, frameIntervalUs / 1_000_000.0);
    }

    /** The best rung whose estimated rate fits every viewer. */
    private int pickRung(long nowUs) {
        double cap = Double.MAX_VALUE;
        for (Map.Entry<UUID, ClientSyncState> e : viewers.entrySet()) cap = Math.min(cap, capacityKbps(e.getKey(), e.getValue(), nowUs));
        for (int r = 0; r < LossyCodec.RUNGS.length; r++) {
            double k = rungKbps(r);
            if (k <= 0) {
                // Not measured: try it if the rung above (if any) was measured too big, else start here.
                return r;
            }
            if (k <= cap) return r;
        }
        return LossyCodec.RUNGS.length - 1;
    }

    /**
     * Build steps for frames queued since the last pass (graphics only, timed)
     * and handle a flush. Caller holds this and the display's monitor.
     */
    private void advanceTimed(UUID computerId, TerminalDisplay d, @Nullable PresentQueue.State qs) {
        if (qs == null) {
            timedPending.clear();
            return;
        }
        if (qs.flushEpoch() != flushEpoch) {
            flushEpoch = qs.flushEpoch();
            boolean dropped = false;
            for (Step s : timedPending) dropped |= s.seq > qs.flushedAfterSeq();
            if (dropped && started) {
                long after = headGen;
                for (Step s : timedPending) {
                    if (s.seq > qs.flushedAfterSeq()) {
                        after = s.from;
                        break;
                    }
                }
                long gen = headGen + 1;
                commit(new Step(headGen, gen, TerminalDeltaPacket.createFlush(computerId, displayId, gen, after), false, 0, false));
                flushMarkers++;
                // The head goes back to what the screen showed at the flush: what clients show once they
                // drop the frames after it. What changed since then goes out as an ordinary delta.
                PresentQueue.Frame f = qs.flushedFrame();
                if (f != null && f.fits(d)) {
                    head.commitShadowGfx(f.width(), f.height(), f.pixels(), f.palette() != null ? f.palette() : d.getPalette(),
                            f.mode(), f.format());
                } else {
                    for (ClientSyncState st : viewers.values()) st.needsKeyframe = true;
                }
            }
            timedPending.removeIf(s -> s.seq > qs.flushedAfterSeq());
            timedSeq = Math.min(timedSeq, qs.appliedSeq());
        }
        timedPending.removeIf(s -> s.seq <= qs.appliedSeq());
        if (!started || !head.matchesGeometry(d)) return; // a keyframe comes first; frames after it are diffed then
        if (timedSeq < qs.appliedSeq()) {
            // Frames went on screen that this hub never built (nobody was watching): this pass diffs the
            // display's graphics untimed, and the frames still queued are built from there on the next pass.
            timedSeq = qs.appliedSeq();
            resyncPending = !qs.pending().isEmpty();
            return;
        }
        long nowUs = DisplayClock.nowUs();
        for (PresentQueue.Frame f : qs.pending()) {
            if (f.seq() <= timedSeq) continue;
            timedSeq = f.seq();
            if (!f.fits(d)) continue; // shape changed under it; the server shows it and the next pass resyncs
            FramebufferDiffTracker.GfxDelta gfx = head.computeGfxDelta(f.width(), f.height(), f.pixels(),
                    f.palette() != null ? f.palette() : d.getPalette(), f.mode(), f.format());
            long gen = headGen + 1;
            TerminalDeltaPacket p;
            boolean whole = false;
            int total = tiles(f.width(), f.height());
            boolean heavyFrame = gfx.changedTileIndices().size() * 2 >= total;
            noteGfxFrame(heavyFrame, nowUs);
            if (lossyNow(lastSource, f.format(), gfx.changedTileIndices().size(), total, nowUs)) {
                p = jpeg(computerId, gen, f.mode(), f.width(), f.height(), f.format(), f.pixels(), total, nowUs);
                whole = p != null;
            } else {
                p = null;
            }
            if (p == null) {
                p = TerminalDeltaPacket.createDelta(computerId, displayId, gen, f.mode(), null, gfx, d.getWidth(),
                        TerminalDeltaPacket.FLAG_GFX_ONLY, deflater);
                for (int t : gfx.changedTileIndices()) lossyTiles.clear(t);
            }
            p = p.withTime(f.ptsUs(), nowUs);
            head.commitShadowGfx(f.width(), f.height(), f.pixels(), f.palette() != null ? f.palette() : d.getPalette(),
                    f.mode(), f.format());
            Step s = new Step(headGen, gen, p, false, f.seq(), whole);
            commit(s);
            timedPending.addLast(s);
            timedBuilds++;
        }
    }

    /** The source of the pass being built (lossy settings for timed frames). */
    @Nullable
    private Source lastSource;
    /** Queued frames wait for the next pass (after a resync), and the lossy frame a refine timer is set for. */
    private boolean resyncPending;
    private long refineTimerFor = Long.MIN_VALUE;

    private static int tiles(int w, int h) {
        return ((w + 15) / 16) * ((h + 15) / 16);
    }

    private void noteGfxFrame(boolean heavyFrame, long nowUs) {
        heavy[heavyAt] = heavyFrame;
        heavyAt = (heavyAt + 1) % heavy.length;
        if (heavyFrame) lastHeavyUs = nowUs;
        if (lastGfxFrameUs > 0) {
            double dt = Math.max(1_000, Math.min(1_000_000, nowUs - lastGfxFrameUs));
            frameIntervalUs += (dt - frameIntervalUs) / 8.0;
        }
        lastGfxFrameUs = nowUs;
    }

    /** Should this graphics change go out as a JPEG frame? */
    private boolean lossyNow(@Nullable Source src, int format, int changed, int total, long nowUs) {
        if (src == null || !src.lossyAllowed() || !com.example.evanscomputermod.EcmConfig.displayAllowLossy()) return false;
        if (!LossyCodec.supports(format) || changed == 0 || total < 16) return false;
        int hint = src.hint();
        if (hint == 1) return false;
        if (hint == 2) return changed * 4 >= total;
        int n = 0;
        for (boolean b : heavy) if (b) n++;
        boolean on = n >= AUTO_LOSSY || (lossyMode && nowUs - lastHeavyUs < AUTO_EXIT_US);
        lossyMode = on;
        return on && changed * 2 >= total;
    }

    private boolean lossyMode;

    /** A JPEG step of a whole plane, or null if encoding failed (the caller sends it exact). */
    @Nullable
    private TerminalDeltaPacket jpeg(UUID computerId, long gen, int mode, int w, int h, int format, byte[] pixels,
                                     int totalTiles, long nowUs) {
        if (codec == null) codec = new LossyCodec();
        int r = pickRung(nowUs);
        byte[] j;
        try {
            j = codec.encode(pixels, w, h, format, r);
        } catch (java.io.IOException | RuntimeException e) {
            return null;
        }
        rungBytes[r] = rungBytes[r] <= 0 ? j.length : rungBytes[r] + (j.length - rungBytes[r]) / 4.0;
        rung = r;
        jpegBuilds++;
        lossyTiles.set(0, totalTiles);
        lastLossyUs = nowUs;
        return TerminalDeltaPacket.createJpeg(computerId, displayId, gen, mode, w, h, format, j);
    }

    /** Add a step at the head of the history. */
    private void commit(Step step) {
        headGen = step.to;
        history.addLast(step);
        historyBytes += step.packet.compressedPayload().length;
        while (history.size() > HISTORY_MAX || (historyBytes > HISTORY_MAX_BYTES && history.size() > 1)) {
            historyBytes -= history.removeFirst().packet.compressedPayload().length;
        }
    }

    /**
     * Diff the display against the head and, if anything changed, commit the
     * next generation. A change of shape (text grid, mode, pixel format,
     * graphics size) is a keyframe step, since deltas carry no sizes. While
     * {@code timedActive} (frames queued and sent ahead), only the text plane
     * is diffed. Caller holds this and the display's monitor.
     */
    @Nullable
    private Step advance(UUID computerId, TerminalDisplay d, Source src, @Nullable PresentQueue.State qs, long nowUs) {
        boolean timedActive = qs != null && timedSeq > qs.appliedSeq();
        long gen = headGen + 1;
        TerminalDeltaPacket packet;
        boolean keyframe;
        boolean whole = false;
        if (!started || !head.matchesGeometry(d)) {
            packet = TerminalDeltaPacket.createKeyframe(computerId, displayId, d, gen, deflater);
            headVersion = d.syncVersion();
            if (started) head.commitShadow(d);
            else head.initShadow(d);
            started = true;
            keyframe = true;
            keyframeBuilds++;
            lossyTiles.clear();
            // Queued frames not shown yet are built again from this keyframe on the next pass
            // (clients drop the timed frames they hold when a keyframe arrives).
            timedPending.clear();
            if (qs != null) {
                timedSeq = qs.appliedSeq();
                resyncPending = !qs.pending().isEmpty();
            }
        } else {
            int mode = d.getDisplayMode();
            FramebufferDiffTracker.TextDelta text = head.computeTextDelta(d);
            BitSet changed = d.changedTilesSince(headVersion);
            headVersion = d.syncVersion();
            boolean gfxPlane = mode >= 1 && !timedActive;
            FramebufferDiffTracker.GfxDelta gfx = gfxPlane ? head.computeGfxDelta(d, changed) : null;
            lastTilesCompared = gfxPlane ? head.lastTilesCompared() : 0;
            boolean textChanged = !text.changedRowIndices().isEmpty() || text.scrollOffset() != 0
                    || head.cursorDiffers(d);
            boolean gfxChanged = gfx != null && (!gfx.changedTileIndices().isEmpty() || gfx.paletteChanged());
            int total = tiles(d.getGfxWidth(), d.getGfxHeight());
            if (gfx != null && gfxChanged) {
                noteGfxFrame(gfx.changedTileIndices().size() * 2 >= total, nowUs);
            }
            boolean refine = false;
            if (gfxPlane && !gfxChanged && !lossyTiles.isEmpty() && nowUs - lastLossyUs >= REFINE_AFTER_US) {
                // The lossy picture stood still: send the tiles it covered exact, once.
                gfx = head.tilesOf(lossyTiles, d.getPalette());
                refine = true;
            }
            if (!textChanged && !gfxChanged && !refine) return null;
            boolean lossy = gfxChanged && !refine && lossyNow(src, d.getPixelFormat(), gfx.changedTileIndices().size(), total, nowUs);
            if (lossy) {
                // Text (if it changed) goes exact first, then the plane as a JPEG: two steps.
                if (textChanged) {
                    TerminalDeltaPacket tp = TerminalDeltaPacket.createDelta(computerId, displayId, gen, mode, text, null,
                            d.getWidth(), TerminalDeltaPacket.FLAG_TEXT_ONLY, deflater);
                    commit(new Step(headGen, gen, tp, false, 0, false));
                    deltaBuilds++;
                    gen = headGen + 1;
                }
                packet = jpeg(computerId, gen, mode, d.getGfxWidth(), d.getGfxHeight(), d.getPixelFormat(), d.getPixelData(), total, nowUs);
                whole = packet != null;
                if (packet == null) {
                    packet = TerminalDeltaPacket.createDelta(computerId, displayId, gen, mode, null, gfx, d.getWidth(),
                            TerminalDeltaPacket.FLAG_GFX_ONLY, deflater);
                }
            } else if (timedActive || mode < 1) {
                if (!textChanged) {
                    head.commitShadowText(d);
                    return null;
                }
                packet = TerminalDeltaPacket.createDelta(computerId, displayId, gen, mode, text, null, d.getWidth(),
                        mode >= 1 ? TerminalDeltaPacket.FLAG_TEXT_ONLY : 0, deflater);
            } else {
                packet = TerminalDeltaPacket.createDelta(computerId, displayId, gen, mode,
                        text, gfx, d.getWidth(), deflater);
                if (gfx != null) {
                    if (refine) {
                        lossyTiles.clear();
                        refines++;
                    } else {
                        for (int t : gfx.changedTileIndices()) lossyTiles.clear(t);
                    }
                }
            }
            if (timedActive) head.commitShadowText(d);
            else head.commitShadow(d);
            keyframe = false;
            deltaBuilds++;
        }
        Step step = new Step(headGen, gen, packet, keyframe, 0, whole);
        commit(step);
        return step;
    }

    /**
     * The generation a keyframe of the display now stands for: the head, or
     * while timed frames are out the step before the first of them (the
     * display shows the frame before those).
     */
    private long keyframeGeneration() {
        return timedPending.isEmpty() ? headGen : timedPending.peekFirst().from;
    }

    /** A keyframe of the display now (its graphics may be behind the head while timed frames are out). */
    private TerminalDeltaPacket buildKeyframe(UUID computerId, TerminalDisplay d, long gen) {
        keyframeBuilds++;
        return TerminalDeltaPacket.createKeyframe(computerId, displayId, d, gen, deflater);
    }

    /** The steps from generation {@code from} up to the head, or null if the history doesn't reach back. */
    @Nullable
    private List<Step> chainFrom(long from) {
        List<Step> chain = null;
        for (Step s : history) {
            if (chain == null) {
                if (s.from != from) continue;
                chain = new ArrayList<>();
            }
            chain.add(s);
        }
        return chain;
    }

    /** Drop viewers that are no longer watching. */
    public synchronized void retainViewers(Collection<UUID> ids) {
        viewers.keySet().retainAll(ids);
    }

    /** A client acked {@code generation}. Returns false if it isn't a viewer here. */
    public boolean onClientReady(UUID viewer, long generation) {
        return onClientReady(viewer, generation, 0);
    }

    /**
     * A client acked {@code generation}, which it received at its own time
     * {@code clientRecvUs} (0 = unknown). Returns true if it is a viewer here
     * that may take more now (its window has room and it is behind).
     */
    public synchronized boolean onClientReady(UUID viewer, long generation, long clientRecvUs) {
        ClientSyncState st = viewers.get(viewer);
        if (st == null) return false;
        st.onClientReady(generation, clientRecvUs, DisplayClock.nowUs());
        return true;
    }

    /** The viewer (added if new) gets a keyframe on the next pass. */
    public synchronized void requestKeyframe(UUID viewer) {
        viewers.computeIfAbsent(viewer, k -> new ClientSyncState()).needsKeyframe = true;
    }

    /** Every viewer gets a keyframe on the next pass. */
    public synchronized void forceKeyframe() {
        for (ClientSyncState st : viewers.values()) st.needsKeyframe = true;
    }

    /** Forget all viewers (the display was replaced); generations keep counting up. */
    public synchronized void reset() {
        viewers.clear();
        history.clear();
        historyBytes = 0;
        timedPending.clear();
        started = false;
        bound = null;
    }

    public synchronized int viewerCount() {
        return viewers.size();
    }

    /** Delta packets built so far (one per generation, however many viewers). */
    public synchronized long deltaBuilds() {
        return deltaBuilds;
    }

    /** Keyframe packets built so far. */
    public synchronized long keyframeBuilds() {
        return keyframeBuilds;
    }

    /** Steps built for queued (timed) frames so far. */
    public synchronized long timedBuilds() {
        return timedBuilds;
    }

    /** JPEG frames built so far (one per frame, however many viewers). */
    public synchronized long jpegBuilds() {
        return jpegBuilds;
    }

    /** Exact refines of a lossy picture that stood still. */
    public synchronized long refines() {
        return refines;
    }

    public synchronized long flushMarkers() {
        return flushMarkers;
    }

    /** Lossy frames a viewer was skipped (it couldn't keep up, or a newer frame replaced them). */
    public synchronized long framesSkipped() {
        return framesSkipped;
    }

    /** The quality rung of the last JPEG (0 best .. 4 half resolution). */
    public synchronized int rung() {
        return rung;
    }

    /** Tiles clients hold lossy now (empty once refined). */
    public synchronized int lossyTiles() {
        return lossyTiles.cardinality();
    }

    /** Packets handed to viewers so far (a shared packet counts once per viewer). */
    public synchronized long packetsSent() {
        return packetsSent;
    }

    /** Bytes handed to viewers so far, including {@link #PACKET_OVERHEAD} per packet. */
    public synchronized long bytesSent() {
        return bytesSent;
    }

    /** Bytes handed to one viewer so far. */
    public synchronized long bytesSent(UUID viewer) {
        return viewerBytes.getOrDefault(viewer, 0L);
    }

    /** Graphics tiles the last delta build compared, or -1 when it compared the whole plane. */
    public synchronized int lastTilesCompared() {
        return lastTilesCompared;
    }

    public synchronized long headGeneration() {
        return headGen;
    }

    /** One viewer's link as the acks tell it: {rtt ms, jitter ms, kbps, frames in flight, timeouts}, or null. */
    @Nullable
    public synchronized double[] viewerStats(UUID viewer) {
        ClientSyncState st = viewers.get(viewer);
        if (st == null) return null;
        double[] r = st.rate.snapshot(DisplayClock.nowUs());
        return new double[]{r[0], r[1], r[2], st.inFlightFrames(), st.timeouts};
    }

    public synchronized java.util.Set<UUID> viewerIds() {
        return java.util.Set.copyOf(viewers.keySet());
    }
}
