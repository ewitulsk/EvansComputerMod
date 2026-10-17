package com.example.evanscomputermod.upload;

import net.neoforged.neoforge.common.ModConfigSpec;

/** Server settings ({@code config/evanscomputermod_upload-server.toml}, per world on a server). */
public final class UploadConfig {
    public static final ModConfigSpec SPEC;
    private static final ModConfigSpec.BooleanValue ENABLED;
    private static final ModConfigSpec.BooleanValue OPERATORS_ONLY;
    private static final ModConfigSpec.IntValue MAX_FILE_KIB;
    private static final ModConfigSpec.IntValue MAX_COMPUTER_MIB;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        ENABLED = b.comment("Let players upload files from their own computers into computers in the game.")
                .define("enabled", true);
        OPERATORS_ONLY = b.comment("Only operators (permission level 2) may upload.")
                .define("operatorsOnly", false);
        MAX_FILE_KIB = b.comment("The largest file a player may upload, in KiB (16384 = 16 MiB).")
                .defineInRange("maxFileSizeKiB", 16_384, 1, 1_048_576);
        MAX_COMPUTER_MIB = b.comment("An upload is refused when it would take a computer's files over this many MiB. 0 = no limit.")
                .defineInRange("maxComputerStorageMiB", 256, 0, 1_048_576);
        SPEC = b.build();
    }

    private UploadConfig() {}

    private static <T> T get(ModConfigSpec.ConfigValue<T> v, T fallback) {
        return SPEC.isLoaded() ? v.get() : fallback;
    }

    public static boolean enabled() {
        return get(ENABLED, true);
    }

    public static boolean operatorsOnly() {
        return get(OPERATORS_ONLY, false);
    }

    public static long maxFileBytes() {
        return get(MAX_FILE_KIB, 16_384) * 1024L;
    }

    public static long maxComputerBytes() {
        return get(MAX_COMPUTER_MIB, 256) * 1024L * 1024L;
    }
}
