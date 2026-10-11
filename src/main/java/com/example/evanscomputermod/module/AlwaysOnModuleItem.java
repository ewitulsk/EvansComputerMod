package com.example.evanscomputermod.module;

import com.example.evanscomputermod.api.module.*;
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;

import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.*;

/** Presence in a bay enables server-owned execution after chunk unload. */
public final class AlwaysOnModuleItem extends Item implements IComputerModuleItem {
    public AlwaysOnModuleItem(Properties p) {
        super(p);
    }

    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag state) {
        return new Module();
    }

    private static final class Module extends AnnotatedPeripheral implements IComputerModule {
        public String getType() {
            return "always_on";
        }

        public void saveState(CompoundTag tag) {}
    }
}
