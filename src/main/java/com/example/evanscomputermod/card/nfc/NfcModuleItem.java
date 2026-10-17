package com.example.evanscomputermod.card.nfc;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * NFC Module: the NFC Reader's coil in a module bay (a Terminal's, or an ePhone's: the
 * phone then reads cards held near it, and a card in the phone's other bay is read by
 * pads the player walks up to). Peripheral {@code nfc_reader}, the same methods as the block.
 */
public class NfcModuleItem extends TooltipItem implements IComputerModuleItem {

    public NfcModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new NfcModule(host, savedState);
    }

    @Override
    public ModuleSlotVisual getSlotVisual(ItemStack stack) {
        return ModuleSlotVisual.GENERIC;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.nfc_module.tooltip").withStyle(ChatFormatting.GRAY));
    }
}
//?}
