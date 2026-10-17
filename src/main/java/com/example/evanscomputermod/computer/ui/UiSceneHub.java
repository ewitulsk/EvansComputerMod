package com.example.evanscomputermod.computer.ui;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.function.Supplier;
import java.util.zip.Deflater;

/**
 * Streams one computer's scene (§14 phase 2a) to every player who renders
 * it client-side, the scene-stream counterpart of
 * {@code DisplaySyncHub}: each step is built once by the
 * {@link SceneExporter} and the same bytes go to every viewer of every
 * display showing the scene (the Terminal GUI and a mirroring Screen wall).
 *
 * <p>Viewers are keyed by player and display id. A new viewer, one whose
 * ack says it lost track, or one that fell further behind than the history
 * reaches gets a keyframe (built at most once per pass and shared). A
 * viewer may have {@link #WINDOW} steps unacknowledged; past that it waits,
 * and its ack sends the missed steps from the history in order. Image data
 * goes out once per client by content key ({@link UiClients}) right before
 * the first message that needs it. A viewer that leaves (the GUI was
 * hidden, client rendering turned off) is sent {@link UiSceneCodec#END} so
 * its client goes back to tile deltas.
 *
 * <p>Flow control (DISPLAY_PIPELINE_SPEC.md Part B3): at most {@link #WINDOW}
 * steps and {@code display.windowBytes} bytes unacknowledged per viewer; a
 * message unacked for {@link #ACK_TIMEOUT_MS} restarts the viewer with a
 * keyframe. Acks carry the client's receive time for each viewer's link
 * estimate.
 *
 * <p>Threads: steps arrive from the compositor's tick ({@link #offer}, any
 * thread); {@link #sync} runs on the display pump and the acks on the
 * network thread. Keyframes take the compositor's monitor, never while this
 * hub's is held.
 */
public final class UiSceneHub {
    /** Steps kept for viewers that fell behind, and their total size. */
    static final int HISTORY_MAX = 64;
    static final long HISTORY_MAX_BYTES = 2L << 20;
    /** Unacknowledged steps a viewer may have in flight. */
    public static final int WINDOW = 32;
    /** A message unacked this long restarts the viewer with a keyframe (ms, Part B3). */
    public static final long ACK_TIMEOUT_MS = 1_000;
    /**
     * Bytes counted per message on top of its data, what the game's custom
     * payload packet adds: the packet id, the payload type
     * ({@code evanscomputermod:ui_step}) and, for steps, the short stream id
     * and length prefix ({@link #STEP_OVERHEAD}); for other messages
     * ({@code evanscomputermod:ui_scene}) the computer UUID, display id,
     * stream id and an int length ({@link #FULL_OVERHEAD}).
     */
    public static final int STEP_OVERHEAD = 30, FULL_OVERHEAD = 49;

    /** The bytes a message costs on the wire. */
    public static int wireBytes(byte[] data) {
        return data.length + (UiSceneCodec.kindOf(data) == UiSceneCodec.STEP ? STEP_OVERHEAD : FULL_OVERHEAD);
    }

    /** Where packets go: one call per message, in order, outside every lock. */
    public interface Sink {
        void send(UUID player, int displayId, byte[] data);
    }

    public record ViewerKey(UUID player, int displayId) {}

    /** One viewer's place in the stream and what it has cost. */
    public static final class Viewer {
        long sentGen = -1, ackedGen = -1;
        boolean needsKeyframe = true;
        long bytes, messages, keyframes, steps, resources, timeouts, timed;
        /** The last timed message sent (the hub's timed sequence), and the timed ones not acked yet. */
        long timedSent = -1;
        final ArrayDeque<long[]> inFlightTimed = new ArrayDeque<>();
        /** Messages sent and not acked: {generation, bytes, sent at ms} (Part B3: the byte window and the timeout). */
        final ArrayDeque<long[]> inFlight = new ArrayDeque<>();
        long inFlightBytes;
        /** What the acks say about the link (RTT, jitter, throughput; Part B2). */
        final com.example.evanscomputermod.computer.pipeline.ViewerRate rate = new com.example.evanscomputermod.computer.pipeline.ViewerRate();

