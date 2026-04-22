package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
//? if >=26.1 {
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
//?}

import org.jetbrains.annotations.Nullable;

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

    /** True when the screen's cluster is both valid AND powered — drives the BER content quad. */
    private volatile boolean active;

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
    public boolean isActive() { return active; }

    /**
     * Update the active flag (cluster-valid AND powered). Called by the
     * owning terminal when either condition changes. Triggers a client
     * sync so the BER re-evaluates whether to draw the content quad.
     * <p>
     * Uses {@link Block#UPDATE_CLIENTS} (flag 2) — this is a BE data
     * change, not a block state change, so neighbor updates are both
     * unnecessary and harmful: a flag-3 call from inside a cluster
     * rescan re-enters {@code rescanScreenCluster} via the adjacent
     * terminal's {@code neighborChanged}, which cascades without bound.
     */
    public void setActive(boolean active) {
        if (this.active == active) return;
        this.active = active;
        setChanged();
        if (level != null && !level.isClientSide()) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
        }
    }

    /**
     * Called by the cluster discovery pass on the server to update
     * membership. Short-circuits when every field already matches the
     * requested values so repeated rescans don't spam block updates.
     * Uses flag 2 for the same reason as {@link #setActive} — BE data
     * changes must not fire neighbor updates or the cluster rescan
     * cascade becomes unbounded.
     */
    public void setClusterMembership(@Nullable BlockPos owner, @Nullable BlockPos anchor,
                                     int cols, int rows, boolean anchorFlag) {
        if (java.util.Objects.equals(this.ownerTerminal, owner)
                && java.util.Objects.equals(this.clusterAnchor, anchor)
                && this.clusterCols == cols
                && this.clusterRows == rows
                && this.isAnchor == anchorFlag) {
            return;
        }
        this.ownerTerminal = owner;
        this.clusterAnchor = anchor;
        this.clusterCols = cols;
        this.clusterRows = rows;
        this.isAnchor = anchorFlag;
        setChanged();
        if (level != null && !level.isClientSide()) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
        }
    }

    /**
     * Remap the stored absolute-world references ({@link #ownerTerminal} and
     * {@link #clusterAnchor}) by a translation delta. Bulk-move hooks (e.g.
     * sable sublevel assembly) copy BE NBT verbatim to the destination
     * position but do not know to update these custom fields — the owning
     * terminal's BlockPos has to be translated by the same vector as the
     * screen itself.
     *
     * <p>No-op on the client. Callers are expected to invoke this during the
     * move hook before the BE starts ticking at its new location.
     */
    public void translateReferences(net.minecraft.core.Vec3i delta) {
        if (level != null && level.isClientSide()) return;
        if (delta.getX() == 0 && delta.getY() == 0 && delta.getZ() == 0) return;
        boolean changed = false;
        if (ownerTerminal != null) {
            ownerTerminal = ownerTerminal.offset(delta);
            changed = true;
        }
        if (clusterAnchor != null) {
            clusterAnchor = clusterAnchor.offset(delta);
            changed = true;
        }
        if (changed) {
            setChanged();
            if (level != null) {
                level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
            }
        }
    }

    /** Clears cluster membership (no owning terminal, no rectangle). Also
     *  forces the active flag off — a screen that's not in a cluster can't
     *  be "powered on" in any meaningful sense. */
    public void clearCluster() {
        setClusterMembership(null, null, 0, 0, false);
        if (active) {
            active = false;
            setChanged();
            if (level != null && !level.isClientSide()) {
                level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
            }
        }
    }

    // --- NBT persistence ---

    //? if >=26.1 {
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
        output.putBoolean("active", active);
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
        active = input.getBooleanOr("active", false);
    }
    //?} else {
    /*@Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        if (ownerTerminal != null) {
            tag.putInt("owner_x", ownerTerminal.getX());
            tag.putInt("owner_y", ownerTerminal.getY());
            tag.putInt("owner_z", ownerTerminal.getZ());
        }
        if (clusterAnchor != null) {
            tag.putInt("anchor_x", clusterAnchor.getX());
            tag.putInt("anchor_y", clusterAnchor.getY());
            tag.putInt("anchor_z", clusterAnchor.getZ());
        }
        tag.putInt("cols", clusterCols);
        tag.putInt("rows", clusterRows);
        tag.putBoolean("anchor", isAnchor);
        tag.putBoolean("active", active);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("owner_x")) {
            ownerTerminal = new BlockPos(tag.getInt("owner_x"), tag.getInt("owner_y"), tag.getInt("owner_z"));
        } else {
            ownerTerminal = null;
        }
        if (tag.contains("anchor_x")) {
            clusterAnchor = new BlockPos(tag.getInt("anchor_x"), tag.getInt("anchor_y"), tag.getInt("anchor_z"));
        } else {
            clusterAnchor = null;
        }
        clusterCols = tag.getInt("cols");
        clusterRows = tag.getInt("rows");
        isAnchor = tag.getBoolean("anchor");
        active = tag.getBoolean("active");
    }*/
    //?}

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
