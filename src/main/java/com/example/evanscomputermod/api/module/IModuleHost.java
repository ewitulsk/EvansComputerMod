package com.example.evanscomputermod.api.module;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;

/**
 * The computer a module is installed in, as the module sees it.
 */
public interface IModuleHost {

    ServerLevel getLevel();

    /** The computer's block position (inside the sub-level's plot when on a Sable/Aeronautics structure). */
    BlockPos getPos();

    /** Direction the computer's screen faces. */
    Direction getFacing();

    /** Bay slot index, 0..3. */
    int getSlot();

    /** Name programs use for this module, e.g. {@code left_bay_1}. */
    String getSlotName();

    /** Whether the computer block entity is still in the world. */
    boolean isAlive();

    /** The module's persistent state changed; the computer will save it. */
    void markDirty();
}
