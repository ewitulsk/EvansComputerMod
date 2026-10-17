package com.example.evanscomputermod.disk;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.items.IItemHandler;
import net.neoforged.neoforge.items.ItemStackHandler;
import org.jetbrains.annotations.Nullable;

import java.nio.file.Path;

/**
 * A Disk Drive: one floppy slot ({@link DiskDrivePeripheral}, type
 * {@code drive}). A player puts a floppy in by using it on the drive and takes
 * it out with an empty hand; hoppers and pipes put floppies in and take them
 * out through the item capability. The {@link DiskDriveBlock#DISK} state shows
 * whether one is in.
 */
public class DiskDriveBlockEntity extends BlockEntity {
    private final ItemStackHandler slot = new ItemStackHandler(1) {
        @Override
        public int getSlotLimit(int s) {
            return 1;
        }

        @Override
        public boolean isItemValid(int s, ItemStack stack) {
            return stack.getItem() instanceof FloppyDiskItem;
        }

        @Override
        protected void onContentsChanged(int s) {
            contentsChanged();
        }
    };

    private final DiskDrivePeripheral peripheral = new DiskDrivePeripheral(this);
    private boolean hadDisk;

    public DiskDriveBlockEntity(BlockPos pos, BlockState state) {
        super(DiskContent.DISK_DRIVE_BE.get(), pos, state);
    }

    public DiskDrivePeripheral peripheral() {
        return peripheral;
    }

    /** Hoppers and pipes: the slot itself. */
    public IItemHandler automation() {
        return slot;
    }

    public boolean hasDisk() {
        return !slot.getStackInSlot(0).isEmpty();
    }

    ItemStack disk() {
        return slot.getStackInSlot(0);
    }

    /** The disk's number, given now if it has none. -1 without a disk or a server. */
    int diskId() {
        ItemStack d = disk();
        if (d.isEmpty() || level == null || level.getServer() == null) return -1;
        Integer id = d.get(DiskContent.DISK_ID.get());
        if (id == null) {
            id = DiskStorage.newId(level.getServer());
            d.set(DiskContent.DISK_ID.get(), id);
            setChanged();
        }
        return id;
    }

    /** The folder holding the disk's files, or null without a disk. */
    @Nullable
    Path diskFolder() {
        int id = diskId();
        return id < 0 ? null : DiskStorage.folder(level.getServer(), id);
    }

    @Nullable
    String label() {
        ItemStack d = disk();
        return d.isEmpty() || !d.has(DataComponents.CUSTOM_NAME) ? null : d.getHoverName().getString();
    }

    void setLabel(@Nullable String label) {
        ItemStack d = disk();
        if (d.isEmpty()) return;
        if (label == null || label.isEmpty()) d.remove(DataComponents.CUSTOM_NAME);
        else d.set(DataComponents.CUSTOM_NAME, Component.literal(label));
        setChanged();
    }

    /** Put a floppy in by hand. */
    void insertByHand(ItemStack disk) {
        slot.setStackInSlot(0, disk);
    }

    /** Take the floppy out by hand (EMPTY if there is none). */
    ItemStack takeByHand() {
        ItemStack d = slot.getStackInSlot(0).copy();
        slot.setStackInSlot(0, ItemStack.EMPTY);
        return d;
    }

    /** Push the floppy out of the front, into the world. */
    void eject() {
        if (level == null || !hasDisk()) return;
        ItemStack d = takeByHand();
        Block.popResourceFromFace(level, worldPosition, getBlockState().getValue(DiskDriveBlock.FACING), d);
    }

    private void contentsChanged() {
        setChanged();
        if (level == null || level.isClientSide()) return;
        boolean has = hasDisk();
        if (has == hadDisk) return;
        hadDisk = has;
        BlockState s = getBlockState();
        if (s.hasProperty(DiskDriveBlock.DISK) && s.getValue(DiskDriveBlock.DISK) != has)
            level.setBlock(worldPosition, s.setValue(DiskDriveBlock.DISK, has), 3);
        level.updateNeighbourForOutputSignal(worldPosition, getBlockState().getBlock());
        if (has) peripheral.diskInserted();
        else peripheral.diskRemoved();
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("Disk", slot.serializeNBT(registries));
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        slot.deserializeNBT(registries, tag.getCompound("Disk"));
        hadDisk = hasDisk();
    }
}
//?}
