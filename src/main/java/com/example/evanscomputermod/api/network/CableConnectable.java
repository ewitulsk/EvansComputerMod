package com.example.evanscomputermod.api.network;

import net.minecraft.core.Direction;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Marker for blocks that network cables connect to (and draw an arm towards),
 * such as the Wi-Fi Access Point or a microwave radio: devices that join the
 * cable segment as a bridge port.
 */
public interface CableConnectable {
    /**
     * True if a network cable may plug into {@code face} of {@code state}: the
     * faces where the model draws a network port. A cable draws no arm to, and
     * the device does not join a cable on, any other face.
     */
    default boolean acceptsCable(BlockState state, Direction face) {
        return true;
    }
}
