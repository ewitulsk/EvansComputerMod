package com.example.evanscomputermod.item;

import net.minecraft.network.chat.Component;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;
//? if >=26.1 {
import net.minecraft.world.item.component.TooltipDisplay;
//?}

import java.util.function.Consumer;

/**
 * Item with extra tooltip lines, independent of the tooltip API version.
 */
public abstract class TooltipItem extends Item {

    protected TooltipItem(Properties properties) {
        super(properties);
    }

    /** Add tooltip lines for {@code stack}. */
    protected abstract void addTooltip(ItemStack stack, Consumer<Component> lines);

    //? if >=26.1 {
    @Override
    public void appendHoverText(ItemStack stack, TooltipContext context, TooltipDisplay display,
                                Consumer<Component> lines, TooltipFlag flag) {
        super.appendHoverText(stack, context, display, lines, flag);
        addTooltip(stack, lines);
    }
    //?} else {
    /*@Override
    public void appendHoverText(ItemStack stack, TooltipContext context, java.util.List<Component> lines, TooltipFlag flag) {
        super.appendHoverText(stack, context, lines, flag);
        addTooltip(stack, lines::add);
    }*/
    //?}
}
