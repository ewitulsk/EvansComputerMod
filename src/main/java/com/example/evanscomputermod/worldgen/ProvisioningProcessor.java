package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.*;
import com.mojang.serialization.MapCodec;

import net.minecraft.core.BlockPos;
import net.minecraft.nbt.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.*;

/**
 * Templates contain roles only. UUIDs and startup files come from SavedData; the ISP's
 * sign gets its village number, AS and prefixes.
 */
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
                || !(current.nbt().contains("ecmRole") || current.nbt().contains("ecmSign"))
                || !(reader instanceof ServerLevelAccessor accessor)) return current;
        var level = accessor.getLevel();
        var data = WorldNetwork.get(level);
        data.plan(level);
        int village = data.nearest(current.pos()).number();
        CompoundTag tag = current.nbt().copy();
        if (tag.contains("ecmSign")) {
            tag.remove("ecmSign");
            ListTag lines = new ListTag();
            for (String line : new String[] {"Tech Village " + village, "ISP  AS " + (65000 + village),
                    "100." + (64 + village) + ".0.0/23", "fiber: mast top"})
                lines.add(StringTag.valueOf("{\"text\":\"" + line + "\"}"));
            CompoundTag front = new CompoundTag();
            front.put("messages", lines);
            front.putString("color", "black");
            front.putBoolean("has_glowing_text", false);
            tag.put("front_text", front);
            return new StructureTemplate.StructureBlockInfo(current.pos(), current.state(), tag);
        }
        String role = tag.getString("ecmRole");
        data.provision(level, village, role);
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
