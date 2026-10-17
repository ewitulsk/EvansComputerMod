package com.example.evanscomputermod.api.module;

import net.minecraft.util.StringRepresentable;

import java.util.Locale;

/**
 * What a computer's module bay slot shows. Each value is a block state of the
 * computer with its own cartridge model, so the slot renders as part of the
 * block (including on Sable / Create Aeronautics structures).
 */
public enum ModuleSlotVisual implements StringRepresentable {
    EMPTY,
    GENERIC,
    REDSTONE_LINK,
    /** Wired Sensor Module: a cartridge with a wire connector (1.21.1). */
    WIRED_SENSOR;
    // Every value here multiplies the Terminal's block states by (n+1)^4/n^4 (four slot properties): two more
    // visuals took the multipart from ~20k to ~100k states and the client out of a 2 GB heap while baking.
    // Modules without a strong visual reason use GENERIC.

    @Override
    public String getSerializedName() {
        return name().toLowerCase(Locale.ROOT);
    }
}
