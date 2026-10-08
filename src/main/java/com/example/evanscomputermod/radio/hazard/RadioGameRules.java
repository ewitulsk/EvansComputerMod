package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioConfig;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.Level;
import org.jetbrains.annotations.Nullable;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Hazard gamerules (spec, Hazards: multiplayer safety).
 * <ul>
 *   <li>{@code radioHazards}: -1 = the server config default
 *       ({@code hazards.level}), 0 = off (warnings only), 1 = equipment
 *       (player-built radio parts can break; no fire, no player damage),
 *       2 = full (adds arc fires and RF exposure damage).</li>
 *   <li>{@code radioLightningDamage}: lightning on an ungrounded antenna
 *       destroys the radio hardware down the coax (also needs the server
 *       config's {@code hazards.lightningDamage} and a hazard level above off).</li>
 * </ul>
 * Fire additionally respects {@code doFireTick}.
 *
 * <p>{@link #override} sets the level for a region instead (GameTests and
 * scenarios, which share one world and run side by side); a region override
 * wins over the gamerule there.
 */
public final class RadioGameRules {
    public static GameRules.Key<GameRules.IntegerValue> HAZARDS;
    public static GameRules.Key<GameRules.BooleanValue> LIGHTNING;

    /** A regional override: hazard level and lightning toggle within {@code radius} blocks of {@code center}. */
    public record Override(String dimension, BlockPos center, int radius, RadioConfig.HazardLevel level, boolean lightning) {
        boolean covers(Level l, BlockPos pos) {
            return l.dimension().location().toString().equals(dimension) && pos.closerThan(center, radius);
        }
    }

    private static final Map<BlockPos, Override> OVERRIDES = new ConcurrentHashMap<>();

    private RadioGameRules() {}

    public static void override(Level level, BlockPos center, int radius, RadioConfig.HazardLevel hazards, boolean lightning) {
        OVERRIDES.put(center.immutable(), new Override(level.dimension().location().toString(), center.immutable(), radius, hazards, lightning));
    }

    public static void clearOverride(BlockPos center) {
        OVERRIDES.remove(center);
    }

    @Nullable
    private static Override overrideAt(Level level, @Nullable BlockPos pos) {
        if (pos == null || OVERRIDES.isEmpty()) return null;
        for (Override o : OVERRIDES.values()) if (o.covers(level, pos)) return o;
        return null;
    }

    /** The hazard level in force at {@code pos}: a regional override, else the gamerule. */
    public static RadioConfig.HazardLevel level(Level level, @Nullable BlockPos pos) {
        Override o = overrideAt(level, pos);
        return o != null ? o.level : level(level);
    }

    public static boolean lightningDamage(Level level, @Nullable BlockPos pos) {
        Override o = overrideAt(level, pos);
        if (o != null) return o.lightning && o.level != RadioConfig.HazardLevel.OFF;
        return lightningDamage(level);
    }

    static synchronized void register() {
        if (HAZARDS != null) return;
        HAZARDS = GameRules.register("radioHazards", GameRules.Category.MISC, GameRules.IntegerValue.create(-1));
        LIGHTNING = GameRules.register("radioLightningDamage", GameRules.Category.MISC, GameRules.BooleanValue.create(true));
    }

    /** The hazard level the gamerule sets in {@code level}. */
    public static RadioConfig.HazardLevel level(Level level) {
        int v = HAZARDS == null ? -1 : level.getGameRules().getInt(HAZARDS);
        if (v < 0 || v >= RadioConfig.HazardLevel.values().length) return RadioConfig.hazardDefault();
        return RadioConfig.HazardLevel.values()[v];
    }

    public static boolean lightningDamage(Level level) {
        return RadioConfig.lightningDefault() && (LIGHTNING == null || level.getGameRules().getBoolean(LIGHTNING))
                && level(level) != RadioConfig.HazardLevel.OFF;
    }

    /** Sets {@code radioHazards} (tests, scenarios); -1 restores the config default. */
    public static void set(Level level, int value) {
        if (level.getServer() != null) level.getGameRules().getRule(HAZARDS).set(value, level.getServer());
    }

    public static void setLightning(Level level, boolean value) {
        if (level.getServer() != null) level.getGameRules().getRule(LIGHTNING).set(value, level.getServer());
    }
}
//?}
