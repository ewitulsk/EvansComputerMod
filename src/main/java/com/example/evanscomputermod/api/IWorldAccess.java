package com.example.evanscomputermod.api;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;

import org.jspecify.annotations.Nullable;

/**
 * Capability for hosts that exist in the Minecraft world and have a position.
 * Used for peripheral discovery and other position-dependent features.
 */
public interface IWorldAccess {

    @Nullable
    Level getLevel();

    BlockPos getBlockPos();
}
