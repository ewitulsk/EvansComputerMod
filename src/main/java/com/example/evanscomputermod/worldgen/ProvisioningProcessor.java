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
 * sign gets its village number, AS and prefixes. The Phone Store's chest gets the village's
 * SIM cards and its display frames their ePhones, all provisioned in the carrier core
 * ({@link CellPlan}, {@link PhoneStore}).
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
                || !(current.nbt().contains("ecmRole") || current.nbt().contains("ecmSign") || current.nbt().contains(PhoneStore.CHEST_MARKER))
                || !(reader instanceof ServerLevelAccessor accessor)) return current;
        var level = accessor.getLevel();
        var data = WorldNetwork.get(level);
        data.plan(level);
        int village = data.nearest(current.pos()).number();
        int chat = data.chatVillage();
        CellPlan cells = data.cells(level);
        CompoundTag tag = current.nbt().copy();
        if (tag.contains(PhoneStore.CHEST_MARKER)) {
            tag.remove(PhoneStore.CHEST_MARKER);
            tag.put("Items", PhoneStore.chestItems(cells, village, level.registryAccess()));
            return new StructureTemplate.StructureBlockInfo(current.pos(), current.state(), tag);
        }
        if (tag.contains("ecmSign")) {
            String kind = tag.getString("ecmSign");
            tag.remove("ecmSign");
            String[] text = switch (kind) {
                case "datacenter" -> new String[] {"DATA CENTER", "Tech Village " + village,
                        "web 100." + (64 + village) + ".0.10",
                        village == chat ? "chat " + WorldNetwork.chatAddress(chat) : "chat: see website"};
                case "rack" -> new String[] {"Free racks: put a", "computer here, its", "top face (eth1)",
                        "is on the LAN (DHCP)"};
                case "store" -> new String[] {"PHONE STORE", cells.displayName(), "Tech Village " + village,
                        "phones + SIM cards"};
                case "sims" -> new String[] {"FREE SIM CARDS", cells.displayName() + " LTE", "put one in your",
                        "phone SIM tray"};
                case "site" -> new String[] {"CELL SITE", CellPlan.storeSiteId(village), "LTE channel " + cells.channel,
                        "core " + cells.coreAddress()};
                default -> new String[] {"Tech Village " + village, "ISP  AS " + (65000 + village),
                        "100." + (64 + village) + ".0.0/23", "fiber: mast top"};
            };
            ListTag lines = new ListTag();
            for (String line : text)
                lines.add(StringTag.valueOf("{\"text\":\"" + line + "\"}"));
            CompoundTag front = new CompoundTag();
            front.put("messages", lines);
            front.putString("color", "black");
            front.putBoolean("has_glowing_text", false);
            tag.put("front_text", front);
            return new StructureTemplate.StructureBlockInfo(current.pos(), current.state(), tag);
        }
        String role = tag.getString("ecmRole");
        // Only the chat village's data center has the chat server and the package server, only the
        // core village's the carrier core; elsewhere those racks are free.
        if (((role.equals(WorldNetwork.CHAT) || role.equals(WorldNetwork.REPO)) && village != chat)
                || (role.equals(WorldNetwork.CORE) && village != cells.coreVillage))
            return new StructureTemplate.StructureBlockInfo(current.pos(),
                    net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), null);
        data.provision(level, village, role);
        tag.remove("ecmRole");
        tag.putUUID("computerId", data.identity(level, village, role));
        tag.putBoolean("wasRunning", true);
        tag.putString("wasmModule", "terminal_os.wasm");
        return new StructureTemplate.StructureBlockInfo(current.pos(), current.state(), tag);
    }

    /**
     * The Phone Store's display frames: the marker item becomes the village's provisioned ePhone,
     * with its computer (fixed by the seed) already holding the Tech Village repository in
     * /etc/pkg/repos ({@link WorldNetwork#provisionStorePhone}); no app is installed.
     */
    @Override
    public StructureTemplate.StructureEntityInfo processEntity(LevelReader reader, BlockPos seedPos,
            StructureTemplate.StructureEntityInfo raw, StructureTemplate.StructureEntityInfo entity,
            StructurePlaceSettings settings, StructureTemplate template) {
        CompoundTag nbt = entity.nbt;
        if (nbt == null || !nbt.contains("Item", Tag.TAG_COMPOUND) || !(reader instanceof ServerLevelAccessor accessor)) return entity;
        CompoundTag custom = nbt.getCompound("Item").getCompound("components").getCompound("minecraft:custom_data");
        if (!custom.contains(PhoneStore.PHONE_MARKER)) return entity;
        var level = accessor.getLevel();
        var data = WorldNetwork.get(level);
        data.plan(level);
        int village = data.nearest(entity.blockPos).number();
        int slot = custom.getInt(PhoneStore.PHONE_MARKER);
        CellPlan cells = data.cells(level);
        CompoundTag out = nbt.copy();
        var phone = PhoneStore.phone(cells, cells.sim(village, true, slot));
        phone.set(com.example.evanscomputermod.phone.PhoneContent.PHONE_ID.get(), data.provisionStorePhone(level, village, slot));
        out.put("Item", phone.save(level.registryAccess()));
        // The frame's block in the world (the template holds its template position).
        out.putInt("TileX", entity.blockPos.getX());
        out.putInt("TileY", entity.blockPos.getY());
        out.putInt("TileZ", entity.blockPos.getZ());
        return new StructureTemplate.StructureEntityInfo(entity.pos, entity.blockPos, out);
    }

    protected StructureProcessorType<?> getType() {
        return TechWorldgen.PROCESSOR.get();
    }
}
//?}
