package com.example.evanscomputermod.missile;

import net.minecraft.util.Mth;
import net.minecraft.world.level.Level;

/**
 * Warhead payload type. Each type defines what happens on missile impact.
 * Add new variants here to extend warhead variety without changing MissileEntity.
 */
public enum WarheadType {

    EXPLOSIVE;

    /** TNT explosion power at minimum yield (0.0). */
    private static final float MIN_POWER = 2.0f;
    /** TNT explosion power at maximum yield (1.0). */
    private static final float MAX_POWER = 10.0f;

    /**
     * Execute impact effect at the given world position.
     *
     * @param level     The server-side level.
     * @param x         Impact X (block-center precision).
     * @param y         Impact Y.
     * @param z         Impact Z.
     * @param yieldScale Configurable yield, clamped to [0.0, 1.0].
     */
    public void onImpact(Level level, double x, double y, double z, float yieldScale) {
        switch (this) {
            case EXPLOSIVE -> {
                float power = MIN_POWER + (MAX_POWER - MIN_POWER) * Mth.clamp(yieldScale, 0.0f, 1.0f);
                level.explode(null, x, y, z, power, true, Level.ExplosionInteraction.TNT);
            }
        }
    }

    public static WarheadType fromId(String id) {
        for (WarheadType t : values()) {
            if (t.name().equalsIgnoreCase(id)) return t;
        }
        return EXPLOSIVE;
    }
}
