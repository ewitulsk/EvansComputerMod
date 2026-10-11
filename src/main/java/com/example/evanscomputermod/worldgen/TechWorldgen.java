package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;

import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.levelgen.structure.placement.StructurePlacementType;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureProcessorType;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.*;

public final class TechWorldgen {
    private static final DeferredRegister<StructurePlacementType<?>> PLACEMENTS =
            DeferredRegister.create(Registries.STRUCTURE_PLACEMENT, EvansComputerMod.MODID);
    private static final DeferredRegister<StructureProcessorType<?>> PROCESSORS =
            DeferredRegister.create(Registries.STRUCTURE_PROCESSOR, EvansComputerMod.MODID);
    public static final DeferredHolder<
                    StructurePlacementType<?>, StructurePlacementType<TechVillagePlacement>>
            PLACEMENT = PLACEMENTS.register("tech_ring", () -> () -> TechVillagePlacement.CODEC);
    public static final DeferredHolder<
                    StructureProcessorType<?>, StructureProcessorType<ProvisioningProcessor>>
            PROCESSOR = PROCESSORS.register("provision", () -> () -> ProvisioningProcessor.CODEC);

    public static void register(IEventBus bus) {
        PLACEMENTS.register(bus);
        PROCESSORS.register(bus);
    }
}
//?}
