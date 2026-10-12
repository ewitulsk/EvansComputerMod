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
 * <p>Threads: steps arrive from the compositor's tick ({@link #offer}, any
 * thread); {@link #sync} and the acks run on the server thread. Keyframes
 * take the compositor's monitor, never while this hub's is held.
 */
public final class UiSceneHub {
    /** Steps kept for viewers that fell behind, and their total size. */
    static final int HISTORY_MAX = 64;
    static final long HISTORY_MAX_BYTES = 2L << 20;
    /** Unacknowledged steps a viewer may have in flight. */
    public static final int WINDOW = 32;
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
        long bytes, messages, keyframes, steps, resources;

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
        for (Viewer v : viewers.values()) {
            v.needsKeyframe = true;
            v.sentGen = -1;
            v.ackedGen = -1;
        }
    }

    /**
     * One pass for display {@code displayId}: {@code ids} are the players
     * who should render it client-side now. Others that did are sent END.
     * {@code exporter} supplies the current exporter for keyframes (null
     * when there is no scene, which ends everyone).
     */
    public void sync(int displayId, Collection<UUID> ids, Supplier<SceneExporter> exporter, Sink sink) {
        List<Object[]> out = new ArrayList<>();
        boolean wantKeyframe = false;
        SceneExporter ex = exporter.get();
        Set<UUID> want = new HashSet<>(ex == null ? List.of() : ids);
        synchronized (this) {
            drain();
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
            for (UUID id : want) {
                ViewerKey key = new ViewerKey(id, displayId);
                Viewer v = viewers.get(key);
                if (v == null) continue;
                List<SceneExporter.Step> chain = null;
                if (!v.needsKeyframe) {
                    if (v.sentGen >= headGen || v.sentGen - v.ackedGen >= WINDOW) continue;
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
                } else {
                    for (SceneExporter.Step s : chain) {
                        if (s.images() != null) resources(key, v, s.images(), ex, out);
                        out.add(new Object[]{key, s.data()});
                        v.steps++;
                    }
                    v.sentGen = headGen;
                }
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
    public synchronized boolean onAck(UUID player, int displayId, int ackEpoch, long gen, long[] missing) {
        Viewer v = viewers.get(new ViewerKey(player, displayId));
        if (v == null) return false;
        if (missing != null) for (long k : missing) UiClients.forget(player, k);
        if (gen <= 0) {
            v.needsKeyframe = true;
            return true;
        }
        if (ackEpoch != epoch) return false;
        boolean blocked = v.sentGen - v.ackedGen >= WINDOW;
        v.ackedGen = Math.max(v.ackedGen, gen);
        return blocked;
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

    public synchronized long headGeneration() {
        drain();
        return headGen;
    }
}