        public long timeouts() {
            return timeouts;
        }

        /** Timed pixel messages sent (protocol v2 CommitAt frames). */
        public long timed() {
            return timed;
        }

        /** {rtt ms, jitter ms, delivered kbps}. */
        public double[] link() {
            return rate.snapshot(com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs());
        }

        public long bytes() {
            return bytes;
        }

        public long messages() {
            return messages;
        }

        public long keyframes() {
            return keyframes;
        }

        public long steps() {
            return steps;
        }

        public long resources() {
            return resources;
        }

        public long sentGeneration() {
            return sentGen;
        }
    }

    private final Object inLock = new Object();
    private final ArrayDeque<SceneExporter.Step> incoming = new ArrayDeque<>();
    private final ArrayDeque<SceneExporter.Step> history = new ArrayDeque<>();
    private long historyBytes;
    private int epoch = -1;
    private long headGen;
    private final Map<ViewerKey, Viewer> viewers = new HashMap<>();
    private final Map<Integer, Long> displayBytes = new HashMap<>();
    private final Deflater deflater = new Deflater(Deflater.BEST_SPEED);
    private long totalBytes, keyframesBuilt;

    // ---- timed pixels (protocol v2 CommitAt; DISPLAY_PIPELINE_SPEC.md Parts C5, D)
    /** Generation bit of an ack for a timed message (the rest is its timed sequence). */
    public static final long TIMED_ACK = 1L << 62;
    static final int TIMED_LOG_MAX = 96;
    static final long TIMED_LOG_BYTES = 8L << 20;
    static final long REFINE_AFTER_US = 250_000;

    private record TimedMsg(long tseq, long key, long seq, byte[] data, boolean wholeLossy) {}

    /** One canvas's or pixel surface's timed stream as the hub built it. */
    private static final class TStream {
        int w, h;
        /** The last frame built (what the next one is diffed against), and its frame number. */
        int[] shadow;
        long shadowSeq;
        final ArrayDeque<TimedMsg> pending = new ArrayDeque<>();
        final boolean[] heavy = new boolean[10];
        int heavyAt;
        long lastHeavyUs = Long.MIN_VALUE / 2, lastFrameUs;
        boolean lossyMode, lossyOut;
        int target, scope, id;
    }

    private final ArrayDeque<SceneExporter.TimedJob> timedJobs = new ArrayDeque<>();
    private final Map<Long, TStream> streams = new HashMap<>();
    private final ArrayDeque<TimedMsg> timedLog = new ArrayDeque<>();
    private long timedLogBytes, tseq;
    private long timedBuilt, timedJpeg, timedSkipped, timedRefines;
    private final double[] rungBytes = new double[com.example.evanscomputermod.computer.pipeline.LossyCodec.RUNGS.length];
    private int rung;
    private com.example.evanscomputermod.computer.pipeline.LossyCodec codec;
    /** Asks for another pass in so many ns (refine timers). */
    private volatile java.util.function.LongConsumer waker;

    public void setWaker(java.util.function.LongConsumer w) {
        this.waker = w;
    }

    /** A timed job from the exporter (compositor lock held; any thread). */
    public void offerTimed(SceneExporter.TimedJob j) {
        synchronized (inLock) {
            timedJobs.addLast(j);
            while (timedJobs.size() > 1024) timedJobs.removeFirst();
        }
    }

