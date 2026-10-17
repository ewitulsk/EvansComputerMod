package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.proto.UiProtoException;
import com.example.evanscomputermod.network.ClientDisplayCache;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Client side: the scene mirrors this client renders itself (§14 phase 2a),
 * keyed by computer UUID and display id like {@link ClientDisplayCache},
 * plus the image cache shared by all of them (content keys, kept across
 * worlds for the whole game session and reported to servers in the hello).
 *
 * <p>Each mirror draws into its display's {@link ClientDisplayCache} entry,
 * so everything that shows a computer display (the Terminal screen, a
 * Screen wall, later the ePhone) picks the pixels up unchanged, and only
 * changed 16x16 tiles get uploaded. While a mirror drives an entry, tile
 * deltas for it are ignored; {@link UiSceneCodec#END} hands it back.
 *
 * <p>Plain Java (tests use it without a game). Client main thread.
 */
public final class ClientUiScenes {
    private ClientUiScenes() {}

    public record Key(UUID computerId, int displayId) {}

    /** What to tell the server after a message: {@code generation <= 0} asks for a keyframe. */
    public record Ack(int epoch, long generation, long[] missing) {}

    /** Image cache budget (premultiplied pixels, 4 bytes each). Images in use are never evicted. */
    public static final long IMAGE_CACHE_BYTES = 256L << 20;
    /** Keyframe requests per mirror are at least this far apart. */
    static final long KEYFRAME_RETRY_MS = 1000;

    private static final Map<Key, UiSceneMirror> MIRRORS = new ConcurrentHashMap<>();
    private static final Map<Key, Long> LAST_KEYFRAME_REQUEST = new ConcurrentHashMap<>();
    private static final LinkedHashMap<Long, int[]> IMAGES = new LinkedHashMap<>(64, 0.75f, true);
    private static long imageBytes;
    private static long messages, bytes;

    /** The mirror of a display this client renders itself, or null (console mode, tile path, never seen). */
    public static UiSceneMirror get(UUID computerId, int displayId) {
        return computerId == null ? null : MIRRORS.get(new Key(computerId, displayId));
    }

    /** Is this display drawn from a scene here (its pixels come from the mirror, not tile deltas)? */
    public static boolean active(UUID computerId, int displayId) {
        UiSceneMirror m = get(computerId, displayId);
        return m != null && m.ready();
    }

    /**
     * Handle one scene message for a display at client time {@code nowMs}.
     * Returns the ack to send, or null for none.
     */
    public static Ack onMessage(UUID computerId, int displayId, byte[] data, long nowMs) {
        UiSceneCodec.Message m;
        try {
            m = UiSceneCodec.unpack(data);
        } catch (UiProtoException e) {
            m = null;
        }
        return onMessage(computerId, displayId, m, data.length, nowMs);
    }

    /**
     * {@link #onMessage(UUID, int, byte[], long)} for a message already
     * unpacked off the render thread ({@code m == null}: it didn't decode).
     */
    public static Ack onMessage(UUID computerId, int displayId, UiSceneCodec.Message m, int size, long nowMs) {
        return onMessage(computerId, displayId, m, size, nowMs, com.example.evanscomputermod.network.ClockSync.clientNowUs());
    }

    /** {@link #onMessage(UUID, int, UiSceneCodec.Message, int, long)} with the time the message arrived (µs). */
    public static Ack onMessage(UUID computerId, int displayId, UiSceneCodec.Message m, int size, long nowMs, long recvUs) {
        messages++;
        bytes += size;
        Key key = new Key(computerId, displayId);
        if (m == null) {
            UiSceneMirror mm = MIRRORS.get(key);
            return keyframeRequest(key, mm == null ? -1 : mm.epoch(), nowMs);
        }
        switch (m.kind()) {
            case UiSceneCodec.TIMED, UiSceneCodec.TFLUSH -> {
                UiSceneMirror mirror = MIRRORS.get(key);
                if (mirror == null || !mirror.ready() || (m.epoch() != mirror.epoch())) return null;
                UiSceneCodec.Timed t = m.timed();
                long due = 0;
                if (m.kind() == UiSceneCodec.TIMED && t.ptsUs() != 0) {
                    due = com.example.evanscomputermod.network.ClientPlayout.timedDue(computerId, t.ptsUs(), t.sendUs(), recvUs);
                }
                if (!mirror.timed(m, due)) return keyframeRequest(key, mirror.epoch(), nowMs);
                return null; // acked on receipt by the caller (timed acks carry their own sequence)
            }
            case UiSceneCodec.RESOURCE -> {
                UiSceneCodec.Resource r = m.resource();
                putImage(r.key(), r.argb());
                for (UiSceneMirror mm : MIRRORS.values()) mm.imageArrived(r.key(), r.argb(), nowMs);
                return null;
            }
            case UiSceneCodec.END -> {
                MIRRORS.remove(key);
                LAST_KEYFRAME_REQUEST.remove(key);
                ClientDisplayCache.Entry e = ClientDisplayCache.entry(computerId, displayId);
                e.sceneDriven = false;
                ClientDisplayCache.invalidate(computerId, displayId);
                return null;
            }
            default -> {
                UiSceneMirror mirror = MIRRORS.computeIfAbsent(key, k -> new UiSceneMirror(ClientUiScenes::image));
                UiSceneMirror.Result res = mirror.apply(m, nowMs);
                if (res == UiSceneMirror.Result.NEED_KEYFRAME) return keyframeRequest(key, mirror.epoch(), nowMs);
                if (res == UiSceneMirror.Result.IGNORED) return null;
                ClientDisplayCache.entry(computerId, displayId).sceneDriven = true;
                if (!mirror.missing().isEmpty()) {
                    long[] miss = mirror.missing().stream().mapToLong(Long::longValue).toArray();
                    mirror.missing().clear();
                    Ack a = keyframeRequest(key, mirror.epoch(), nowMs);
                    return a == null ? null : new Ack(a.epoch(), 0, miss);
                }
                return new Ack(mirror.epoch(), mirror.generation(), null);
            }
        }
    }

    private static Ack keyframeRequest(Key key, int epoch, long nowMs) {
        Long last = LAST_KEYFRAME_REQUEST.get(key);
        if (last != null && nowMs - last < KEYFRAME_RETRY_MS) return null;
        LAST_KEYFRAME_REQUEST.put(key, nowMs);
        return new Ack(epoch, 0, null);
    }

    /**
     * Draw every mirror that changed (or is animating) at client time
     * {@code nowMs} into its display's cache entry.
     */
    public static void tick(long nowMs) {
        long nowUs = com.example.evanscomputermod.network.ClockSync.clientNowUs();
        for (Map.Entry<Key, UiSceneMirror> e : MIRRORS.entrySet()) {
            UiSceneMirror m = e.getValue();
            if (!m.ready()) continue;
            int shown = m.tickTimed(nowUs, nowMs);
            if (shown > 0) com.example.evanscomputermod.network.ClientPlayout.noteShown(e.getKey().computerId(), shown);
            List<Box> boxes = m.render(nowMs);
            if (boxes.isEmpty()) continue;
            TerminalDisplay td = ClientDisplayCache.entry(e.getKey().computerId(), e.getKey().displayId()).display;
            m.scanOut(td, boxes);
        }
    }

    /** Leaving a world: scenes go, images stay (they're addressed by content). */
    public static void clearScenes() {
        MIRRORS.clear();
        LAST_KEYFRAME_REQUEST.clear();
        STREAMS.clear();
    }

    // =========================================================== short stream ids

    private static final Map<Integer, Key> STREAMS = new ConcurrentHashMap<>();

    /** A fully addressed message named its stream: later steps come with the stream id only. */
    public static void bindStream(int stream, UUID computerId, int displayId) {
        STREAMS.put(stream, new Key(computerId, displayId));
    }

    /** The display a stream id stands for, or null if no message bound it yet. */
    public static Key stream(int stream) {
        return STREAMS.get(stream);
    }

    // =========================================================== image cache

    static synchronized int[] image(long key) {
        return IMAGES.get(key);
    }

    static synchronized void putImage(long key, int[] argb) {
        int[] old = IMAGES.put(key, argb);
        if (old != null) imageBytes -= old.length * 4L;
        imageBytes += argb.length * 4L;
        if (imageBytes <= IMAGE_CACHE_BYTES) return;
        // Evict least recently used images no mirror uses.
        java.util.Set<Long> inUse = new java.util.HashSet<>();
        for (UiSceneMirror m : MIRRORS.values()) inUse.addAll(m.imageKeys());
        var it = IMAGES.entrySet().iterator();
        while (imageBytes > IMAGE_CACHE_BYTES && it.hasNext()) {
            var e = it.next();
            if (inUse.contains(e.getKey()) || e.getKey() == key) continue;
            imageBytes -= e.getValue().length * 4L;
            it.remove();
        }
    }

    /** Image keys held, for the hello (most recently used first, at most {@link UiClients#MAX_HELD}). */
    public static synchronized long[] heldImageKeys() {
        List<Long> keys = new ArrayList<>(IMAGES.keySet());
        java.util.Collections.reverse(keys);
        return keys.stream().limit(UiClients.MAX_HELD).mapToLong(Long::longValue).toArray();
    }

    /** Scene messages and their bytes received so far (diagnostics). */
    public static long messagesReceived() {
        return messages;
    }

    public static long bytesReceived() {
        return bytes;
    }

    /** Tests: forget everything, images included. */
    public static synchronized void clearAll() {
        clearScenes();
        IMAGES.clear();
        imageBytes = 0;
        messages = 0;
        bytes = 0;
    }
}
