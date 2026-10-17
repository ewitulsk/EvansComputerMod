package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.core.Holder;
import net.minecraft.network.chat.Component;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResultHolder;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.Equipable;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;

import java.util.function.Consumer;

/**
 * AirPods: wireless earbuds with a microphone (docs/BLUETOOTH.md). Worn in the head
 * slot (right-click to put them in your ears). Paired with an ePhone, they connect
 * to it over Bluetooth while worn: the phone's sound plays in your ears only, and
 * its microphone is the AirPods'. Pair them from the phone's Settings > Bluetooth
 * with their case's lid open.
 */
public class AirPodsItem extends TooltipItem implements Equipable {

    public AirPodsItem(Properties properties) {
        super(properties);
    }

    @Override
    public EquipmentSlot getEquipmentSlot() {
        return EquipmentSlot.HEAD;
    }

    @Override
    public Holder<SoundEvent> getEquipSound() {
        return SoundEvents.ARMOR_EQUIP_GENERIC;
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        if (!level.isClientSide()) AirPodsData.pods(player.getItemInHand(hand));
        return swapWithEquipmentSlot(this, level, player, hand);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        AirPodsData.Pods p = AirPodsData.peekPods(stack);
        int battery = p == null ? 100 : (int) Math.round(p.battery());
        lines.accept(Component.translatable("tooltip.evanscomputermod.airpods.battery", battery).withStyle(ChatFormatting.GRAY));
        if (p != null && p.pairedTo() != 0) {
            lines.accept(Component.translatable("tooltip.evanscomputermod.airpods.paired",
                    p.phoneName().isEmpty() ? BtAddress.format(p.pairedTo()) : p.phoneName()).withStyle(ChatFormatting.GRAY));
        } else {
            lines.accept(Component.translatable("tooltip.evanscomputermod.airpods.unpaired").withStyle(ChatFormatting.DARK_GRAY));
        }
        lines.accept(Component.translatable("tooltip.evanscomputermod.airpods.use").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
