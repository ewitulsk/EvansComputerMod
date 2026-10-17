package com.example.evanscomputermod.computer.pipeline;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The lossy bandwidth budget of each player (DISPLAY_PIPELINE_SPEC.md Part
 * D2): {@code display.maxStreamKbps} per viewer, shared by every display that
 * sent that player a lossy frame in the last two seconds, so watching two
 * video walls doesn't cost twice the budget. Thread-safe.
 */
public final class StreamBudget {
    private StreamBudget() {}

    /** A stream counts as active this long after its last lossy frame (µs). */
    static final long ACTIVE_US = 2_000_000;

    private static final Map<UUID, Map<Object, Long>> ACTIVE = new ConcurrentHashMap<>();

    /** {@code stream} (a hub) sent {@code player} a lossy frame at {@code nowUs}. */
    public static void note(UUID player, Object stream, long nowUs) {
        ACTIVE.computeIfAbsent(player, k -> new ConcurrentHashMap<>()).put(stream, nowUs);
    }

    /** Lossy streams {@code player} watches now (at least 1). */
    public static int streams(UUID player, Object stream, long nowUs) {
        Map<Object, Long> m = ACTIVE.get(player);
        if (m == null) return 1;
        int n = 0;
        for (Map.Entry<Object, Long> e : m.entrySet()) {
            if (nowUs - e.getValue() > ACTIVE_US) m.remove(e.getKey());
            else if (e.getKey() != stream) n++;
        }
        return n + 1;
    }

    /** This stream's share of the player's budget, kbit/s. */
    public static double shareKbps(UUID player, Object stream, long nowUs) {
        return com.example.evanscomputermod.EcmConfig.displayMaxStreamKbps() / (double) streams(player, stream, nowUs);
    }

    /** The player left. */
    public static void forget(UUID player) {
        ACTIVE.remove(player);
    }
}