    /** Build the messages of the timed jobs that came in: once each, shared by every viewer. Hub lock held. */
    private void drainTimed(long nowUs) {
        List<SceneExporter.TimedJob> got;
        synchronized (inLock) {
            if (timedJobs.isEmpty()) return;
            got = new ArrayList<>(timedJobs);
            timedJobs.clear();
        }
        for (SceneExporter.TimedJob j : got) {
            TStream st = streams.computeIfAbsent(j.key(), k -> new TStream());
            st.target = j.target();
            st.scope = j.scope();
            st.id = j.id();
            if (st.w != j.w() || st.h != j.h()) {
                st.w = j.w();
                st.h = j.h();
                st.shadow = null;
                st.shadowSeq = 0;
            }
            switch (j.kind()) {
                case SceneExporter.TimedJob.FRAME -> buildTimed(st, j, nowUs);
                case SceneExporter.TimedJob.SHOWN -> st.pending.removeIf(m -> m.seq() <= j.seq());
                default -> {
                    st.pending.removeIf(m -> m.seq() > j.seq());
                    st.shadow = j.argb();
                    st.shadowSeq = j.seq();
                    byte[] body = UiSceneCodec.encodeTimedFlush(++tseq, st.target, st.scope, st.id, j.seq());
                    log(new TimedMsg(tseq, j.key(), j.seq(), UiSceneCodec.pack(UiSceneCodec.TFLUSH, epoch, body, deflater), false));
                }
            }
        }
    }

    private void log(TimedMsg m) {
        timedLog.addLast(m);
        timedLogBytes += m.data().length;
        while (timedLog.size() > TIMED_LOG_MAX || (timedLogBytes > TIMED_LOG_BYTES && timedLog.size() > 1)) {
            timedLogBytes -= timedLog.removeFirst().data().length;
        }
    }

    private void buildTimed(TStream st, SceneExporter.TimedJob j, long nowUs) {
        int w = j.w(), h = j.h(), T = UiSceneCodec.TILE;
        int tx = (w + T - 1) / T, ty = (h + T - 1) / T, total = tx * ty;
        int[] px = j.argb();
        List<Integer> changed = new ArrayList<>();
        if (st.shadow == null || st.shadow.length != px.length) {
            for (int t = 0; t < total; t++) changed.add(t);
        } else {
            for (int t = 0; t < total; t++) {
                int x0 = (t % tx) * T, y0 = (t / tx) * T, x1 = Math.min(w, x0 + T), y1 = Math.min(h, y0 + T);
                for (int y = y0; y < y1; y++) {
                    if (!java.util.Arrays.equals(px, y * w + x0, y * w + x1, st.shadow, y * w + x0, y * w + x1)) {
                        changed.add(t);
                        break;
                    }
                }
            }
        }
        boolean heavyFrame = changed.size() * 2 >= total;
        st.heavy[st.heavyAt] = heavyFrame;
        st.heavyAt = (st.heavyAt + 1) % st.heavy.length;
        if (heavyFrame) st.lastHeavyUs = nowUs;
        st.lastFrameUs = nowUs;
        boolean lossy = lossy(st, j, changed.size(), total, nowUs);
        byte[] body;
        long base = st.shadow == null ? 0 : st.shadowSeq;
        long send = com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs();
        if (lossy) {
            if (codec == null) codec = new com.example.evanscomputermod.computer.pipeline.LossyCodec();
            int r = pickRung(nowUs);
            byte[] jpeg = null;
            try {
                jpeg = codec.encodeArgb(px, w, h, r);
            } catch (java.io.IOException | RuntimeException e) {
                lossy = false;
            }
            if (jpeg != null) {
                rungBytes[r] = rungBytes[r] <= 0 ? jpeg.length : rungBytes[r] + (jpeg.length - rungBytes[r]) / 4.0;
                rung = r;
                timedJpeg++;
                st.lossyOut = true;
                body = UiSceneCodec.encodeTimed(++tseq, st.target, st.scope, st.id, w, h, j.seq(), base, j.ptsUs(), send,
                        null, null, jpeg);
                emitTimed(st, j, body, true);
                return;
            }
        }
        boolean all = changed.size() == total;
        int[] tiles = all ? null : changed.stream().mapToInt(Integer::intValue).toArray();
        int[] out = tilePixels(px, w, h, tiles);
        body = UiSceneCodec.encodeTimed(++tseq, st.target, st.scope, st.id, w, h, j.seq(), all ? 0 : base, j.ptsUs(), send,
                tiles, out, null);
        if (all) st.lossyOut = false;
        emitTimed(st, j, body, false);
    }

