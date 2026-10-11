package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;

import java.util.List;

/**
 * The RF wrench. Right-click a conductor's arm (or a face of its centre) to
 * cut that side's connection so parallel elements don't merge; click again
 * to restore it. The behaviour lives in {@link ConductorBlock#useItemOn}
 * for every item in {@code #evanscomputermod:rf_wrenches}, so packs can add
 * other mods' wrenches to the tag.
 */
public class RfWrenchItem extends Item {
    public RfWrenchItem(Properties properties) {
        super(properties);
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext context, List<Component> tooltip, TooltipFlag flag) {
        tooltip.add(Component.translatable("item.evanscomputermod.rf_wrench.tooltip").withStyle(ChatFormatting.GRAY));
    }
}
//?}
