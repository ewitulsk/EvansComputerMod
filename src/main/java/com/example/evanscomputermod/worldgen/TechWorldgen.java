package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;

import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.feature.configurations.NoneFeatureConfiguration;
import net.minecraft.world.level.levelgen.structure.StructureType;
import net.minecraft.world.level.levelgen.structure.pieces.StructurePieceType;
import net.minecraft.world.level.levelgen.structure.placement.StructurePlacementType;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureProcessorType;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.*;

public final class TechWorldgen {
    private static final DeferredRegister<StructurePlacementType<?>> PLACEMENTS =
            DeferredRegister.create(Registries.STRUCTURE_PLACEMENT, EvansComputerMod.MODID);
    private static final DeferredRegister<StructureProcessorType<?>> PROCESSORS =
            DeferredRegister.create(Registries.STRUCTURE_PROCESSOR, EvansComputerMod.MODID);
    private static final DeferredRegister<StructureType<?>> STRUCTURES =
            DeferredRegister.create(Registries.STRUCTURE_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<StructurePieceType> PIECES =
            DeferredRegister.create(Registries.STRUCTURE_PIECE, EvansComputerMod.MODID);
    private static final DeferredRegister<Feature<?>> FEATURES =
            DeferredRegister.create(Registries.FEATURE, EvansComputerMod.MODID);

    public static final DeferredHolder<StructurePlacementType<?>, StructurePlacementType<TechVillagePlacement>>
            PLACEMENT = PLACEMENTS.register("tech_ring", () -> () -> TechVillagePlacement.CODEC);
    public static final DeferredHolder<StructureProcessorType<?>, StructureProcessorType<ProvisioningProcessor>>
            PROCESSOR = PROCESSORS.register("provision", () -> () -> ProvisioningProcessor.CODEC);
    public static final DeferredHolder<StructureType<?>, StructureType<TechVillageStructure>>
            STRUCTURE = STRUCTURES.register("tech_village", () -> () -> TechVillageStructure.CODEC);
    public static final DeferredHolder<StructurePieceType, StructurePieceType>
            NETWORK_PIECE = PIECES.register("village_network",
                    () -> (StructurePieceType.ContextlessType) TechNetworkPiece::new);
    public static final DeferredHolder<Feature<?>, Feature<NoneFeatureConfiguration>>
            FIBER_LINE = FEATURES.register("fiber_line", FiberLineFeature::new);

    public static void register(IEventBus bus) {
        PLACEMENTS.register(bus);
        PROCESSORS.register(bus);
        STRUCTURES.register(bus);
        PIECES.register(bus);
        FEATURES.register(bus);
    }
}
//?}
