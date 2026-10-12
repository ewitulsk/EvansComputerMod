package com.example.evanscomputermod.computer.ui;

import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Server side: which players' clients can draw scenes themselves (they sent
 * a hello with a matching {@link UiSceneCodec#VERSION}), and which image
 * keys each one holds, so image data goes to a client at most once
 * (§6 "Resources", §14). A player who never said hello (or a version that
 * doesn't match) keeps getting tile deltas: the per-client fallback.
 */
public final class UiClients {
    private UiClients() {}

    /** Most image keys a hello may report. */
    public static final int MAX_HELD = 4096;

    private record State(int version, Set<Long> held, Map<StreamKey, Integer> streams,
                         java.util.concurrent.atomic.AtomicInteger nextStream) {}

    private record StreamKey(UUID computer, int displayId) {}

    private static final Map<UUID, State> CLIENTS = new ConcurrentHashMap<>();

    /** A client said hello: its scene stream version and the image keys it already has. */
    public static void hello(UUID player, int version, long[] held) {
        Set<Long> set = ConcurrentHashMap.newKeySet();
        if (held != null) {
            for (int i = 0; i < Math.min(held.length, MAX_HELD); i++) set.add(held[i]);
        }
        CLIENTS.put(player, new State(version, set, new ConcurrentHashMap<>(), new java.util.concurrent.atomic.AtomicInteger(1)));
    }

    /** The player left. */
    public static void forget(UUID player) {
        CLIENTS.remove(player);
    }

    /** Can this player's client render scenes (the right stream version)? */
    public static boolean canRender(UUID player) {
        State s = CLIENTS.get(player);
        return s != null && s.version == UiSceneCodec.VERSION;
    }

    /** Record that the client gets image {@code key} now; false if it already has it. */
    static boolean markSent(UUID player, long key) {
        State s = CLIENTS.get(player);
        if (s == null) return true;
        return s.held.add(key);
    }

    /** The client doesn't hold {@code key} after all (it evicted it). */
    static void forget(UUID player, long key) {
        State s = CLIENTS.get(player);
        if (s != null) s.held.remove(key);
    }

    /**
     * The short id of a player's stream for one computer display: steps carry
     * it instead of the UUID and display id (fully addressed messages bind it
     * on the client first). 0 for a player without a hello.
     */
    public static int stream(UUID player, UUID computer, int displayId) {
        State s = CLIENTS.get(player);
        if (s == null) return 0;
        return s.streams.computeIfAbsent(new StreamKey(computer, displayId), k -> s.nextStream.getAndIncrement());
    }

    public static boolean holds(UUID player, long key) {
        State s = CLIENTS.get(player);
        return s != null && s.held.contains(key);
    }

    /** Tests: forget every client. */
    public static void clear() {
        CLIENTS.clear();
    }
}