    private void emitTimed(TStream st, SceneExporter.TimedJob j, byte[] body, boolean wholeLossy) {
        TimedMsg m = new TimedMsg(tseq, j.key(), j.seq(), UiSceneCodec.pack(UiSceneCodec.TIMED, epoch, body, deflater), wholeLossy);
        log(m);
        st.pending.addLast(m);
        st.shadow = j.argb();
        st.shadowSeq = j.seq();
        timedBuilt++;
    }

    private static int[] tilePixels(int[] px, int w, int h, int[] tiles) {
        if (tiles == null) return px.clone();
        int T = UiSceneCodec.TILE, tx = (w + T - 1) / T;
        int n = 0;
        for (int t : tiles) n += UiSceneCodec.tileW(t, tx, w) * UiSceneCodec.tileH(t, tx, h);
        int[] out = new int[n];
        int k = 0;
        for (int t : tiles) {
            int x0 = (t % tx) * T, y0 = (t / tx) * T, x1 = Math.min(w, x0 + T), y1 = Math.min(h, y0 + T);
            for (int y = y0; y < y1; y++) {
                System.arraycopy(px, y * w + x0, out, k, x1 - x0);
                k += x1 - x0;
            }
        }
        return out;
    }

    /** Lossy for this frame? Same rules as the tile path (Part D1). */
    private boolean lossy(TStream st, SceneExporter.TimedJob j, int changed, int total, long nowUs) {
        if (!com.example.evanscomputermod.EcmConfig.displayAllowLossy() || j.tier() <= 1 || j.hint() == 1) return false;
        if (changed == 0 || total < 16) return false;
        if (j.hint() == 2) return changed * 4 >= total;
        int n = 0;
        for (boolean b : st.heavy) if (b) n++;
        st.lossyMode = n >= 8 || (st.lossyMode && nowUs - st.lastHeavyUs < 1_000_000);
        return st.lossyMode && changed * 2 >= total;
    }

    /** The best rung whose rate fits every viewer's share of the budget (a saturated one: what it delivers). */
    private int pickRung(long nowUs) {
        double cap = Double.MAX_VALUE;
        for (Map.Entry<ViewerKey, Viewer> e : viewers.entrySet()) {
            Viewer v = e.getValue();
            double c = com.example.evanscomputermod.computer.pipeline.StreamBudget.shareKbps(e.getKey().player(), this, nowUs);
            if ((!roomFor(v) || v.timeouts > 0) && v.rate.samples() > 4) c = Math.min(c, Math.max(200, v.rate.deliveredKbps(nowUs) * 1.1));
            cap = Math.min(cap, c);
        }
        double fps = 30;
        for (int r = 0; r < rungBytes.length; r++) {
            if (rungBytes[r] <= 0) return r;
            if (rungBytes[r] * 8 / 1000.0 * fps <= cap) return r;
        }
        return rungBytes.length - 1;
    }

    /**
     * Lossy pictures that stood still get one exact frame (refine): the
     * exporter sends the canvas or surface whole and exact in its next step,
     * from what the compositor shows now (the app may have committed it again
     * untimed after the last timed frame). Clients show it after any timed
     * frames still waiting.
     */
    private void refine(long nowUs, SceneExporter ex) {
        if (ex == null) return;
        for (TStream st : streams.values()) {
            if (!st.lossyOut || !st.pending.isEmpty() || nowUs - st.lastFrameUs < REFINE_AFTER_US) continue;
            ex.refine(st.target, st.scope, st.id);
            st.lossyOut = false;
            timedRefines++;
        }
    }

