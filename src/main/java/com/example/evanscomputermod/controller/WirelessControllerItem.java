package com.example.evanscomputermod.controller;

import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
//? if <=1.21.1 {
/*import net.minecraft.world.InteractionResultHolder;*/
//?}
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.UseOnContext;
import net.minecraft.world.level.Level;

import java.util.function.Consumer;

/**
 * The Wireless Xbox Controller.
 * <ul>
 *   <li>Right-click a Terminal: pair with that computer (right-clicking the one
 *       it is already paired with opens the Terminal as usual).</li>
 *   <li>Right-click: connect / disconnect. While connected, the bound keys
 *       drive the controller instead of the player.</li>
 *   <li>Sneak + right-click: the key binding screen.</li>
 * </ul>
 * The pairing and bindings are stored on the item ({@link ControllerData}).
 */
public class WirelessControllerItem extends TooltipItem {

    public WirelessControllerItem(Properties properties) {
        super(properties);
    }

    /** Whether {@code stack} is paired with the Terminal at {@code pos} in {@code level}. */
    public static boolean isPairedWith(ItemStack stack, Level level, BlockPos pos) {
        return pos.equals(ControllerData.pairedPos(stack))
                && ControllerServer.dimensionId(level).equals(ControllerData.pairedDimension(stack));
    }

    @Override
    public InteractionResult onItemUseFirst(ItemStack stack, UseOnContext ctx) {
        Level level = ctx.getLevel();
        Player player = ctx.getPlayer();
        if (player == null || player.isSecondaryUseActive()) return InteractionResult.PASS;
        if (!(level.getBlockEntity(ctx.getClickedPos()) instanceof TerminalBlockEntity tbe)) {
            return InteractionResult.PASS;
        }
        if (level.isClientSide()) {
            // Already paired here: let the Terminal open.
            return isPairedWith(stack, level, tbe.getBlockPos()) ? InteractionResult.PASS : InteractionResult.SUCCESS;
        }
        if (tbe.getComputerId().equals(ControllerData.computer(stack))) {
            // Moved computers (Sable) still match by id; refresh the position.
            if (!isPairedWith(stack, level, tbe.getBlockPos()) && player instanceof ServerPlayer sp) {
                ControllerServer.pair(sp, stack, tbe);
                return InteractionResult.SUCCESS;
            }
            return InteractionResult.PASS;
        }
        if (player instanceof ServerPlayer sp) ControllerServer.pair(sp, stack, tbe);
        return InteractionResult.SUCCESS;
    }

    //? if >=26.1 {
    @Override
    public InteractionResult use(Level level, Player player, InteractionHand hand) {
        if (level.isClientSide()) clientUse(player, hand);
        return InteractionResult.SUCCESS;
    }
    //?} else {
    /*@Override
    public InteractionResultHolder<ItemStack> use(Level level, Player player, InteractionHand hand) {
        if (level.isClientSide()) clientUse(player, hand);
        return InteractionResultHolder.sidedSuccess(player.getItemInHand(hand), level.isClientSide());
    }*/
    //?}

    private static void clientUse(Player player, InteractionHand hand) {
        if (player.isSecondaryUseActive()) {
            com.example.evanscomputermod.controller.client.ControllerClient.openBindings(hand);
        } else {
            com.example.evanscomputermod.controller.client.ControllerClient.toggle(hand);
        }
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        BlockPos pos = ControllerData.pairedPos(stack);
        if (pos == null) {
            lines.accept(Component.translatable("item.evanscomputermod.wireless_controller.unpaired")
                    .withStyle(ChatFormatting.GRAY));
        } else {
            lines.accept(Component.translatable("item.evanscomputermod.wireless_controller.paired",
                    pos.getX(), pos.getY(), pos.getZ()).withStyle(ChatFormatting.GRAY));
        }
        lines.accept(Component.translatable("item.evanscomputermod.wireless_controller.tooltip")
                .withStyle(ChatFormatting.DARK_GRAY));
    }
}
