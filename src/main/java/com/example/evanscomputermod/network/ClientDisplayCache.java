package com.example.evanscomputermod.network;

import com.example.evanscomputermod.computer.TerminalDisplay;

import org.jetbrains.annotations.Nullable;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Client side: what each computer's displays show, keyed by computer id and
 * display id. Delta packets land here whatever the computer is hosted in, so
 * the Terminal GUI, a Screen cluster and (later) a held device all read the
 * same cache, and a block moving (a Sable structure assembling) doesn't lose
 * its screen. Plain Java with no client classes, so common code and tests
 * can use it. Main client thread.
 */
public final class ClientDisplayCache {

    public record Key(UUID computerId, int displayId) {}

    /** One display as this client knows it. */
    public static final class Entry {
        public final TerminalDisplay display = new TerminalDisplay(1, 1);
        /** Whether a keyframe has arrived since the entry was (re)started: deltas need one as their base. */
        public boolean hasKeyframe;
        /**
         * Drawn by this client from a scene ({@code ClientUiScenes}, UI_PROTOCOL §14 phase 2a): tile
         * packets are ignored until the server ends the scene stream.
         */
        public volatile boolean sceneDriven;
    }

    private static final Map<Key, Entry> ENTRIES = new ConcurrentHashMap<>();

    /** The entry for a display, created empty on first use. */
    public static Entry entry(UUID computerId, int displayId) {
        return ENTRIES.computeIfAbsent(new Key(computerId, displayId), k -> new Entry());
    }

    /** The display if this client has heard of it, else null (renderers use this to skip drawing). */
    @Nullable
    public static TerminalDisplay peek(@Nullable UUID computerId, int displayId) {
        if (computerId == null) return null;
        Entry e = ENTRIES.get(new Key(computerId, displayId));
        return e != null ? e.display : null;
    }

    /** Apply one parsed packet. Returns false for a delta that arrived before any keyframe (dropped). */
    public static boolean apply(UUID computerId, int displayId, TerminalDeltaPacket.ParsedDelta delta) {
        Entry e = entry(computerId, displayId);
        if (e.sceneDriven) return false;
        if (delta.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
            DeltaApplier.applyKeyframe(e.display, delta);
            e.hasKeyframe = true;
            return true;
        }
        if (!e.hasKeyframe) return false;
        DeltaApplier.applyDelta(e.display, delta);
        return true;
    }

    /** The next delta needs a keyframe first (the host's view of this display was just set up). */
    public static void invalidate(UUID computerId, int displayId) {
        Entry e = ENTRIES.get(new Key(computerId, displayId));
        if (e != null) e.hasKeyframe = false;
        ClientPlayout.forget(computerId);
    }

    /** Forget everything (leaving a world). */
    public static void clear() {
        ENTRIES.clear();
        ClientPlayout.clear();
    }

    private ClientDisplayCache() {}
}
