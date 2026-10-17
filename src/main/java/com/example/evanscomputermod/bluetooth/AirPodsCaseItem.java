package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResultHolder;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.SlotAccess;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.ClickAction;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;

import java.util.function.Consumer;

/**
 * The AirPods' charging case (docs/BLUETOOTH.md).
 * <ul>
 *   <li>Right-click: open or close the lid. While it is open (a minute) the AirPods
 *       inside are in pairing mode: a phone's Settings > Bluetooth lists them.</li>
 *   <li>Sneak + right-click: take the AirPods out and put them in your ears (or into
 *       your inventory if you already wear something); with the case empty, put the
 *       AirPods you wear back in.</li>
 *   <li>In the inventory, click AirPods onto the case to put them in.</li>
 * </ul>
 * AirPods in the case charge.
 */
public class AirPodsCaseItem extends TooltipItem {

    /** How long the lid stays open, ticks. */
    public static final int LID_TICKS = 1200;

    public AirPodsCaseItem(Properties properties) {
        super(properties);
    }

    public static boolean lidOpen(ItemStack stack, Level level) {
        return AirPodsData.lidUntil(stack) > level.getGameTime();
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        ItemStack stack = player.getItemInHand(hand);
        if (level.isClientSide()) return InteractionResultHolder.success(stack);
        if (player.isSecondaryUseActive()) {
            AirPodsData.Pods inside = AirPodsData.inCase(stack);
            if (inside != null) {
                ItemStack pods = new ItemStack(BluetoothContent.AIRPODS.get());
                AirPodsData.setPods(pods, inside);
                AirPodsData.setInCase(stack, null);
                if (player.getItemBySlot(EquipmentSlot.HEAD).isEmpty()) {
                    player.setItemSlot(EquipmentSlot.HEAD, pods);
                    player.displayClientMessage(Component.translatable("message.evanscomputermod.airpods.worn"), true);
                } else if (!player.getInventory().add(pods)) {
                    player.drop(pods, false);
                }
                level.playSound(null, player.blockPosition(), SoundEvents.ARMOR_EQUIP_GENERIC.value(), SoundSource.PLAYERS, 0.6f, 1.4f);
            } else {
                ItemStack worn = player.getItemBySlot(EquipmentSlot.HEAD);
                if (worn.getItem() instanceof AirPodsItem) {
                    AirPodsData.setInCase(stack, AirPodsData.pods(worn));
                    player.setItemSlot(EquipmentSlot.HEAD, ItemStack.EMPTY);
                    player.displayClientMessage(Component.translatable("message.evanscomputermod.airpods.stored"), true);
                    level.playSound(null, player.blockPosition(), SoundEvents.ARMOR_EQUIP_GENERIC.value(), SoundSource.PLAYERS, 0.6f, 1.0f);
                } else {
                    player.displayClientMessage(Component.translatable("message.evanscomputermod.airpods.case_empty"), true);
                }
            }
            return InteractionResultHolder.success(stack);
        }
        boolean open = !lidOpen(stack, level);
        AirPodsData.setLidUntil(stack, open ? level.getGameTime() + LID_TICKS : 0);
        level.playSound(null, player.blockPosition(), open ? SoundEvents.CHEST_OPEN : SoundEvents.CHEST_CLOSE, SoundSource.PLAYERS, 0.3f, 2.0f);
        String key = !open ? "message.evanscomputermod.airpods.lid_closed"
                : AirPodsData.inCase(stack) == null ? "message.evanscomputermod.airpods.case_empty" : "message.evanscomputermod.airpods.pairing";
        player.displayClientMessage(Component.translatable(key), true);
        return InteractionResultHolder.success(stack);
    }

    /** Click AirPods onto the case in an inventory: they go in. */
    @Override
    public boolean overrideOtherStackedOnMe(ItemStack caseStack, ItemStack other, Slot slot, ClickAction action, Player player, SlotAccess access) {
        if (action != ClickAction.PRIMARY || !(other.getItem() instanceof AirPodsItem) || AirPodsData.inCase(caseStack) != null) return false;
        if (!player.level().isClientSide()) {
            AirPodsData.setInCase(caseStack, AirPodsData.pods(other));
        }
        other.shrink(1);
        return true;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        AirPodsData.Pods inside = AirPodsData.inCase(stack);
        if (inside == null) {
            lines.accept(Component.translatable("tooltip.evanscomputermod.airpods_case.empty").withStyle(ChatFormatting.GRAY));
        } else {
            lines.accept(Component.translatable("tooltip.evanscomputermod.airpods_case.inside", (int) Math.round(inside.battery()))
                    .withStyle(ChatFormatting.GRAY));
        }
        lines.accept(Component.translatable("tooltip.evanscomputermod.airpods_case.use").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
