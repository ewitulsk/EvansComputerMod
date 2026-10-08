package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.level.Level;

import java.util.List;
import java.util.Locale;

/**
 * RF meter: while held (either hand) it shows the RF field strength at the
 * player on the action bar, in V/m and as the power a 0 dBi antenna would
 * pick up (dBm), with the exposure limit at that frequency. Reads
 * {@link RfExposure}, so it sees every transmitting antenna and amplifier
 * chain.
 */
public class RfMeterItem extends Item {
    public RfMeterItem(Properties properties) {
        super(properties);
    }

    /** The meter's reading at a player, as shown. */
    public static Component reading(ServerPlayer p) {
        var level = p.serverLevel();
        RfExposure.Field f = RfExposure.at(level.dimension().location().toString(), p.getX(), p.getEyeY(), p.getZ(), level.getGameTime());
        if (f.vPerM() <= 0) return Component.translatable("message.evanscomputermod.rf_meter.quiet");
        double limit = RfExposure.limitVPerM(f.hz());
        ChatFormatting color = f.vPerM() >= limit ? ChatFormatting.RED : f.vPerM() >= 0.5 * limit ? ChatFormatting.GOLD : ChatFormatting.GREEN;
        return Component.translatable("message.evanscomputermod.rf_meter.reading",
                String.format(Locale.ROOT, f.vPerM() < 10 ? "%.2f" : "%.0f", f.vPerM()),
                String.format(Locale.ROOT, "%.1f", f.dbmIsotropic()),
                String.format(Locale.ROOT, "%.3f", f.hz() / 1e6),
                String.format(Locale.ROOT, "%.0f", limit)).withStyle(color);
    }

    @Override
    public void inventoryTick(ItemStack stack, Level level, Entity entity, int slot, boolean selected) {
        if (level.isClientSide() || !(entity instanceof ServerPlayer p) || level.getGameTime() % 10 != 0) return;
        if (!selected && p.getOffhandItem() != stack) return;
        p.displayClientMessage(reading(p), true);
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext context, List<Component> tooltip, TooltipFlag flag) {
        tooltip.add(Component.translatable("item.evanscomputermod.rf_meter.tooltip").withStyle(ChatFormatting.GRAY));
    }
}
//?}
