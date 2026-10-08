package com.example.evanscomputermod.radio.antenna;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.conductor.Feedline;
import net.minecraft.ChatFormatting;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.context.UseOnContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * Antenna analyzer: right-click a feed point for the antenna's resonance,
 * 2:1 SWR band, power rating and weakest link, e.g. "Resonant at 7.1 MHz ·
 * 2:1 SWR band 6.9–7.3 MHz · rated 200 W (antenna wire) / 1.4 kW
 * (insulators)". While the solver runs it shows the estimate and sends the
 * solved report when it lands.
 */
public class AntennaAnalyzerItem extends Item {
    public AntennaAnalyzerItem(Properties properties) {
        super(properties);
    }

    @Override
    public InteractionResult useOn(UseOnContext ctx) {
        Level level = ctx.getLevel();
        BlockPos pos = ctx.getClickedPos();
        BlockState state = level.getBlockState(pos);
        if (!(state.getBlock() instanceof ConductorBlock)) return InteractionResult.PASS;
        if (level.isClientSide()) return InteractionResult.SUCCESS;
        if (!(ctx.getPlayer() instanceof ServerPlayer player)) return InteractionResult.CONSUME;
        if (!(state.getBlock() instanceof FeedPointBlock)) {
            player.sendSystemMessage(Component.translatable("message.evanscomputermod.antenna_analyzer.not_feed").withStyle(ChatFormatting.GRAY));
            return InteractionResult.CONSUME;
        }
        if (player.isShiftKeyDown()) {
            // Sneak + right-click: the SWR plot screen.
            com.example.evanscomputermod.radio.antenna.tools.AnalyzerPackets.open(player, level, pos);
            return InteractionResult.CONSUME;
        }
        Antenna a = AntennaManager.get(level, pos);
        for (String line : reportLines(level, a)) player.sendSystemMessage(Component.literal(line));
        if (a.pending()) {
            AntennaManager.whenSolved(level, pos).thenAccept(solved -> {
                if (player.isRemoved()) return;
                player.sendSystemMessage(Component.literal(solved.summary()).withStyle(ChatFormatting.GREEN));
                if (!solved.details().isEmpty()) player.sendSystemMessage(Component.literal(solved.details()).withStyle(ChatFormatting.GRAY));
            });
        }
        return InteractionResult.CONSUME;
    }

    /** The analyzer's text for an antenna: summary, details, then the feedline. */
    public static List<String> reportLines(Level level, Antenna a) {
        List<String> out = new ArrayList<>();
        out.add(a.summary() + (a.pending() ? " (solving…)" : ""));
        if (!a.details().isEmpty()) out.add(a.details());
        Feedline line = Feedline.trace(level, a.feed());
        if (line.length() > 0) {
            double f = Double.isFinite(a.resonantHz()) ? a.resonantHz() : a.report().analysisHz();
            String loss = Double.isFinite(f) ? String.format(Locale.ROOT, ", %.2f dB at %s", line.lossDb(f), AntennaReport.hz(f)) : "";
            out.add("Feedline: " + line.length() + " blocks" + loss
                    + (line.arrestor() ? " · lightning arrestor fitted" : " · no lightning arrestor")
                    + (line.end() == null ? " · ends open" : ""));
        }
        return out;
    }

    @Override
    public void appendHoverText(ItemStack stack, TooltipContext context, List<Component> tooltip, TooltipFlag flag) {
        tooltip.add(Component.translatable("item.evanscomputermod.antenna_analyzer.tooltip").withStyle(ChatFormatting.GRAY));
        tooltip.add(Component.translatable("item.evanscomputermod.antenna_analyzer.tooltip_sneak").withStyle(ChatFormatting.GRAY));
    }
}
//?}