    /** The timed messages after {@code from} (tseq), or null when the log doesn't reach back. */
    private List<TimedMsg> timedFrom(long from) {
        if (timedLog.isEmpty() || timedLog.peekLast().tseq() <= from) return List.of();
        if (timedLog.peekFirst().tseq() > from + 1) return null;
        List<TimedMsg> out = new ArrayList<>();
        for (TimedMsg m : timedLog) if (m.tseq() > from) out.add(m);
        return out;
    }

    /** Skip a whole lossy frame that a later whole lossy frame of the same target in this batch replaces. */
    private static boolean superseded(List<TimedMsg> batch, int i) {
        TimedMsg m = batch.get(i);
        if (!m.wholeLossy()) return false;
        for (int k = i + 1; k < batch.size(); k++) {
            TimedMsg n = batch.get(k);
            if (n.key() != m.key()) continue;
            return n.wholeLossy();
        }
        return false;
    }

    /** A step from the exporter (compositor thread). */
    public void offer(SceneExporter.Step s) {
        synchronized (inLock) {
            incoming.addLast(s);
            // A hub nobody drains (no sync passes) must not grow without bound.
            while (incoming.size() > HISTORY_MAX * 4) incoming.removeFirst();
        }
    }

    private void drain() {
        List<SceneExporter.Step> got;
        synchronized (inLock) {
            if (incoming.isEmpty()) return;
            got = new ArrayList<>(incoming);
            incoming.clear();
        }
        for (SceneExporter.Step s : got) {
            if (s.epoch() != epoch) restart(s.epoch());
            if (s.from() != headGen && !history.isEmpty()) {
                // A hole (steps dropped while nobody drained): everyone starts over.
                history.clear();
                historyBytes = 0;
                for (Viewer v : viewers.values()) v.needsKeyframe = true;
            }
            history.addLast(s);
            historyBytes += s.data().length;
            headGen = s.to();
            while (history.size() > HISTORY_MAX || (historyBytes > HISTORY_MAX_BYTES && history.size() > 1)) {
                historyBytes -= history.removeFirst().data().length;
            }
        }
    }

    private void restart(int newEpoch) {
        epoch = newEpoch;
        history.clear();
        historyBytes = 0;
        headGen = 0;
        streams.clear();
        timedLog.clear();
        timedLogBytes = 0;
        for (Viewer v : viewers.values()) {
            v.needsKeyframe = true;
            v.sentGen = -1;
            v.ackedGen = -1;
            v.timedSent = -1;
            v.inFlightTimed.clear();
        }
    }

    /**
     * One pass for display {@code displayId}: {@code ids} are the players
     * who should render it client-side now. Others that did are sent END.
     * {@code exporter} supplies the current exporter for keyframes (null
     * when there is no scene, which ends everyone).
     */
    public void sync(int displayId, Collection<UUID> ids, Supplier<SceneExporter> exporter, Sink sink) {
        long t0 = System.nanoTime();
        try {
            syncPass(displayId, ids, exporter, sink);
        } finally {
            if (com.example.evanscomputermod.computer.pipeline.DisplayPump.isServerThread(Thread.currentThread())) {
                com.example.evanscomputermod.computer.pipeline.DisplayPump.noteServerThread(System.nanoTime() - t0);
            }
        }
    }

