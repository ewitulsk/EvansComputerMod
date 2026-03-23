package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IFramebufferHost;
import com.example.evanscomputermod.computer.Framebuffer;
import com.example.evanscomputermod.config.DisplayConfig;
import com.example.evanscomputermod.network.FramebufferUpdatePacket;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.network.PacketDistributor;

import javax.annotation.Nullable;
import java.util.List;

/**
 * Block entity for the Display block.
 * Implements {@link IFramebufferHost} - a passive pixel display surface.
 * Does NOT have its own computer; requires an adjacent Terminal to control it.
 */
public class DisplayBlockEntity extends BlockEntity implements IFramebufferHost {

    private static final int DEFAULT_PIXELS_PER_BLOCK = 128;

    @Nullable
    private Framebuffer framebuffer;

    /** Position of the Terminal block controlling this display. */
    @Nullable
    private BlockPos controllerPos;

    /** Tick counter for rate-limiting flushes. */
    private int ticksSinceFlush = 0;

    public DisplayBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.DISPLAY_BLOCK_ENTITY.get(), pos, state);
    }

    // ==================== IFramebufferHost ====================

    @Override
    @Nullable
    public Framebuffer getFramebuffer() {
        if (framebuffer == null) {
            initFramebuffer();
        }
        return framebuffer;
    }

    @Override
    public int getDisplayWidth() {
        int ppb = DisplayConfig.SERVER_SPEC.isLoaded()
                ? DisplayConfig.SERVER.pixelsPerBlock.get()
                : DEFAULT_PIXELS_PER_BLOCK;
        return ppb;
    }

    @Override
    public int getDisplayHeight() {
        int ppb = DisplayConfig.SERVER_SPEC.isLoaded()
                ? DisplayConfig.SERVER.pixelsPerBlock.get()
                : DEFAULT_PIXELS_PER_BLOCK;
        return ppb;
    }

    private void initFramebuffer() {
        int w = getDisplayWidth();
        int h = getDisplayHeight();
        int tileSize = DisplayConfig.SERVER_SPEC.isLoaded()
                ? DisplayConfig.SERVER.tileSize.get() : 16;

        // Check global memory budget
        long maxMB = DisplayConfig.SERVER_SPEC.isLoaded()
                ? DisplayConfig.SERVER.maxFramebufferMemoryMB.get() : 256;
        long maxBytes = maxMB * 1024 * 1024;
        long needed = (long) w * h * 4 * 2; // pixels + previousPixels
        if (Framebuffer.getGlobalMemoryUsage() + needed > maxBytes) {
            EvansComputerMod.LOGGER.warn("Cannot create framebuffer: global memory budget exceeded ({} MB)",
                    maxMB);
            return;
        }

        framebuffer = new Framebuffer(w, h, tileSize);
        // Initialize to black with full opacity
        framebuffer.clear(0, 0, 0, 255);
    }

    // ==================== Controller Management ====================

    @Nullable
    public BlockPos getControllerPos() {
        return controllerPos;
    }

    /**
     * Sets the controller (Terminal block) for this display.
     * @param pos Position of the Terminal block, or null to disconnect.
     */
    public void setController(@Nullable BlockPos pos) {
        BlockPos old = this.controllerPos;
        this.controllerPos = pos;
        setChanged();

        if (old == null && pos != null) {
            EvansComputerMod.LOGGER.info("Display at {} connected to terminal at {}", worldPosition, pos);
        } else if (old != null && pos == null) {
            EvansComputerMod.LOGGER.info("Display at {} disconnected from terminal", worldPosition);
        }
    }

    /** Returns true if this display has a controller. */
    public boolean hasController() {
        return controllerPos != null;
    }

    // ==================== Server Tick ====================

    public static void serverTick(Level level, BlockPos pos, BlockState state, DisplayBlockEntity entity) {
        entity.ticksSinceFlush++;

        int flushRate = DisplayConfig.SERVER_SPEC.isLoaded()
                ? DisplayConfig.SERVER.maxFlushRateTicks.get() : 2;

        if (entity.ticksSinceFlush >= flushRate && entity.framebuffer != null && entity.framebuffer.isAnyDirty()) {
            entity.flushToClients();
            entity.ticksSinceFlush = 0;
        }
    }

    /**
     * Flush dirty tiles to all tracking players.
     * Called from fb_flush() or from the tick-based auto-flush.
     */
    public void flushToClients() {
        if (framebuffer == null || level == null || level.isClientSide) return;

        List<Framebuffer.DirtyTile> dirtyTiles = framebuffer.flush();
        if (dirtyTiles.isEmpty()) return;

        FramebufferUpdatePacket packet = FramebufferUpdatePacket.fromDirtyTiles(
                worldPosition, framebuffer.getWidth(), framebuffer.getHeight(),
                framebuffer.getTileSize(), dirtyTiles);

        // Send to all players tracking this chunk
        PacketDistributor.sendToPlayersTrackingChunk(
                (ServerLevel) level,
                new net.minecraft.world.level.ChunkPos(worldPosition),
                packet);

        ticksSinceFlush = 0;
    }

    // ==================== Neighbor Changes ====================

    public void onNeighborChanged() {
        // Validate controller still exists
        if (controllerPos != null && level != null && !level.isClientSide) {
            if (!(level.getBlockEntity(controllerPos) instanceof TerminalBlockEntity)) {
                setController(null);
            }
        }
    }

    public void onRemoved() {
        if (framebuffer != null) {
            framebuffer.release();
            framebuffer = null;
        }
        // Notify controller that this display is gone
        if (controllerPos != null && level != null && !level.isClientSide) {
            if (level.getBlockEntity(controllerPos) instanceof TerminalBlockEntity terminal) {
                terminal.onDisplayDetached(worldPosition);
            }
        }
    }

    // ==================== NBT ====================

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        if (controllerPos != null) {
            tag.putInt("ctrlX", controllerPos.getX());
            tag.putInt("ctrlY", controllerPos.getY());
            tag.putInt("ctrlZ", controllerPos.getZ());
        }
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("ctrlX")) {
            controllerPos = new BlockPos(tag.getInt("ctrlX"), tag.getInt("ctrlY"), tag.getInt("ctrlZ"));
        }
    }

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
