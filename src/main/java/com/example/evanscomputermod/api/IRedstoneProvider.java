package com.example.evanscomputermod.api;

import net.minecraft.core.Direction;

/**
 * Capability for hosts that can interact with redstone.
 * Only relevant for blocks or entities that have a physical presence in the world.
 */
public interface IRedstoneProvider {

    /**
     * Sets the redstone output power for a specific absolute side.
     * @param absoluteSide The Direction ordinal (0=DOWN, 1=UP, 2=NORTH, 3=SOUTH, 4=WEST, 5=EAST)
     * @param power The power level (0-15)
     */
    void setRedstoneOutput(int absoluteSide, int power);

    /**
     * Gets the redstone input power for a specific absolute side.
     * @param absoluteSide The Direction ordinal
     * @return The power level (0-15)
     */
    int getRedstoneInput(int absoluteSide);

    /**
     * Converts a relative side (FRONT/BACK/LEFT/RIGHT/UP/DOWN) to an
     * absolute Minecraft Direction based on the host's facing.
     * @param relativeSide The relative side (0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT)
     * @return The absolute Direction
     */
    Direction relativeToAbsolute(int relativeSide);
}
