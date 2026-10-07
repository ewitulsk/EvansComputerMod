package com.example.evanscomputermod;

import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Mod settings ({@code config/evanscomputermod-common.toml}). Read through the
 * static getters, which fall back to the defaults before the file is loaded
 * (unit tests, early startup).
 */
public final class EcmConfig {

    public static final ModConfigSpec SPEC;

    private static final ModConfigSpec.IntValue DISPLAY_DEFAULT_HZ;
    private static final ModConfigSpec.IntValue DISPLAY_MAX_HZ;
    private static final ModConfigSpec.IntValue CONTROLLER_RANGE;
    private static final ModConfigSpec.IntValue SPEAKER_RANGE;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();

        b.comment("Graphics displays (the Terminal's screen and Screen clusters).").push("display");
        DISPLAY_DEFAULT_HZ = b
                .comment("Refresh rate a display starts at, in Hz. Programs can ask for up to display.maxRefreshHz.")
                .defineInRange("defaultRefreshHz", 30, 1, 60);
        DISPLAY_MAX_HZ = b
                .comment("Highest refresh rate a program may set, in Hz. Each refresh can send a frame to every viewer,",
                        "so higher rates cost server bandwidth.")
                .defineInRange("maxRefreshHz", 60, 1, 60);
        b.pop();

        b.comment("Wireless Xbox Controller.").push("controller");
        CONTROLLER_RANGE = b
                .comment("How far from its computer a controller stays connected, in blocks (same dimension only).")
                .defineInRange("range", 64, 4, 512);
        b.pop();

        b.comment("Speaker block.").push("speaker");
        SPEAKER_RANGE = b
                .comment("How far a speaker can be heard, in blocks. Audio is only sent to players this close.")
                .defineInRange("range", 48, 8, 128);
        b.pop();

        SPEC = b.build();
    }

    private EcmConfig() {}

    private static int get(ModConfigSpec.IntValue v, int fallback) {
        try {
            return SPEC.isLoaded() ? v.get() : fallback;
        } catch (RuntimeException e) {
            return fallback;
        }
    }

    public static int displayDefaultRefreshHz() {
        return Math.min(get(DISPLAY_DEFAULT_HZ, 30), displayMaxRefreshHz());
    }

    public static int displayMaxRefreshHz() {
        return get(DISPLAY_MAX_HZ, 60);
    }

    public static int controllerRange() {
        return get(CONTROLLER_RANGE, 64);
    }

    public static int speakerRange() {
        return get(SPEAKER_RANGE, 48);
    }
}
