package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerLevelAccess;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.items.IItemHandler;
import net.neoforged.neoforge.items.ItemStackHandler;
import net.neoforged.neoforge.items.SlotItemHandler;
import org.jetbrains.annotations.Nullable;

/**
 * Drive GUI: two rows of five cell slots above the player inventory.
 * Shift-click moves cells in and out.
 */
public class DriveMenu extends AbstractContainerMenu {

    /** Slot grid origin in the screen (pixels), shared with DriveScreen. */
    public static final int CELLS_X = 44, CELLS_Y = 20, INV_X = 8, INV_Y = 84;

    @Nullable
    private final DriveBlockEntity drive;
    private final ContainerLevelAccess access;

    /** Client side, opened from the server's packet. */
    public DriveMenu(int id, Inventory inventory, FriendlyByteBuf extra) {
        this(id, inventory, lookup(inventory, extra.readBlockPos()));
    }

    public DriveMenu(int id, Inventory inventory, @Nullable DriveBlockEntity drive) {
        super(StorageContent.DRIVE_MENU.get(), id);
        this.drive = drive;
        this.access = drive == null || drive.getLevel() == null ? ContainerLevelAccess.NULL
                : ContainerLevelAccess.create(drive.getLevel(), drive.getBlockPos());
        IItemHandler cells = drive != null ? drive.cellHandler() : new ItemStackHandler(DriveBlockEntity.SLOTS);
        for (int i = 0; i < DriveBlockEntity.SLOTS; i++) {
            addSlot(new SlotItemHandler(cells, i, CELLS_X + (i % 5) * 18, CELLS_Y + (i / 5) * 18) {
                @Override
                public boolean mayPlace(ItemStack stack) {
                    return stack.getItem() instanceof StorageCellItem;
                }

                @Override
                public int getMaxStackSize() {
                    return 1;
                }
            });
        }
        for (int row = 0; row < 3; row++) {
            for (int col = 0; col < 9; col++) {
                addSlot(new Slot(inventory, col + row * 9 + 9, INV_X + col * 18, INV_Y + row * 18));
            }
        }
        for (int col = 0; col < 9; col++) addSlot(new Slot(inventory, col, INV_X + col * 18, INV_Y + 58));
    }

    @Nullable
    private static DriveBlockEntity lookup(Inventory inventory, BlockPos pos) {
        return inventory.player.level().getBlockEntity(pos) instanceof DriveBlockEntity d ? d : null;
    }

    @Nullable
    public DriveBlockEntity drive() {
        return drive;
    }

    @Override
    public ItemStack quickMoveStack(Player player, int index) {
        Slot slot = slots.get(index);
        if (!slot.hasItem()) return ItemStack.EMPTY;
        ItemStack stack = slot.getItem();
        ItemStack copy = stack.copy();
        int cellEnd = DriveBlockEntity.SLOTS;
        if (index < cellEnd) {
            if (!moveItemStackTo(stack, cellEnd, slots.size(), true)) return ItemStack.EMPTY;
        } else {
            if (!(stack.getItem() instanceof StorageCellItem)) return ItemStack.EMPTY;
            if (!moveItemStackTo(stack, 0, cellEnd, false)) return ItemStack.EMPTY;
        }
        if (stack.isEmpty()) slot.setByPlayer(ItemStack.EMPTY);
        else slot.setChanged();
        return copy;
    }

    @Override
    public boolean stillValid(Player player) {
        return drive != null && !drive.isRemoved()
                && stillValid(access, player, StorageContent.DRIVE.get());
    }
}
//?}
