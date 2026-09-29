package com.example.evanscomputermod.api.module;

import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.ItemStack;

/**
 * Implemented by items that can be installed in a computer's module bay
 * (right-click the computer with the item once it has an expansion card).
 * Only items implementing this can go in a bay.
 */
public interface IComputerModuleItem {

    /**
     * Create the live module for an installed stack. Server side only.
     *
     * @param savedState what the module last wrote in {@link IComputerModule#saveState}
     *                   (empty for a fresh module)
     */
    IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState);

    /**
     * How the module looks in its bay slot. The computer's block model has a
     * built-in cartridge for each {@link ModuleSlotVisual}; modules without
     * their own use {@link ModuleSlotVisual#GENERIC}.
     */
    default ModuleSlotVisual getSlotVisual(ItemStack stack) {
        return ModuleSlotVisual.GENERIC;
    }
}
