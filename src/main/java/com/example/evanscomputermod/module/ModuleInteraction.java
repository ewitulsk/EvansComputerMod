package com.example.evanscomputermod.module;

import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.item.ModItems;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;

/**
 * Player interactions with a computer's module bays: right-click with an
 * expansion card or a module to install it, sneak-right-click a bay slot with
 * an empty hand to take a module (or an empty bay's card) out.
 */
public final class ModuleInteraction {

    public enum Result { HANDLED, PASS }

    private ModuleInteraction() {
    }

    /** Whether right-clicking a computer with {@code stack} is a bay action. */
    public static boolean isBayItem(ItemStack stack) {
        return stack.is(ModItems.MODULE_EXPANSION_CARD.get()) || stack.getItem() instanceof IComputerModuleItem;
    }

    /** Bay slot under the cursor, or -1 when not aiming at a bay face. */
    public static int aimedSlot(BlockState state, BlockPos pos, BlockHitResult hit) {
        Direction facing = state.getValue(TerminalBlock.FACING);
        int bay = ModuleBays.bayOnFace(facing, hit.getDirection());
        if (bay < 0) return -1;
        double y = hit.getLocation().y - pos.getY();
        return bay * 2 + (y >= 0.5 ? 0 : 1); // _1 is the upper slot
    }

    /** Server side. Right-click with a card or module. */
    public static Result useItem(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                 InteractionHand hand, BlockHitResult hit) {
        if (!(level.getBlockEntity(pos) instanceof TerminalBlockEntity te)) return Result.PASS;
        ModuleBays bays = te.getModuleBays();
        Direction facing = state.getValue(TerminalBlock.FACING);

        if (stack.is(ModItems.MODULE_EXPANSION_CARD.get())) {
            int bay = bays.installCard(ModuleBays.bayOnFace(facing, hit.getDirection()));
            if (bay < 0) {
                tell(player, "message.evanscomputermod.bays_full");
                return Result.HANDLED;
            }
            consume(player, hand, stack);
            level.playSound(null, pos, SoundEvents.ARMOR_EQUIP_IRON.value(), SoundSource.BLOCKS, 1.0f, 1.2f);
            tell(player, bay == ModuleBays.LEFT ? "message.evanscomputermod.bay_opened_left"
                    : "message.evanscomputermod.bay_opened_right");
            return Result.HANDLED;
        }

        if (stack.getItem() instanceof IComputerModuleItem) {
            if (bays.cardCount() == 0) {
                tell(player, "message.evanscomputermod.needs_expansion_card");
                return Result.HANDLED;
            }
            int slot = bays.installModule(stack, aimedSlot(state, pos, hit));
            if (slot < 0) {
                tell(player, "message.evanscomputermod.no_free_slot");
                return Result.HANDLED;
            }
            consume(player, hand, stack);
            level.playSound(null, pos, SoundEvents.ITEM_FRAME_ADD_ITEM, SoundSource.BLOCKS, 1.0f, 1.0f);
            overlay(player, Component.translatable("message.evanscomputermod.module_installed",
                    stack.getHoverName(), ModuleBays.SLOT_NAMES[slot]));
            return Result.HANDLED;
        }
        return Result.PASS;
    }

    /** Server side. Sneak-right-click with an empty hand. */
    public static Result sneakUse(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!(level.getBlockEntity(pos) instanceof TerminalBlockEntity te)) return Result.PASS;
        int slot = aimedSlot(state, pos, hit);
        if (slot < 0) return Result.PASS;
        ModuleBays bays = te.getModuleBays();
        int bay = ModuleBays.bayOf(slot);
        if (!bays.hasBay(bay)) return Result.PASS;

        int take = !bays.getStack(slot).isEmpty() ? slot : !bays.getStack(slot ^ 1).isEmpty() ? slot ^ 1 : -1;
        ItemStack out;
        if (take >= 0) {
            out = bays.removeModule(take);
            level.playSound(null, pos, SoundEvents.ITEM_FRAME_REMOVE_ITEM, SoundSource.BLOCKS, 1.0f, 1.0f);
        } else if (bays.removeCard(bay)) {
            out = new ItemStack(ModItems.MODULE_EXPANSION_CARD.get());
            level.playSound(null, pos, SoundEvents.ARMOR_EQUIP_IRON.value(), SoundSource.BLOCKS, 1.0f, 0.8f);
        } else {
            return Result.PASS;
        }
        if (!player.getInventory().add(out)) {
            player.drop(out, false);
        }
        return Result.HANDLED;
    }

    // ---------------------------------------------------------------- display panel

    /** Whether {@code stack} is a display panel (right-click the screen face to install it). */
    public static boolean isPanelItem(ItemStack stack) {
        return stack.getItem() instanceof com.example.evanscomputermod.item.DisplayPanelItem;
    }

    /**
     * Server side. Right-click the screen face with a panel: install it, or
     * swap it for the one installed (which goes to the player). Hot-plug: a
     * running computer's console resizes and keeps running.
     */
    public static Result installPanel(ItemStack stack, Level level, BlockPos pos, Player player, InteractionHand hand) {
        if (!(level.getBlockEntity(pos) instanceof TerminalBlockEntity te)) return Result.PASS;
        ItemStack current = te.getPanel();
        if (!current.isEmpty() && ItemStack.isSameItemSameComponents(current, stack)) {
            tell(player, "message.evanscomputermod.panel_same");
            return Result.HANDLED;
        }
        ItemStack old = te.installPanel(stack.copyWithCount(1));
        consume(player, hand, stack);
        if (!old.isEmpty() && !player.getAbilities().instabuild && !player.getInventory().add(old)) {
            player.drop(old, false);
        }
        level.playSound(null, pos, SoundEvents.ITEM_FRAME_ADD_ITEM, SoundSource.BLOCKS, 1.0f, 1.3f);
        var m = com.example.evanscomputermod.item.DisplayPanelItem.tierOf(stack).modes();
        overlay(player, Component.translatable("message.evanscomputermod.panel_installed", stack.getHoverName(),
                m.terminal().textCols(), m.terminal().textRows(), m.terminal().gfxWidth(), m.terminal().gfxHeight()));
        return Result.HANDLED;
    }

    /** Server side. Sneak-right-click the screen face with an empty hand: take the panel out (headless). */
    public static Result removePanel(Level level, BlockPos pos, Player player) {
        if (!(level.getBlockEntity(pos) instanceof TerminalBlockEntity te)) return Result.PASS;
        ItemStack old = te.installPanel(ItemStack.EMPTY);
        if (old.isEmpty()) return Result.PASS;
        if (!player.getInventory().add(old)) player.drop(old, false);
        level.playSound(null, pos, SoundEvents.ITEM_FRAME_REMOVE_ITEM, SoundSource.BLOCKS, 1.0f, 1.3f);
        tell(player, "message.evanscomputermod.panel_removed");
        return Result.HANDLED;
    }

    private static void consume(Player player, InteractionHand hand, ItemStack stack) {
        if (!player.getAbilities().instabuild) {
            stack.shrink(1);
        }
    }

    private static void tell(Player player, String key) {
        overlay(player, Component.translatable(key));
    }

    /** Action-bar message. */
    private static void overlay(Player player, Component message) {
        //? if >=26.1 {
        player.sendOverlayMessage(message);
        //?} else
        /*player.displayClientMessage(message, true);*/
    }
}
