package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioConfig;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.Level;

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
 */
public final class RadioGameRules {
    public static GameRules.Key<GameRules.IntegerValue> HAZARDS;
    public static GameRules.Key<GameRules.BooleanValue> LIGHTNING;

    private RadioGameRules() {}

    static synchronized void register() {
        if (HAZARDS != null) return;
        HAZARDS = GameRules.register("radioHazards", GameRules.Category.MISC, GameRules.IntegerValue.create(-1));
        LIGHTNING = GameRules.register("radioLightningDamage", GameRules.Category.MISC, GameRules.BooleanValue.create(true));
    }

    /** The hazard level in force in {@code level}. */
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
