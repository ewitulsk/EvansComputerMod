package com.example.evanscomputermod.radio.handheld;

//? if <=1.21.1 {
import com.example.evanscomputermod.controller.ControllerData;
import net.minecraft.ChatFormatting;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.component.CustomData;

import java.util.List;

/**
 * Walkie-Talkie: a two-way FM radio for the VHF and UHF two-way channels. It
 * receives like the Handheld Radio, and while its holder holds its
 * push-to-talk key it transmits their voice (5 W FM, see
 * {@link HandheldServer}). Half duplex, like a real one: while it talks it
 * hears nothing. The push-to-talk key is bound on the item itself, the way the
 * Wireless Controller binds its buttons (sneak + right-click: the tuning
 * screen, then click PTT and press a key); it is read straight from the
 * keyboard while the radio is in hand.
 */
public class WalkieTalkieItem extends HandheldRadioItem {

    /** Push-to-talk key a new Walkie-Talkie starts with. */
    public static final String DEFAULT_PTT = "key.keyboard.b";
    /** A new Walkie-Talkie: off, UHF 462.5625 MHz (FRS / PMR channel 1), volume 70, squelch 20. */
    public static final HandheldSettings DEFAULT = new HandheldSettings(false, HandheldBand.UHF, 462.5625e6, 70, 20);

    public WalkieTalkieItem(Properties properties) {
        super(properties);
    }

    /** The bands a Walkie-Talkie tunes: the two-way bands. */
    public static boolean tunes(HandheldBand band) {
        return band == HandheldBand.VHF || band == HandheldBand.UHF;
    }

    public static boolean isWalkie(ItemStack stack) {
        return stack.getItem() instanceof WalkieTalkieItem;
    }

    /** The push-to-talk key name bound on this radio (Minecraft key name, e.g. {@code key.keyboard.b}). */
    public static String ptt(ItemStack stack) {
        CompoundTag t = stack.getOrDefault(DataComponents.CUSTOM_DATA, CustomData.EMPTY).copyTag();
        String k = t.getString("ecm_ptt");
        return !k.isEmpty() && ControllerData.isValidKeyName(k) ? k : DEFAULT_PTT;
    }

    /** Bind the push-to-talk key ("" restores the default). Invalid names are ignored. */
    public static void setPtt(ItemStack stack, String key) {
        if (key == null || !ControllerData.isValidKeyName(key)) return;
        CustomData.update(DataComponents.CUSTOM_DATA, stack, root -> {
            if (key.isEmpty()) root.remove("ecm_ptt");
            else root.putString("ecm_ptt", key);
        });
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext ctx, List<Component> lines, TooltipFlag flag) {
        HandheldSettings s = HandheldSettings.read(stack);
        lines.add(Component.literal(String.format("%s %s %s", s.band(), freqLabel(s.freqHz()), s.on() ? "ON" : "off"))
                .withStyle(ChatFormatting.GRAY));
        lines.add(Component.translatable("item.evanscomputermod.walkie_talkie.tooltip").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
