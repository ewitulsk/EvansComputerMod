package com.example.evanscomputermod.compat.create;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

/**
 * Block entity of the Redstone Link Interface: a {@link RedstoneLinkPeripheral}
 * exposed to every computer touching the block (through the peripheral
 * capability on all six faces).
 */
public class RedstoneLinkInterfaceBlockEntity extends BlockEntity {

    private final RedstoneLinkPeripheral peripheral = new RedstoneLinkPeripheral(new RedstoneLinkPeripheral.Location() {
        @Override
        @Nullable
        public ServerLevel level() {
            return level instanceof ServerLevel sl ? sl : null;
        }

        @Override
        public BlockPos pos() {
            return worldPosition;
        }

        @Override
        public void markDirty() {
            setChanged();
        }
    });

    private boolean started;

    public RedstoneLinkInterfaceBlockEntity(BlockPos pos, BlockState state) {
        super(CreateCompat.REDSTONE_LINK_INTERFACE_BE.get(), pos, state);
    }

    public RedstoneLinkPeripheral getPeripheral() {
        return peripheral;
    }

    public void serverTick() {
        if (!started) {
            started = true;
            peripheral.load();
        }
        peripheral.tick();
    }

    private void stop() {
        if (started) {
            started = false;
            peripheral.unload();
        }
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        stop();
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
        stop();
    }

    /** Channel configuration, as stored on the dropped item. */
    public CompoundTag saveConfig() {
        CompoundTag tag = new CompoundTag();
        peripheral.save(tag);
        return tag;
    }

    public void loadConfig(CompoundTag tag) {
        peripheral.load(tag);
        setChanged();
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("link", saveConfig());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        peripheral.load(tag.getCompound("link"));
    }
}
//?}
