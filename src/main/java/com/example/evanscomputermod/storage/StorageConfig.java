package com.example.evanscomputermod.storage;

//? if <=1.21.1 {

import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Server config for item storage ({@code serverconfig/evanscomputermod-server.toml}).
 * Values are read through the getters, which fall back to the defaults
 * before the config has loaded.
 */
public final class StorageConfig {

    public static final ModConfigSpec SPEC;

    private static final ModConfigSpec.IntValue TOKEN_DEFAULT_TTL;
    private static final ModConfigSpec.IntValue TOKEN_MAX_TTL;
    private static final ModConfigSpec.IntValue MAX_TOKENS_PER_ISSUER;
    private static final ModConfigSpec.LongValue MAX_ITEMS_IN_FLIGHT;
    private static final ModConfigSpec.IntValue CELL_GC_DAYS;
    private static final ModConfigSpec.IntValue MAX_TYPE_BYTES;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        b.comment("Item storage (Storage Cells, drives, encoders, decoders and tokens)").push("storage");
        TOKEN_DEFAULT_TTL = b.comment("Seconds (game time) before an unspent token expires, when a program doesn't choose")
                .defineInRange("tokenDefaultTtlSeconds", 600, 1, 86_400);
        TOKEN_MAX_TTL = b.comment("Longest token lifetime a program may ask for, in seconds (game time)")
                .defineInRange("tokenMaxTtlSeconds", 3600, 1, 604_800);
        MAX_TOKENS_PER_ISSUER = b.comment("Most unspent tokens one drive or storage module may have out")
                .defineInRange("maxTokensPerIssuer", 64, 1, 100_000);
        MAX_ITEMS_IN_FLIGHT = b.comment("Most items one drive or module may have in unspent tokens plus lost & found")
                .defineInRange("maxItemsInFlightPerIssuer", 16_384L, 1L, Long.MAX_VALUE);
        CELL_GC_DAYS = b.comment("Delete a cell's contents after it hasn't been in a drive or module for this many",
                        "in-game days (it was probably destroyed, e.g. by /clear). 0 = never.")
                .defineInRange("cellGcDays", 365, 0, 1_000_000);
        MAX_TYPE_BYTES = b.comment("Largest encoded size of one item type (bytes of NBT); bigger stacks can't be encoded")
                .defineInRange("maxTypeBytes", 65_536, 256, 16_777_216);
        b.pop();
        SPEC = b.build();
    }

    private StorageConfig() {
    }

    private static int get(ModConfigSpec.IntValue v) {
        return SPEC.isLoaded() ? v.get() : v.getDefault();
    }

    public static long tokenDefaultTtlTicks() {
        return get(TOKEN_DEFAULT_TTL) * 20L;
    }

    public static long tokenMaxTtlTicks() {
        return get(TOKEN_MAX_TTL) * 20L;
    }

    public static int maxTokensPerIssuer() {
        return get(MAX_TOKENS_PER_ISSUER);
    }

    public static long maxItemsInFlight() {
        return SPEC.isLoaded() ? MAX_ITEMS_IN_FLIGHT.get() : MAX_ITEMS_IN_FLIGHT.getDefault();
    }

    /** Ticks of game time after which an unmounted cell is garbage-collected; 0 = never. */
    public static long cellGcTicks() {
        return get(CELL_GC_DAYS) * 24_000L;
    }

    public static int maxTypeBytes() {
        return get(MAX_TYPE_BYTES);
    }
}
//?}
