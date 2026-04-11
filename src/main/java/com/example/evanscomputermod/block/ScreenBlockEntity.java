package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;

import org.jspecify.annotations.Nullable;

/**
 * Block entity for a Screen block.
 * <p>
 * Holds cluster membership metadata set by the owning terminal's
 * {@link ScreenClusterDiscovery} rescan. Non-anchor screens render a plain
 * block face; the anchor screen's BER draws a single textured quad that
 * covers the entire rectangle in world space.
 */
public class ScreenBlockEntity extends BlockEntity {

    /** Terminal position that owns this screen, or null if unbound. */
    @Nullable
    private BlockPos ownerTerminal;

    /** Anchor (top-left in the face's 2D plane) of the validated cluster. */
    @Nullable
    private BlockPos clusterAnchor;

    /** Cluster width in screen blocks. */
    private int clusterCols;

    /** Cluster height in screen blocks. */
    private int clusterRows;

    /** True iff this screen is the anchor of its cluster. */
    private boolean isAnchor;

    /** Client-side: cached graphics texture for this cluster (anchor only). May be null. */
    public volatile com.example.evanscomputermod.computer.@Nullable TerminalDisplay clientDisplay;

    public ScreenBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.SCREEN_BLOCK_ENTITY.get(), pos, state);
    }

    // --- Cluster accessors ---

    @Nullable public BlockPos getOwnerTerminal() { return ownerTerminal; }
    @Nullable public BlockPos getClusterAnchor() { return clusterAnchor; }
    public int getClusterCols() { return clusterCols; }
    public int getClusterRows() { return clusterRows; }
    public boolean isAnchor() { return isAnchor; }

    /** Called by the cluster discovery pass on the server to update membership. */
    public void setClusterMembership(@Nullable BlockPos owner, @Nullable BlockPos anchor,
                                     int cols, int rows, boolean anchorFlag) {
        this.ownerTerminal = owner;
        this.clusterAnchor = anchor;
        this.clusterCols = cols;
        this.clusterRows = rows;
        this.isAnchor = anchorFlag;
        setChanged();
        if (level != null && !level.isClientSide()) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    /** Clears cluster membership (no owning terminal, no rectangle). */
    public void clearCluster() {
        setClusterMembership(null, null, 0, 0, false);
    }

    // --- NBT persistence ---

    @Override
    protected void saveAdditional(ValueOutput output) {
        super.saveAdditional(output);
        if (ownerTerminal != null) {
            output.putInt("owner_x", ownerTerminal.getX());
            output.putInt("owner_y", ownerTerminal.getY());
            output.putInt("owner_z", ownerTerminal.getZ());
        }
        if (clusterAnchor != null) {
            output.putInt("anchor_x", clusterAnchor.getX());
            output.putInt("anchor_y", clusterAnchor.getY());
            output.putInt("anchor_z", clusterAnchor.getZ());
        }
        output.putInt("cols", clusterCols);
        output.putInt("rows", clusterRows);
        output.putBoolean("anchor", isAnchor);
    }

    @Override
    protected void loadAdditional(ValueInput input) {
        super.loadAdditional(input);
        if (input.getInt("owner_x").isPresent()) {
            int x = input.getIntOr("owner_x", 0);
            int y = input.getIntOr("owner_y", 0);
            int z = input.getIntOr("owner_z", 0);
            ownerTerminal = new BlockPos(x, y, z);
        } else {
            ownerTerminal = null;
        }
        if (input.getInt("anchor_x").isPresent()) {
            int x = input.getIntOr("anchor_x", 0);
            int y = input.getIntOr("anchor_y", 0);
            int z = input.getIntOr("anchor_z", 0);
            clusterAnchor = new BlockPos(x, y, z);
        } else {
            clusterAnchor = null;
        }
        clusterCols = input.getIntOr("cols", 0);
        clusterRows = input.getIntOr("rows", 0);
        isAnchor = input.getBooleanOr("anchor", false);
    }

    // --- Client sync ---

    @Nullable
    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        return saveWithoutMetadata(registries);
    }
}
