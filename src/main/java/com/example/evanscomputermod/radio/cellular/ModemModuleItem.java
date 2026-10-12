package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
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
 * LTE Modem / mmWave Modem: a cellular modem for a computer's module bay
 * ({@code wwan0}). Its SIM (if any) lives in the module's saved state on the
 * item; sneak-right-click the air holding the modem to take the SIM out.
 */
public class ModemModuleItem extends TooltipItem implements IComputerModuleItem {
    private final CellTech tech;

    public ModemModuleItem(CellTech tech, Properties properties) {
        super(properties);
        this.tech = tech;
    }

    public CellTech tech() {
        return tech;
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new ModemModule(host, tech, savedState);
    }

    /** The SIM inside a loose modem item, or null. */
    public static SimCard simIn(ItemStack stack) {
        return CellNbt.readSim(ModuleState.read(stack).getCompound("sim"));
    }

    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        ItemStack stack = player.getItemInHand(hand);
        if (!player.isSecondaryUseActive() || CellNbt.readIdentity(ModuleState.read(stack).getCompound("sim")) == null)
            return super.use(level, player, hand);
        SimCard sim = simIn(stack);
        if (!level.isClientSide() && sim != null) {
            CompoundTag state = ModuleState.read(stack);
            state.remove("sim");
            ModuleState.write(stack, state);
            ItemStack out = SimCardItem.issued(sim);
            if (!player.getInventory().add(out)) player.drop(out, false);
            player.displayClientMessage(Component.translatable("message.evanscomputermod.modem.sim_out", sim.carrier(), sim.imsiText()), true);
        }
        return InteractionResultHolder.sidedSuccess(stack, level.isClientSide());
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod." + tech.id + "_modem.tooltip").withStyle(ChatFormatting.GRAY));
        com.example.evanscomputermod.card.SimIdentity sim = CellNbt.readIdentity(ModuleState.read(stack).getCompound("sim"));
        lines.accept(sim == null
                ? Component.translatable("item.evanscomputermod.modem.no_sim").withStyle(ChatFormatting.DARK_GRAY)
                : Component.translatable("item.evanscomputermod.modem.sim", sim.carrier(), sim.imsiText()).withStyle(ChatFormatting.AQUA));
    }
}
//?}
