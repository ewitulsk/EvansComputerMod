package com.example.evanscomputermod.network;

import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Each player's own display settings ({@code config/evanscomputermod-display-client.toml}).
 * {@code display.playoutDelayMs}: how long timed frames (video) wait before they
 * are shown, DISPLAY_PIPELINE_SPEC.md Part E1. -1 (the default) adapts to the
 * connection's jitter; a fixed value trades smoothness against delay. The
 * server's {@code display.maxPlayoutMs} still caps it.
 */
public final class DisplayClientConfig {
    public static final ModConfigSpec SPEC;
    public static final ModConfigSpec.IntValue PLAYOUT_DELAY_MS;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        b.comment("Computer displays.").push("display");
        PLAYOUT_DELAY_MS = b.comment("Playout delay for timed frames (video) in ms; -1 = adaptive (2 x jitter + 40 ms,",
                        "60-400 ms). Untimed updates (text, menus, games) are never delayed.")
                .defineInRange("playoutDelayMs", -1, -1, 400);
        b.pop();
        SPEC = b.build();
    }

    /** Copy the setting to the playout buffer (on load and change). */
    public static void apply() {
        try {
            ClientPlayout.fixedDelayMs = SPEC.isLoaded() ? PLAYOUT_DELAY_MS.get() : -1;
        } catch (RuntimeException e) {
            ClientPlayout.fixedDelayMs = -1;
        }
    }

    private DisplayClientConfig() {}
}
