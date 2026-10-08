package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.*;
import com.mojang.serialization.MapCodec;

import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.*;

/** Templates contain roles only. UUIDs and startup files come from SavedData. */
public final class ProvisioningProcessor extends StructureProcessor {
    public static final MapCodec<ProvisioningProcessor> CODEC =
            MapCodec.unit(ProvisioningProcessor::new);

    @Override
    public StructureTemplate.StructureBlockInfo processBlock(
            LevelReader reader,
            BlockPos origin,
            BlockPos pivot,
            StructureTemplate.StructureBlockInfo original,
            StructureTemplate.StructureBlockInfo current,
            StructurePlaceSettings settings) {
        if (current.nbt() == null
                || !current.nbt().contains("ecmRole")
                || !(reader instanceof ServerLevelAccessor accessor)) return current;
        var level = accessor.getLevel();
        var data = WorldNetwork.get(level);
        data.plan(level);
        int village = data.nearest(current.pos()).number();
        String role = current.nbt().getString("ecmRole");
        data.provision(level, village, role);
        CompoundTag tag = current.nbt().copy();
        tag.remove("ecmRole");
        tag.putUUID("computerId", data.identity(level, village, role));
        tag.putBoolean("wasRunning", true);
        tag.putString("wasmModule", "terminal_os.wasm");
        return new StructureTemplate.StructureBlockInfo(current.pos(), current.state(), tag);
    }

    protected StructureProcessorType<?> getType() {
        return TechWorldgen.PROCESSOR.get();
    }
}
//?}
