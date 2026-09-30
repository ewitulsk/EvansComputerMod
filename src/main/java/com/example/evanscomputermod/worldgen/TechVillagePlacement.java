package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.WorldNetwork;
import com.mojang.serialization.MapCodec;

import net.minecraft.core.Vec3i;
import net.minecraft.world.level.chunk.ChunkGeneratorStructureState;
import net.minecraft.world.level.levelgen.structure.placement.*;

import java.util.Optional;

/** Exactly ten persisted ring sites, selected without loading distant chunks. */
public final class TechVillagePlacement extends StructurePlacement {
    public static final MapCodec<TechVillagePlacement> CODEC =
            MapCodec.unit(TechVillagePlacement::new);

    public TechVillagePlacement() {
        super(Vec3i.ZERO, FrequencyReductionMethod.DEFAULT, 1, 65001, Optional.empty());
    }

    protected boolean isPlacementChunk(ChunkGeneratorStructureState state, int x, int z) {
        return WorldNetwork.SITES.getOrDefault(state.getLevelSeed(), java.util.List.of()).stream()
                .anyMatch(p -> (p.getX() >> 4) == x && (p.getZ() >> 4) == z);
    }

    public StructurePlacementType<?> type() {
        return TechWorldgen.PLACEMENT.get();
    }
}
//?}