    private void syncPass(int displayId, Collection<UUID> ids, Supplier<SceneExporter> exporter, Sink sink) {
        List<Object[]> out = new ArrayList<>();
        boolean wantKeyframe = false;
        SceneExporter ex = exporter.get();
        Set<UUID> want = new HashSet<>(ex == null ? List.of() : ids);
        long nowMs = System.currentTimeMillis();
        long nowUs = com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs();
        synchronized (this) {
            drain();
            drainTimed(nowUs);
            refine(nowUs, ex);
            for (Viewer v : viewers.values()) {
                long[] oldest = v.inFlight.peekFirst();
                long[] oldestT = v.inFlightTimed.peekFirst();
                if (oldest == null || (oldestT != null && oldestT[2] < oldest[2])) oldest = oldestT;
                if (oldest != null && nowMs - oldest[2] > ACK_TIMEOUT_MS) {
                    // A lost ack or a stalled client: start it over rather than waiting forever.
                    v.inFlight.clear();
                    v.inFlightTimed.clear();
                    v.inFlightBytes = 0;
                    v.rate.reset();
                    v.needsKeyframe = true;
                    v.timeouts++;
                }
            }
            for (var it = viewers.entrySet().iterator(); it.hasNext(); ) {
                var e = it.next();
                if (e.getKey().displayId() != displayId || want.contains(e.getKey().player())) continue;
                out.add(new Object[]{e.getKey(), UiSceneCodec.pack(UiSceneCodec.END, epoch, null, deflater)});
                it.remove();
            }
            for (UUID id : want) {
                Viewer v = viewers.computeIfAbsent(new ViewerKey(id, displayId), k -> new Viewer());
                if (v.needsKeyframe || (v.sentGen < headGen && chainFrom(v.sentGen) == null)) wantKeyframe = true;
            }
        }
        SceneExporter.Step kf = null;
        if (wantKeyframe && ex != null) {
            kf = ex.keyframe();
            synchronized (this) {
                keyframesBuilt++;
            }
        }
        synchronized (this) {
            drain();
            drainTimed(nowUs);
            for (UUID id : want) {
                ViewerKey key = new ViewerKey(id, displayId);
                Viewer v = viewers.get(key);
                if (v == null) continue;
                List<SceneExporter.Step> chain = null;
                if (!v.needsKeyframe) {
                    if (v.sentGen >= headGen || !roomFor(v)) continue;
                    chain = chainFrom(v.sentGen);
                }
                if (chain == null) {
                    if (kf == null || kf.epoch() != epoch || kf.to() != headGen) continue; // next pass
                    resources(key, v, kf.images(), ex, out);
                    out.add(new Object[]{key, kf.data()});
                    v.keyframes++;
                    v.sentGen = kf.to();
                    v.ackedGen = kf.to();
                    v.needsKeyframe = false;
                    v.inFlight.clear();
                    v.inFlightTimed.clear();
                    v.inFlightBytes = 0;
                    sent(v, kf.to(), kf.data().length, nowMs);
                    // Frames queued and not shown in the keyframe follow it (the keyframe is "now").
                    for (Map.Entry<Long, TStream> e : streams.entrySet()) {
                        long shown = kf.timedShown().getOrDefault(e.getKey(), 0L);
                        for (TimedMsg m : e.getValue().pending) {
                            if (m.seq() <= shown) continue;
                            out.add(new Object[]{key, m.data()});
                            sentTimed(v, m, nowMs);
                        }
                    }
                    v.timedSent = tseq;
                } else {
                    for (SceneExporter.Step s : chain) {
                        if (!roomFor(v)) break;
                        if (s.images() != null) resources(key, v, s.images(), ex, out);
                        out.add(new Object[]{key, s.data()});
                        v.steps++;
                        v.sentGen = s.to();
                        sent(v, s.to(), s.data().length, nowMs);
                    }
                }
            }
            // Timed pixels after the steps (a step may create the canvas they draw into); they don't
            // wait for the step window, only for the byte window.
            for (UUID id : want) {
                ViewerKey key = new ViewerKey(id, displayId);
                Viewer v = viewers.get(key);
                if (v != null) sendTimed(key, v, out, nowMs);
            }
            for (Object[] o : out) {
                ViewerKey k = (ViewerKey) o[0];
                int n = wireBytes((byte[]) o[1]);
                Viewer v = viewers.get(k);
                if (v != null) {
                    v.bytes += n;
                    v.messages++;
                }
                displayBytes.merge(k.displayId(), (long) n, Long::sum);
                totalBytes += n;
            }
        }
        for (Object[] o : out) {
            ViewerKey k = (ViewerKey) o[0];
            sink.send(k.player(), k.displayId(), (byte[]) o[1]);
        }
    }

