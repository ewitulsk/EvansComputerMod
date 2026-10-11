package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Per-block state of a conductor that doesn't belong in the block state: the
 * wrench cut mask (one bit per {@link Direction}). Saved with the chunk, and
 * carried through Sable assemble/disassemble by Sable's own block-entity
 * save/load around the move.
 */
public class ConductorBlockEntity extends BlockEntity {
    private byte cutMask;

    public ConductorBlockEntity(BlockPos pos, BlockState state) {
        super(RadioAntennaContent.CONDUCTOR_BE.get(), pos, state);
    }

    public boolean isCut(Direction side) {
        return (cutMask & (1 << side.ordinal())) != 0;
    }

    public int cutMask() {
        return cutMask;
    }

    public void setCut(Direction side, boolean cut) {
        byte before = cutMask;
        cutMask = (byte) (cut ? cutMask | (1 << side.ordinal()) : cutMask & ~(1 << side.ordinal()));
        if (before != cutMask) {
            setChanged();
            if (level != null && !level.isClientSide()) level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        if (cutMask != 0) tag.putByte("Cut", cutMask);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        cutMask = tag.getByte("Cut");
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        return saveWithoutMetadata(registries);
    }

    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }
}
//?}
