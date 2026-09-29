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
    REDSTONE_LINK;

    @Override
    public String getSerializedName() {
        return name().toLowerCase(Locale.ROOT);
    }
}
