package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IBayUseItem;
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * A SIM card. Blank until a carrier issues it at one of its cell towers
 * (right-click the tower with the blank card, or the tower screen's Issue SIM
 * button); then it carries the {@code evanscomputermod:sim_card} component:
 * carrier, IMSI and the secret key the carrier keeps too. Right-click an
 * installed LTE / mmWave modem's bay with it to put it in (any SIM already
 * there comes back out).
 */
public class SimCardItem extends TooltipItem implements IBayUseItem {

    public SimCardItem(Properties properties) {
        super(properties);
    }

    /** The SIM's identity, or null for a blank card. */
    public static SimCard sim(ItemStack stack) {
        return stack.isEmpty() ? null : stack.get(CellularContent.SIM_CARD_DATA.get());
    }

    public static boolean isIssued(ItemStack stack) {
        return stack.getItem() instanceof SimCardItem && sim(stack) != null;
    }

    public static ItemStack issued(SimCard sim) {
        ItemStack s = new ItemStack(CellularContent.SIM_CARD.get());
        s.set(CellularContent.SIM_CARD_DATA.get(), sim);
        return s;
    }

    @Override
    public boolean isFoil(ItemStack stack) {
        return false;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        SimCard s = sim(stack);
        if (s == null) {
            lines.accept(Component.translatable("item.evanscomputermod.sim_card.blank").withStyle(ChatFormatting.GRAY));
            return;
        }
        lines.accept(Component.translatable("item.evanscomputermod.sim_card.carrier", s.carrier()).withStyle(ChatFormatting.AQUA));
        lines.accept(Component.translatable("item.evanscomputermod.sim_card.imsi", s.imsiText()).withStyle(ChatFormatting.GRAY));
        lines.accept(Component.translatable("item.evanscomputermod.sim_card.use").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
