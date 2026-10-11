package com.example.evanscomputermod.radio.controller;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/** Controller Receiver module item: a 2.4 GHz receiver for Wireless Controllers, in a module bay. */
public class ControllerReceiverModuleItem extends TooltipItem implements IComputerModuleItem {

    public ControllerReceiverModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new ControllerReceiverModule(host, savedState);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.controller_receiver_module.tooltip").withStyle(ChatFormatting.GRAY));
    }
}
//?}
