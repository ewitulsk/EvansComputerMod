package com.example.evanscomputermod.missile;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;

/**
 * Abstraction for any structure capable of holding and launching a missile.
 * Single-block launchers implement this directly; future multi-block silo
 * controllers would implement the same interface.
 */
public interface IMissileMount {

    /** Compass bearing in degrees (0 = North, 90 = East, 180 = South, 270 = West). */
    float getBearing();

    /** Launch elevation angle in degrees above horizontal (5–85). */
    float getElevation();

    /** The tier of the currently loaded missile, or {@code null} if empty. */
    MissileTier getLoadedTier();

    /** True if a missile is loaded and ready to fire. */
    boolean isLoaded();

    /**
     * Called immediately after a missile is spawned.
     * Implementations should consume the loaded missile and mark the mount empty.
     */
    void onLaunch();

    /** World block position of this mount. */
    BlockPos getMountPos();

    /** The level this mount exists in. */
    Level getMountLevel();
}
