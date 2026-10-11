package com.example.evanscomputermod.radio.wifi;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/** Wi-Fi module item: a SoftMAC 2.4/5 GHz radio (wlan0) for a computer's module bay. */
public class WifiModuleItem extends TooltipItem implements IComputerModuleItem {

    public WifiModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new WifiModule(host, savedState);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.wifi_module.tooltip").withStyle(ChatFormatting.GRAY));
    }
}
//?}
