package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResultHolder;
import net.minecraft.world.SimpleMenuProvider;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.level.Level;

import java.util.List;
import java.util.Objects;
import java.util.UUID;

/**
 * The ePhone: a computer you carry. It carries its computer's UUID (given on
 * first use; copies of one phone are the same computer), its battery and its
 * hardware (panel, two modules, SIM) as data components.
 *
 * <ul>
 *   <li>Right-click: open the phone full screen (every key and the mouse go to it; Escape closes).</li>
 *   <li>Sneak + right-click: the phone's slot screen (panel, modules, SIM, visibility toggles).</li>
 * </ul>
 * Held in either hand it runs; in the hotbar it sleeps; anywhere else it is off.
 */
public class EPhoneItem extends Item {

    public EPhoneItem(Properties properties) {
        super(properties);
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        ItemStack stack = player.getItemInHand(hand);
        if (!level.isClientSide()) {
            UUID id = PhoneItemData.ensureId(stack);
            if (player.isShiftKeyDown() && player instanceof ServerPlayer sp) {
                sp.openMenu(new SimpleMenuProvider((c, inv, p) -> new PhoneMenu(c, inv, hand, id),
                        Component.translatable("container.evanscomputermod.ephone")), buf -> buf.writeBoolean(hand == InteractionHand.MAIN_HAND));
            }
        } else if (!player.isShiftKeyDown()) {
            com.example.evanscomputermod.phone.client.PhoneScreen.open(hand);
        }
        return InteractionResultHolder.sidedSuccess(stack, level.isClientSide());
    }

    /** Battery and component churn must not replay the hand's equip animation. */
    @Override
    public boolean shouldCauseReequipAnimation(ItemStack oldStack, ItemStack newStack, boolean slotChanged) {
        return slotChanged || oldStack.getItem() != newStack.getItem()
                || !Objects.equals(PhoneItemData.id(oldStack), PhoneItemData.id(newStack));
    }

    @Override
    public boolean isBarVisible(ItemStack stack) {
        return PhoneItemData.id(stack) != null && PhoneItemData.batteryPercent(stack) < 100;
    }

    @Override
    public int getBarWidth(ItemStack stack) {
        return (int) Math.round(13 * PhoneItemData.batteryPercent(stack) / 100.0);
    }

    @Override
    public int getBarColor(ItemStack stack) {
        double p = PhoneItemData.batteryPercent(stack);
        return p > 50 ? 0x55FF55 : p > 20 ? 0xFFFF55 : 0xFF5555;
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext ctx, List<Component> lines, TooltipFlag flag) {
        PhoneHardware h = PhoneHardware.of(stack);
        lines.add(Component.translatable("item.evanscomputermod.ephone.battery",
                String.format(java.util.Locale.ROOT, "%.0f", PhoneItemData.batteryPercent(stack))).withStyle(ChatFormatting.GRAY));
        var tier = h.tier();
        lines.add((tier == null ? Component.translatable("item.evanscomputermod.ephone.no_panel")
                : Component.translatable("item.evanscomputermod.ephone.panel", h.panel().getHoverName())).withStyle(ChatFormatting.GRAY));
        for (int i = 0; i < 2; i++) {
            ItemStack m = h.module(i);
            if (!m.isEmpty()) lines.add(Component.literal("  " + m.getHoverName().getString()).withStyle(ChatFormatting.DARK_GRAY));
        }
        if (!h.sim().isEmpty()) lines.add(Component.translatable("item.evanscomputermod.ephone.sim").withStyle(ChatFormatting.DARK_GRAY));
        lines.add(Component.translatable("item.evanscomputermod.ephone.tooltip").withStyle(ChatFormatting.DARK_GRAY));
        UUID id = PhoneItemData.id(stack);
        if (flag.isAdvanced() && id != null) lines.add(Component.literal(id.toString()).withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
