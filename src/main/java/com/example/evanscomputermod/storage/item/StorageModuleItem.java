package com.example.evanscomputermod.storage.item;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.module.ModuleState;
import com.example.evanscomputermod.storage.StorageEvents;
import com.example.evanscomputermod.storage.device.StorageModule;
import net.minecraft.ChatFormatting;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/** Storage Module item: install it in a bay, then right-click the slot with a Storage Cell. */
public class StorageModuleItem extends TooltipItem implements IComputerModuleItem {

    public StorageModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new StorageModule(host, savedState);
    }

    @Override
    public ModuleSlotVisual getSlotVisual(ItemStack stack) {
        return ModuleSlotVisual.STORAGE;
    }

    /** The cell saved in a module item (EMPTY if none). */
    public static ItemStack cellIn(ItemStack module, HolderLookup.Provider registries) {
        CompoundTag state = ModuleState.read(module);
        if (!state.contains("cell")) return ItemStack.EMPTY;
        return ItemStack.parse(registries, state.getCompound("cell")).orElse(ItemStack.EMPTY);
    }

    /** Burnt, blown up, cactus'd: the cell inside loses its contents. */
    @Override
    public void onDestroyed(ItemEntity entity) {
        super.onDestroyed(entity);
        StorageEvents.itemGone(entity);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.storage_module.tooltip").withStyle(ChatFormatting.GRAY));
        CompoundTag state = ModuleState.read(stack);
        if (state.contains("cell")) {
            CompoundTag cell = state.getCompound("cell");
            String id = cell.getString("id");
            lines.accept(Component.translatable("item.evanscomputermod.storage_module.holds", id)
                    .withStyle(ChatFormatting.DARK_GRAY));
        }
    }
}
//?}
