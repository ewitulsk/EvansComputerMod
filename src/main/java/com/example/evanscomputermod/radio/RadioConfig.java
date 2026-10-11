package com.example.evanscomputermod.radio;

import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Radio &amp; Wireless gameplay settings ({@code <world>/serverconfig/evanscomputermod-server.toml},
 * synced to clients). Static getters fall back to the defaults before the file
 * is loaded (unit tests, early startup), like {@code EcmConfig}.
 */
public final class RadioConfig {

    public static final ModConfigSpec SPEC;

    public enum Realism { ARCADE, REALISTIC, SIMULATION }

    public enum HazardLevel { OFF, EQUIPMENT, FULL }

    private static final ModConfigSpec.EnumValue<Realism> REALISM;
    private static final ModConfigSpec.DoubleValue HOP_COMPRESSION;
    private static final ModConfigSpec.IntValue RAYS_PER_TICK;
    private static final ModConfigSpec.DoubleValue SABLE_MOVE_METRES;
    private static final ModConfigSpec.DoubleValue SABLE_TURN_DEGREES;
    private static final ModConfigSpec.IntValue SABLE_MIN_RECOMPUTE_TICKS;
    private static final ModConfigSpec.DoubleValue WATTS_PER_FE_PER_TICK;
    private static final ModConfigSpec.BooleanValue BURNER_ENABLED;
    private static final ModConfigSpec.IntValue BURNER_FE_PER_TICK;
    private static final ModConfigSpec.EnumValue<HazardLevel> HAZARD_DEFAULT;
    private static final ModConfigSpec.BooleanValue LIGHTNING_DEFAULT;
    private static final ModConfigSpec.IntValue SDR_BASIC_RATE;
    private static final ModConfigSpec.IntValue SDR_STANDARD_RATE;
    private static final ModConfigSpec.IntValue SDR_ADVANCED_RATE;
    private static final ModConfigSpec.IntValue SDR_CHICORY_CAP;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();

        b.comment("Radio propagation.").push("propagation");
        REALISM = b.comment("arcade: forgiving fading and noise; realistic: default; simulation: adds oscillator drift, DC spike, IQ imbalance.")
                .defineEnum("realism", Realism.REALISTIC);
        HOP_COMPRESSION = b.comment("Divides real ionospheric hop distances (~2000-4000 km) so HF skywave lands 2-8k blocks away.")
                .defineInRange("hopCompression", 500.0, 1.0, 100000.0);
        RAYS_PER_TICK = b.comment("Voxel rays the link cache may trace per server tick (shared by every radio).")
                .defineInRange("raysPerTick", 4096, 64, 1 << 20);
        b.pop();

        b.comment("Radios on Sable sub-levels and Create Aeronautics airships.").push("sable");
        SABLE_MOVE_METRES = b.comment("A ship moving this far invalidates its radios' links.")
                .defineInRange("recomputeMetres", 0.5, 0.05, 64.0);
        SABLE_TURN_DEGREES = b.comment("A ship turning this far invalidates its radios' links.")
                .defineInRange("recomputeDegrees", 2.0, 0.1, 90.0);
        SABLE_MIN_RECOMPUTE_TICKS = b.comment("At most one link recompute per ship per this many ticks; in between, links interpolate.")
                .defineInRange("minRecomputeTicks", 4, 1, 200);
        b.pop();

        b.comment("FE power for radio hardware.").push("power");
        WATTS_PER_FE_PER_TICK = b.comment("Watts of DC input equal to 1 FE/t.")
                .defineInRange("wattsPerFePerTick", 5.0, 0.01, 10000.0);
        b.push("burnerGenerator");
        BURNER_ENABLED = b.comment("If false the Burner Generator has no recipe, is hidden from JEI/EMI and existing ones stop producing.")
                .define("enabled", true);
        BURNER_FE_PER_TICK = b.comment("Output while burning, FE/t.")
                .defineInRange("fePerTick", 40, 1, 100000);
        b.pop();
        b.pop();

        b.comment("Hazards. These are defaults for the gamerules of the same name.").push("hazards");
        HAZARD_DEFAULT = b.comment("off: warnings only; equipment: player-built radio parts can break; full: adds fire and RF exposure.")
                .defineEnum("level", HazardLevel.EQUIPMENT);
        LIGHTNING_DEFAULT = b.comment("Lightning strikes on ungrounded antennas destroy the radio down the coax.")
                .define("lightningDamage", true);
        b.pop();

        b.comment("Software-defined radio sample-rate caps, samples per second.").push("sdr");
        SDR_BASIC_RATE = b.defineInRange("basicMaxRate", 48_000, 8_000, 10_000_000);
        SDR_STANDARD_RATE = b.defineInRange("standardMaxRate", 250_000, 8_000, 10_000_000);
        SDR_ADVANCED_RATE = b.defineInRange("advancedMaxRate", 1_000_000, 8_000, 10_000_000);
        SDR_CHICORY_CAP = b.comment("Cap for every tier when computers run on Chicory (pure Java) instead of wasmtime.")
                .defineInRange("chicoryMaxRate", 250_000, 8_000, 10_000_000);
        b.pop();

        SPEC = b.build();
    }

    private RadioConfig() {}

    private static <T> T get(ModConfigSpec.ConfigValue<T> v, T fallback) {
        try {
            return SPEC.isLoaded() ? v.get() : fallback;
        } catch (RuntimeException e) {
            return fallback;
        }
    }

    public static Realism realism() { return get(REALISM, Realism.REALISTIC); }
    public static double hopCompression() { return get(HOP_COMPRESSION, 500.0); }
    public static int raysPerTick() { return get(RAYS_PER_TICK, 4096); }
    public static double sableRecomputeMetres() { return get(SABLE_MOVE_METRES, 0.5); }
    public static double sableRecomputeRadians() { return Math.toRadians(get(SABLE_TURN_DEGREES, 2.0)); }
    public static int sableMinRecomputeTicks() { return get(SABLE_MIN_RECOMPUTE_TICKS, 4); }
    public static double wattsPerFePerTick() { return get(WATTS_PER_FE_PER_TICK, 5.0); }
    public static boolean burnerGeneratorEnabled() {
        Boolean o = burnerOverride;
        return o != null ? o : get(BURNER_ENABLED, true);
    }

    /** Test hook: force the Burner Generator on/off (null = use the config). */
    private static volatile Boolean burnerOverride;

    public static void overrideBurnerGeneratorEnabled(Boolean enabled) {
        burnerOverride = enabled;
    }
    public static int burnerFePerTick() { return get(BURNER_FE_PER_TICK, 40); }
    public static HazardLevel hazardDefault() { return get(HAZARD_DEFAULT, HazardLevel.EQUIPMENT); }
    public static boolean lightningDefault() { return get(LIGHTNING_DEFAULT, true); }
    public static int sdrBasicRate() { return get(SDR_BASIC_RATE, 48_000); }
    public static int sdrStandardRate() { return get(SDR_STANDARD_RATE, 250_000); }
    public static int sdrAdvancedRate() { return get(SDR_ADVANCED_RATE, 1_000_000); }
    public static int sdrChicoryCap() { return get(SDR_CHICORY_CAP, 250_000); }

    /** FE per tick needed to supply {@code watts} of DC input. */
    public static double fePerTickFor(double watts) {
        return watts / wattsPerFePerTick();
    }
}
