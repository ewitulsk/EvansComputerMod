package com.example.evanscomputermod.module;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Item data components added by the mod.
 */
public final class ModDataComponents {

    public static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /** Expansion cards and modules of a terminal item (kept when the block is broken). */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<InstalledModules>> INSTALLED_MODULES =
            COMPONENTS.register("installed_modules", () -> DataComponentType.<InstalledModules>builder()
                    .persistent(InstalledModules.CODEC)
                    .networkSynchronized(InstalledModules.STREAM_CODEC)
                    .build());

    private ModDataComponents() {
    }
}
