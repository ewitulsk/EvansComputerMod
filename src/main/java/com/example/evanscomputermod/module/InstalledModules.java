package com.example.evanscomputermod.module;

import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.world.item.ItemStack;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * A computer's installed expansion cards and bay modules, as saved in the
 * block entity and carried on the terminal item when the block is broken
 * ({@code evanscomputermod:installed_modules}).
 *
 * @param bays  bit 0 = left bay card installed, bit 1 = right bay card
 * @param slots exactly {@link ModuleBays#SLOTS} stacks (empty where unused);
 *              module state lives in each stack's custom data
 */
public record InstalledModules(int bays, List<ItemStack> slots) {

    public static final InstalledModules EMPTY = new InstalledModules(0, emptySlots());

    public static final Codec<InstalledModules> CODEC = RecordCodecBuilder.create(i -> i.group(
            Codec.intRange(0, 3).fieldOf("bays").forGetter(InstalledModules::bays),
            ItemStack.OPTIONAL_CODEC.listOf().fieldOf("slots").forGetter(InstalledModules::slots)
    ).apply(i, InstalledModules::new));

    public static final StreamCodec<RegistryFriendlyByteBuf, InstalledModules> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.VAR_INT, InstalledModules::bays,
            ItemStack.OPTIONAL_LIST_STREAM_CODEC, InstalledModules::slots,
            InstalledModules::new);

    public InstalledModules {
        List<ItemStack> fixed = new ArrayList<>(ModuleBays.SLOTS);
        for (int i = 0; i < ModuleBays.SLOTS; i++) {
            fixed.add(i < slots.size() ? slots.get(i).copy() : ItemStack.EMPTY);
        }
        slots = Collections.unmodifiableList(fixed);
        bays &= 3;
    }

    private static List<ItemStack> emptySlots() {
        List<ItemStack> l = new ArrayList<>(ModuleBays.SLOTS);
        for (int i = 0; i < ModuleBays.SLOTS; i++) l.add(ItemStack.EMPTY);
        return l;
    }

    public boolean isEmpty() {
        return bays == 0 && slots.stream().allMatch(ItemStack::isEmpty);
    }

    // ItemStack has identity equality; data components need value equality.
    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (!(o instanceof InstalledModules other) || other.bays != bays) return false;
        for (int i = 0; i < ModuleBays.SLOTS; i++) {
            if (!ItemStack.matches(slots.get(i), other.slots.get(i))) return false;
        }
        return true;
    }

    @Override
    public int hashCode() {
        int h = bays;
        for (ItemStack s : slots) h = 31 * h + ItemStack.hashItemAndComponents(s) + s.getCount();
        return h;
    }
}
