package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.module.ModuleState;
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
 * LTE Modem / mmWave Modem: a framed cellular radio for a computer's module bay
 * ({@link FramedModemModule}; {@code celld} drives it and makes {@code wwan0}). The SIM in
 * its own slot is kept on the server ({@link ModemSimStore}); the item's module state
 * names it. Sneak-right-click the air holding the modem to take the SIM out.
 */
public class ModemModuleItem extends TooltipItem implements IComputerModuleItem {
    private final PhyProfile tech;

    public ModemModuleItem(PhyProfile tech, Properties properties) {
        super(properties);
        this.tech = tech;
    }

    public PhyProfile tech() {
        return tech;
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new FramedModemModule(host, tech, savedState);
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        ItemStack stack = player.getItemInHand(hand);
        com.example.evanscomputermod.card.SimIdentity sim = ModemSimStore.identity(ModuleState.read(stack).getCompound("sim"));
        if (!player.isSecondaryUseActive() || sim == null) return super.use(level, player, hand);
        if (!level.isClientSide()) {
            CompoundTag state = ModuleState.read(stack);
            ItemStack out = ModemSimStore.take(ModemSimStore.slotOf(state.getCompound("sim")));
            state.remove("sim");
            ModuleState.write(stack, state);
            if (!out.isEmpty() && !player.getInventory().add(out)) player.drop(out, false);
            player.displayClientMessage(Component.translatable("message.evanscomputermod.modem.sim_out", sim.carrier(), sim.imsiText()), true);
        }
        return InteractionResultHolder.sidedSuccess(stack, level.isClientSide());
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod." + tech.id + "_modem.tooltip").withStyle(ChatFormatting.GRAY));
        com.example.evanscomputermod.card.SimIdentity sim = ModemSimStore.identity(ModuleState.read(stack).getCompound("sim"));
        lines.accept(sim == null
                ? Component.translatable("item.evanscomputermod.modem.no_sim").withStyle(ChatFormatting.DARK_GRAY)
                : Component.translatable("item.evanscomputermod.modem.sim", sim.carrier(), sim.imsiText()).withStyle(ChatFormatting.AQUA));
    }
}
//?}
