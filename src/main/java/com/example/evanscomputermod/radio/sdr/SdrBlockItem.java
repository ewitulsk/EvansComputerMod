package com.example.evanscomputermod.radio.sdr;

//? if <=1.21.1 {
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;

import java.util.List;

/**
 * The SDR blocks' item: the tooltip says what the tier does and when to use IQ
 * mode or (Advanced SDR) framed PHY mode (docs/radio/SDR_PROGRAMS.md).
 */
public class SdrBlockItem extends BlockItem {
    private final SdrTier tier;

    public SdrBlockItem(SdrBlock block, Properties properties) {
        super(block, properties);
        this.tier = block.tier();
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext context, List<Component> lines, TooltipFlag flag) {
        super.appendHoverText(stack, context, lines, flag);
        lines.add(Component.translatable("block.evanscomputermod." + tier.id() + ".tooltip").withStyle(ChatFormatting.GRAY));
        lines.add(Component.translatable("tooltip.evanscomputermod.sdr.iq_mode").withStyle(ChatFormatting.DARK_GRAY));
        if (SdrPeripheral.framedCapable(tier))
            lines.add(Component.translatable("tooltip.evanscomputermod.sdr.framed_mode").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
