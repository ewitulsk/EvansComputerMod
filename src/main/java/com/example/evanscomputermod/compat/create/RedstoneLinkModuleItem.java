package com.example.evanscomputermod.compat.create;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.module.ModuleState;
import net.minecraft.ChatFormatting;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * Redstone Link module item: install it in a computer's module bay to give
 * the computer 64 programmable Create Redstone Link channels.
 */
public class RedstoneLinkModuleItem extends TooltipItem implements IComputerModuleItem {

    public RedstoneLinkModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new RedstoneLinkModule(host, savedState);
    }

    @Override
    public ModuleSlotVisual getSlotVisual(ItemStack stack) {
        return ModuleSlotVisual.REDSTONE_LINK;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.redstone_link_module.tooltip")
                .withStyle(ChatFormatting.GRAY));
        int configured = ModuleState.read(stack).getList("channels", 10).size();
        if (configured > 0) {
            lines.accept(Component.translatable("item.evanscomputermod.redstone_link_module.configured", configured)
                    .withStyle(ChatFormatting.DARK_GRAY));
        }
    }
}
//?}
