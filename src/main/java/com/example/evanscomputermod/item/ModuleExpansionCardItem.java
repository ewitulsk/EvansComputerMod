package com.example.evanscomputermod.item;

import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * Right-click a computer to open a module bay on its left side (or the
 * right side, once the left is open, or the side you clicked). Each bay
 * holds two modules. Sneak-right-click an empty bay with an empty hand to
 * take the card back out.
 */
public class ModuleExpansionCardItem extends TooltipItem {

    public ModuleExpansionCardItem(Properties properties) {
        super(properties);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.module_expansion_card.tooltip")
                .withStyle(ChatFormatting.GRAY));
    }
}
