package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.saveddata.SavedData;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.level.BlockEvent;
import org.jetbrains.annotations.Nullable;

import java.util.HashMap;
import java.util.Map;
import java.util.UUID;

/**
 * Who placed each antenna feed point, per level (saved). Hazard world edits
 * on an antenna act as a fake player owned by this player, so claim mods
 * judge the edit by the antenna's builder (spec, multiplayer safety).
 */
public final class RadioOwners extends SavedData {
    private static final String NAME = "evanscomputermod_radio_owners";
    private final Map<Long, UUID> owners = new HashMap<>();

    static void register() {
        NeoForge.EVENT_BUS.addListener((BlockEvent.EntityPlaceEvent e) -> {
            if (e.getLevel() instanceof ServerLevel level && e.getPlacedBlock().getBlock() instanceof FeedPointBlock
                    && e.getEntity() instanceof Player p) {
                set(level, e.getPos(), p.getUUID());
            }
        });
    }

    private static RadioOwners of(ServerLevel level) {
        return level.getDataStorage().computeIfAbsent(new SavedData.Factory<>(RadioOwners::new, RadioOwners::load, null), NAME);
    }

    public static void set(ServerLevel level, BlockPos pos, UUID owner) {
        RadioOwners o = of(level);
        o.owners.put(pos.asLong(), owner);
        o.setDirty();
    }

    @Nullable
    public static UUID get(ServerLevel level, @Nullable BlockPos pos) {
        return pos == null ? null : of(level).owners.get(pos.asLong());
    }

    private static RadioOwners load(CompoundTag tag, HolderLookup.Provider registries) {
        RadioOwners o = new RadioOwners();
        ListTag list = tag.getList("Owners", Tag.TAG_COMPOUND);
        for (int i = 0; i < list.size(); i++) {
            CompoundTag t = list.getCompound(i);
            if (t.hasUUID("Owner")) o.owners.put(t.getLong("Pos"), t.getUUID("Owner"));
        }
        return o;
    }

    @Override
    public CompoundTag save(CompoundTag tag, HolderLookup.Provider registries) {
        ListTag list = new ListTag();
        for (var e : owners.entrySet()) {
            CompoundTag t = new CompoundTag();
            t.putLong("Pos", e.getKey());
            t.putUUID("Owner", e.getValue());
            list.add(t);
        }
        tag.put("Owners", list);
        return tag;
    }
}
//?}
