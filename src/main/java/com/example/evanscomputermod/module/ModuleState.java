package com.example.evanscomputermod.module;

import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.CustomData;

/**
 * A module's persistent state, stored under one key of its item stack's
 * custom data so it travels with the item.
 */
public final class ModuleState {

    public static final String KEY = "ecm_module_state";

    private ModuleState() {
    }

    public static CompoundTag read(ItemStack stack) {
        CustomData data = stack.get(DataComponents.CUSTOM_DATA);
        if (data == null) return new CompoundTag();
        CompoundTag tag = data.copyTag();
        //? if >=26.1 {
        return tag.getCompoundOrEmpty(KEY);
        //?} else
        /*return tag.getCompound(KEY);*/
    }

    public static void write(ItemStack stack, CompoundTag state) {
        CustomData data = stack.get(DataComponents.CUSTOM_DATA);
        CompoundTag tag = data == null ? new CompoundTag() : data.copyTag();
        if (state.isEmpty()) tag.remove(KEY);
        else tag.put(KEY, state);
        if (tag.isEmpty()) stack.remove(DataComponents.CUSTOM_DATA);
        else stack.set(DataComponents.CUSTOM_DATA, CustomData.of(tag));
    }
}
