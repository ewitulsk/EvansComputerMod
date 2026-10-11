package com.example.evanscomputermod.radio.handheld;

//? if <=1.21.1 {
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResultHolder;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.level.Level;

import java.util.List;

/**
 * Handheld radio: a receive-only AM / shortwave / VHF FM receiver with a
 * telescopic whip. Right-click turns it on or off; sneak + right-click opens
 * the tuning screen (dial, band, scan, volume, squelch).
 */
public class HandheldRadioItem extends Item {

    public HandheldRadioItem(Properties properties) {
        super(properties);
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        ItemStack stack = player.getItemInHand(hand);
        if (player.isShiftKeyDown()) {
            if (level.isClientSide()) com.example.evanscomputermod.radio.handheld.client.HandheldScreen.open(hand);
            return InteractionResultHolder.sidedSuccess(stack, level.isClientSide());
        }
        if (!level.isClientSide()) {
            HandheldSettings s = HandheldSettings.read(stack);
            s.withOn(!s.on()).write(stack);
            player.displayClientMessage(Component.translatable(s.on() ? "item.evanscomputermod.handheld_radio.off"
                    : "item.evanscomputermod.handheld_radio.on"), true);
        }
        return InteractionResultHolder.sidedSuccess(stack, level.isClientSide());
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext ctx, List<Component> lines, TooltipFlag flag) {
        HandheldSettings s = HandheldSettings.read(stack);
        lines.add(Component.literal(String.format("%s %s %s", s.band(), freqLabel(s.freqHz()), s.on() ? "ON" : "off"))
                .withStyle(ChatFormatting.GRAY));
        lines.add(Component.translatable("item.evanscomputermod.handheld_radio.tooltip").withStyle(ChatFormatting.DARK_GRAY));
    }

    public static String freqLabel(double hz) {
        return hz < 3e6 ? String.format("%.0f kHz", hz / 1e3) : String.format("%.4f MHz", hz / 1e6);
    }
}
//?}
