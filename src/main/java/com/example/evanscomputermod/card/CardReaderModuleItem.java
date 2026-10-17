package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.module.ModuleState;
import com.example.evanscomputermod.radio.cellular.ModemSimStore;
import net.minecraft.ChatFormatting;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResultHolder;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;

import java.util.function.Consumer;

/**
 * Card Reader Module: the Card Reader/Writer's contacts in a module bay (a Terminal's or
 * an ePhone's). Right-click the installed module's bay with a card to put it in; the
 * computer's {@code eject()} only releases it: take it out by sneak-right-clicking the
 * air while holding the module (sneak-use of the bay removes the module, card and all). The card
 * stays on the server ({@link ModemSimStore}, as a modem's SIM does); the item's state
 * names it. Peripheral {@code card_reader}, the same methods as the block.
 */
public class CardReaderModuleItem extends TooltipItem implements IComputerModuleItem {

    public CardReaderModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new CardReaderModule(host, savedState);
    }

    @Override
    public ModuleSlotVisual getSlotVisual(ItemStack stack) {
        return ModuleSlotVisual.GENERIC;
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        ItemStack stack = player.getItemInHand(hand);
        CompoundTag state = ModuleState.read(stack);
        String slot = ModemSimStore.slotOf(state.getCompound("card"));
        if (!player.isSecondaryUseActive() || slot == null) return super.use(level, player, hand);
        if (!level.isClientSide()) {
            ItemStack out = ModemSimStore.take(slot);
            state.remove("card");
            ModuleState.write(stack, state);
            if (!out.isEmpty() && !player.getInventory().add(out)) player.drop(out, false);
            player.displayClientMessage(Component.translatable("message.evanscomputermod.card_reader.taken"), true);
        }
        return InteractionResultHolder.sidedSuccess(stack, level.isClientSide());
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.card_reader_module.tooltip").withStyle(ChatFormatting.GRAY));
        CompoundTag c = ModuleState.read(stack).getCompound("card");
        lines.accept(c.contains("kind")
                ? Component.translatable("item.evanscomputermod.card_reader_module.card", c.getString("kind")).withStyle(ChatFormatting.AQUA)
                : Component.translatable("item.evanscomputermod.card_reader_module.empty").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
