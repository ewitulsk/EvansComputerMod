package com.example.customworld.block;

import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerLevelAccess;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;

import javax.annotation.Nullable;

/**
 * Menu (Container) for the Terminal.
 * Handles client-server synchronization for the terminal state.
 * This is intentionally minimal since we use custom packets for terminal data.
 */
public class TerminalMenu extends AbstractContainerMenu {
    
    private final TerminalBlockEntity blockEntity;
    private final ContainerLevelAccess access;
    
    // Client-side constructor (called when opening from packet)
    public TerminalMenu(int containerId, Inventory playerInventory, FriendlyByteBuf extraData) {
        this(containerId, playerInventory, getBlockEntity(playerInventory, extraData));
    }
    
    // Server-side constructor
    public TerminalMenu(int containerId, Inventory playerInventory, TerminalBlockEntity blockEntity) {
        super(ModMenuTypes.TERMINAL_MENU.get(), containerId);
        this.blockEntity = blockEntity;
        this.access = ContainerLevelAccess.create(blockEntity.getLevel(), blockEntity.getBlockPos());
    }
    
    private static TerminalBlockEntity getBlockEntity(Inventory playerInventory, FriendlyByteBuf extraData) {
        BlockPos pos = extraData.readBlockPos();
        Level level = playerInventory.player.level();
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            return te;
        }
        throw new IllegalStateException("Terminal block entity not found at " + pos);
    }
    
    /**
     * Gets the terminal block entity associated with this menu.
     */
    public TerminalBlockEntity getBlockEntity() {
        return blockEntity;
    }
    
    /**
     * Gets the current terminal buffer as a string.
     */
    public String getTerminalBuffer() {
        return blockEntity.getBufferAsString();
    }
    
    /**
     * Gets the cursor X position.
     */
    public int getCursorX() {
        return blockEntity.getCursorX();
    }
    
    /**
     * Gets the cursor Y position.
     */
    public int getCursorY() {
        return blockEntity.getCursorY();
    }
    
    @Override
    public ItemStack quickMoveStack(Player player, int index) {
        // No inventory slots, nothing to quick-move
        return ItemStack.EMPTY;
    }
    
    @Override
    public boolean stillValid(Player player) {
        return stillValid(access, player, ModBlocks.TERMINAL_BLOCK.get());
    }
}