    /**
     * The timed messages the viewer hasn't had (before its steps, which they
     * never wait for). A viewer the log no longer reaches back for gets a
     * keyframe on the next pass.
     */
    private void sendTimed(ViewerKey key, Viewer v, List<Object[]> out, long nowMs) {
        if (v.needsKeyframe || v.timedSent < 0) return;
        List<TimedMsg> batch = timedFrom(v.timedSent);
        if (batch == null) {
            v.needsKeyframe = true;
            return;
        }
        for (int i = 0; i < batch.size(); i++) {
            TimedMsg m = batch.get(i);
            if (v.inFlightBytes >= com.example.evanscomputermod.EcmConfig.displayWindowBytes()) break;
            v.timedSent = m.tseq();
            if (superseded(batch, i)) {
                timedSkipped++;
                continue;
            }
            out.add(new Object[]{key, m.data()});
            sentTimed(v, m, nowMs);
        }
    }

    private static void sentTimed(Viewer v, TimedMsg m, long nowMs) {
        v.inFlightTimed.addLast(new long[]{m.tseq(), m.data().length, nowMs});
        v.inFlightBytes += m.data().length;
        v.timed++;
    }

    /** May the viewer take another step: under {@link #WINDOW} steps and {@code display.windowBytes} unacked? */
    private static boolean roomFor(Viewer v) {
        return v.sentGen - v.ackedGen < WINDOW
                && v.inFlightBytes < com.example.evanscomputermod.EcmConfig.displayWindowBytes();
    }

    private static void sent(Viewer v, long gen, long bytes, long nowMs) {
        v.inFlight.addLast(new long[]{gen, bytes, nowMs});
        v.inFlightBytes += bytes;
        v.rate.sent(gen, bytes, com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs());
    }

    /** Image data the viewer's client doesn't hold yet, queued before the message that needs it. */
    private void resources(ViewerKey key, Viewer v, long[] keys, SceneExporter ex, List<Object[]> out) {
        if (keys == null || ex == null) return;
        for (long k : keys) {
            if (!UiClients.markSent(key.player(), k)) continue;
            SceneExporter.Image img = ex.image(k);
            if (img == null) {
                UiClients.forget(key.player(), k);
                continue;
            }
            byte[] body = UiSceneCodec.encodeResource(k, img.w(), img.h(), img.argb());
            out.add(new Object[]{key, UiSceneCodec.pack(UiSceneCodec.RESOURCE, epoch, body, deflater)});
            v.resources++;
        }
    }

    /** The steps from generation {@code from} to the head, or null if the history doesn't reach back. */
    private List<SceneExporter.Step> chainFrom(long from) {
        List<SceneExporter.Step> chain = null;
        for (SceneExporter.Step s : history) {
            if (chain == null) {
                if (s.from() != from) continue;
                chain = new ArrayList<>();
            }
            chain.add(s);
        }
        return chain;
    }

    /**
     * A client acked generation {@code gen} of {@code epoch} (stale epochs
     * are ignored). {@code gen <= 0} asks for a keyframe; {@code missing}
     * are image keys it doesn't hold after all. Returns true if a pass
     * should run now (the viewer may take more).
     */
    public boolean onAck(UUID player, int displayId, int ackEpoch, long gen, long[] missing) {
        return onAck(player, displayId, ackEpoch, gen, missing, 0);
    }

