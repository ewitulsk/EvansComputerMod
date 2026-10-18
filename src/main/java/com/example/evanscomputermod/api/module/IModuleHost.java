package com.example.evanscomputermod.api.module;

import com.example.evanscomputermod.radio.api.Pose;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;

import org.jetbrains.annotations.Nullable;

/**
 * The computer a module is installed in, as the module sees it. The computer
 * need not be a block (a held device carries modules too): modules locate
 * themselves with {@link #getPose()}, and only block-bound modules (wires,
 * Create links) use {@link #getBlockPos()}.
 */
public interface IModuleHost {

    ServerLevel getLevel();

    /**
     * Where the computer is in world space: dimension, position (projected
     * out of a Sable/Aeronautics sub-level when it is on one) and the
     * orientation of the structure it is on (identity elsewhere). The
     * screen's own direction is {@link #getFacing()}.
     */
    Pose getPose();

    /**
     * The computer's block position (inside the sub-level's plot when on a
     * Sable/Aeronautics structure), or null when the computer isn't a block.
     */
    @Nullable
    BlockPos getBlockPos();

    /** Direction the computer's screen faces, in the frame of the structure it is on. */
    Direction getFacing();

    /** Bay slot index, 0..3. */
    int getSlot();

    /** Name programs use for this module, e.g. {@code left_bay_1}. */
    String getSlotName();

    /** Whether the computer is still in the world. */
    boolean isAlive();

    /** The module's persistent state changed; the computer will save it. */
    void markDirty();

    /** Wireless controllers connected to the computer (receiver modules feed them), or null if it takes none. */
    @Nullable
    /**
     * A cellular modem in this host heard a page for its idle identity: wake the
     * host's computer if it sleeps (an ePhone in the hotbar or pocket). Any thread.
     */
    default void wakeForPage() {}

    default com.example.evanscomputermod.controller.WirelessControllerHub getControllers() {
        return null;
    }
}
