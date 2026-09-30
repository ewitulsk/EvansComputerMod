package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.Containers;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;

import java.util.UUID;

/**
 * Shared block entity plumbing for storage blocks: a persistent device id,
 * start on the first server tick (covers chunk loads and Sable moves), stop on
 * removal or unload.
 */
public abstract class StorageDeviceBlockEntity extends BlockEntity {

    protected UUID deviceId = UUID.randomUUID();
    private boolean started;

    protected StorageDeviceBlockEntity(BlockEntityType<?> type, BlockPos pos, BlockState state) {
        super(type, pos, state);
    }

    public UUID deviceId() {
        return deviceId;
    }

    /** First server tick in the world. */
    protected void start() {
    }

    /** Leaving the world (removed, unloaded, moved). */
    protected void stop() {
    }

    protected void tick() {
    }

    public final void serverTick() {
        if (!started) {
            started = true;
            start();
        }
        tick();
    }

    public boolean isStarted() {
        return started && !isRemoved();
    }

    private void halt() {
        if (started) {
            started = false;
            stop();
        }
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        halt();
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
        halt();
    }

    /** The block was broken: stop and drop what it holds. */
    public void dropContents() {
        halt();
        if (level == null) return;
        for (ItemStack stack : takeDrops()) {
            Containers.dropItemStack(level, worldPosition.getX() + 0.5, worldPosition.getY() + 0.5,
                    worldPosition.getZ() + 0.5, stack);
        }
    }

    /** Remove and return every stack the block holds. */
    protected abstract Iterable<ItemStack> takeDrops();

    public String describe() {
        return getBlockState().getBlock().getName().getString() + " at " + worldPosition.getX() + " "
                + worldPosition.getY() + " " + worldPosition.getZ();
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putUUID("device_id", deviceId);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("device_id")) deviceId = tag.getUUID("device_id");
    }
}
//?}