    /**
     * {@link #onAck(UUID, int, int, long, long[])} with the client's receive
     * time ({@code clientRecvUs}, microseconds on its clock, 0 = unknown) for
     * the viewer's link estimate. Any thread.
     */
    public synchronized boolean onAck(UUID player, int displayId, int ackEpoch, long gen, long[] missing, long clientRecvUs) {
        Viewer v = viewers.get(new ViewerKey(player, displayId));
        if (v == null) return false;
        if (missing != null) for (long k : missing) UiClients.forget(player, k);
        if (gen <= 0) {
            v.needsKeyframe = true;
            return true;
        }
        if (ackEpoch != epoch) return false;
        if ((gen & TIMED_ACK) != 0) {
            long t = gen & ~TIMED_ACK;
            boolean full = v.inFlightBytes >= com.example.evanscomputermod.EcmConfig.displayWindowBytes();
            while (!v.inFlightTimed.isEmpty() && v.inFlightTimed.peekFirst()[0] <= t) {
                v.inFlightBytes -= v.inFlightTimed.removeFirst()[1];
            }
            return full || v.timedSent < tseq;
        }
        boolean blocked = !roomFor(v);
        v.ackedGen = Math.max(v.ackedGen, gen);
        while (!v.inFlight.isEmpty() && v.inFlight.peekFirst()[0] <= gen) v.inFlightBytes -= v.inFlight.removeFirst()[1];
        v.rate.acked(gen, clientRecvUs, com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs());
        return blocked || v.sentGen < headGen;
    }

    /** The viewer (if it is one) gets a keyframe on the next pass. */
    public synchronized void requestKeyframe(UUID player, int displayId) {
        Viewer v = viewers.get(new ViewerKey(player, displayId));
        if (v != null) v.needsKeyframe = true;
    }

    public synchronized boolean isViewer(UUID player, int displayId) {
        return viewers.containsKey(new ViewerKey(player, displayId));
    }

    public synchronized boolean hasViewers() {
        return !viewers.isEmpty();
    }

    public synchronized int viewerCount() {
        return viewers.size();
    }

    /** A copy of one viewer's counters, or null. */
    public synchronized Viewer viewer(UUID player, int displayId) {
        Viewer v = viewers.get(new ViewerKey(player, displayId));
        if (v == null) return null;
        Viewer c = new Viewer();
        c.sentGen = v.sentGen;
        c.ackedGen = v.ackedGen;
        c.needsKeyframe = v.needsKeyframe;
        c.bytes = v.bytes;
        c.messages = v.messages;
        c.keyframes = v.keyframes;
        c.steps = v.steps;
        c.resources = v.resources;
        c.timeouts = v.timeouts;
        c.inFlightBytes = v.inFlightBytes;
        return c;
    }

    public synchronized Map<ViewerKey, Viewer> viewers() {
        Map<ViewerKey, Viewer> out = new HashMap<>();
        for (ViewerKey k : viewers.keySet()) out.put(k, viewer(k.player(), k.displayId()));
        return out;
    }

    /** Bytes sent for one display (all its viewers), including the per-packet overhead ({@link #wireBytes}). */
    public synchronized long displayBytes(int displayId) {
        return displayBytes.getOrDefault(displayId, 0L);
    }

    public synchronized long totalBytes() {
        return totalBytes;
    }

    public synchronized long keyframesBuilt() {
        return keyframesBuilt;
    }

    /** Timed frames built (once each), as JPEG, skipped for a lagging viewer, refined: {built, jpeg, skipped, refines}. */
    public synchronized long[] timedCounters() {
        return new long[]{timedBuilt, timedJpeg, timedSkipped, timedRefines};
    }

    /** The quality rung of the last JPEG timed frame (0 best .. 4 half resolution). */
    public synchronized int rung() {
        return rung;
    }

    /** Is a canvas or pixel surface picture held lossy by clients now (not refined yet)? */
    public synchronized boolean lossyOut() {
        for (TStream st : streams.values()) if (st.lossyOut) return true;
        return false;
    }

    /** Would a pass now send a refine that is due (refine timers)? Nanoseconds until one is due, or -1. */
    public synchronized long refineDueInNanos(long nowUs) {
        long best = -1;
        for (TStream st : streams.values()) {
            if (!st.lossyOut || !st.pending.isEmpty()) continue;
            long due = Math.max(0, st.lastFrameUs + REFINE_AFTER_US - nowUs) * 1000 + 2_000_000;
            if (best < 0 || due < best) best = due;
        }
        return best;
    }

    public synchronized long headGeneration() {
        drain();
        return headGen;
    }
}
