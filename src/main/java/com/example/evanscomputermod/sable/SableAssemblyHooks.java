package com.example.evanscomputermod.sable;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Vec3i;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Dispatcher invoked by {@code BlockSubLevelAssemblyListener} implementations
 * on the four block classes we care about (Terminal, Screen, NetworkCable,
 * Interface). Responsible for preserving live state across sable's
 * block→sublevel and sublevel→block bulk-move transactions.
 *
 * <p>Sable's move flow (see {@code SubLevelAssemblyHelper.moveBlocks}):
 * <ol>
 *   <li>{@code blockEntity.saveWithFullMetadata()} on the source BE,</li>
 *   <li>{@code beforeMove()} on the Block (this is where we stash live
 *       state and flip the source BE's {@code transferring} flag),</li>
 *   <li>{@code chunk.setBlockState(newPos, rotatedState)} — creates a
 *       fresh BE at the destination,</li>
 *   <li>{@code newBlockEntity.loadWithComponents(tag)} — restores NBT,</li>
 *   <li>{@code afterMove()} on the Block (we retrieve the stash and
 *       hand the live instance to the new BE),</li>
 *   <li>{@code chunk.setBlockState(oldPos, AIR)} — triggers {@code
 *       setRemoved} on the source BE (suppressed by our transfer flag).</li>
 * </ol>
 *
 * <p>All methods are no-ops if sable isn't loaded, so callers on the
 * block classes don't need to re-check.
 */
public final class SableAssemblyHooks {

    private SableAssemblyHooks() {}

    /**
     * TTL-bounded stash of live computer state keyed by computerId. Entries
     * are created in {@link #onTerminalBeforeMove} and consumed in
     * {@link #onTerminalAfterMove}. If the matching afterMove never fires
     * (e.g. sable aborts the move), {@link #sweep} drops entries older
     * than {@value #STASH_TTL_NANOS} ns and shuts their computers down.
     */
    private static final Map<UUID, StashedTransfer> STASH = new ConcurrentHashMap<>();
    private static final long STASH_TTL_NANOS = 2_000_000_000L; // 2 seconds

    private static final class StashedTransfer {
        final TerminalBlockEntity.TransferBundle bundle;
        final UUID computerId;
        final long capturedAtNanos;
        StashedTransfer(TerminalBlockEntity.TransferBundle bundle, UUID computerId) {
            this.bundle = bundle;
            this.computerId = computerId;
            this.capturedAtNanos = System.nanoTime();
        }
    }

    // ================== Terminal ==================

    public static void onTerminalBeforeMove(ServerLevel from, ServerLevel to, BlockState state,
                                            BlockPos oldPos, BlockPos newPos) {
        sweep();
        if (!(SableCompat.resolveBlockEntity(from, oldPos) instanceof TerminalBlockEntity tbe)) {
            EvansComputerMod.LOGGER.info("[sable] terminal beforeMove {} -> {}: no source BE, skipping", oldPos, newPos);
            return;
        }
        ComputerInstance live = tbe.getComputer();
        if (live == null) {
            EvansComputerMod.LOGGER.info("[sable] terminal beforeMove {} -> {}: no live computer (wasRunning={})",
                    oldPos, newPos, tbe.getComputerId());
            return;
        }
        TerminalBlockEntity.TransferBundle bundle = tbe.captureForTransfer();
        if (bundle == null) return;
        STASH.put(tbe.getComputerId(), new StashedTransfer(bundle, tbe.getComputerId()));
        EvansComputerMod.LOGGER.info(
                "[sable] terminal beforeMove {} -> {}: captured computer {} (screenDisplay={}, cluster={})",
                oldPos, newPos, tbe.getComputerId(),
                bundle.screenDisplay != null, bundle.screenClusterInfo != null);
    }

    public static void onTerminalAfterMove(ServerLevel from, ServerLevel to, BlockState state,
                                           BlockPos oldPos, BlockPos newPos) {
        BlockEntity rawBE = SableCompat.resolveBlockEntity(to, newPos);
        if (!(rawBE instanceof TerminalBlockEntity newBE)) {
            EvansComputerMod.LOGGER.warn("[sable] terminal afterMove {} -> {}: destination BE missing (got {})",
                    oldPos, newPos, rawBE);
            sweep();
            return;
        }
        StashedTransfer stash = STASH.remove(newBE.getComputerId());
        if (stash == null) {
            EvansComputerMod.LOGGER.info("[sable] terminal afterMove {} -> {}: no stash for computer {}",
                    oldPos, newPos, newBE.getComputerId());
            return;
        }
        try {
            newBE.adoptComputer(stash.bundle);
            Vec3i delta = newPos.subtract(oldPos);
            EvansComputerMod.LOGGER.info(
                    "[sable] terminal afterMove {} -> {}: adopted computer {} (screenDisplay={}, delta={})",
                    oldPos, newPos, newBE.getComputerId(),
                    newBE.getScreenDisplay() != null, delta);
            // Defer cluster-info translation + peripheral rescan to the next
            // server tick. At this instant sable is midway through its
            // per-block moveBlocks loop: other cluster members may not yet
            // be placed at their destination positions, so looking them up
            // now returns null and setClusterMembership is skipped for
            // those members. By the next tick every block in the group has
            // been moved, and lookups via level.getBlockEntity hit.
            to.getServer().execute(() -> {
                if (newBE.isRemoved()) return;
                newBE.translateScreenClusterInfo(delta);
                // Shift the NIC exit positions by the move delta BEFORE
                // re-registering with CableNetworkManager — otherwise the
                // BFS starts from stale overworld coordinates and finds no
                // cables, silently dropping the terminal from its network.
                newBE.translateInterfaceExitPositions(delta);
                newBE.reregisterWithCableNetwork();
                ComputerInstance live = stash.bundle.computer;
                if (live != null) live.rescanPeripherals();
            });
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error(
                    "[sable] terminal afterMove: failed to adopt computer {} at {}",
                    newBE.getComputerId(), newPos, t);
        }
    }

    // ================== Screen ==================

    public static void onScreenAfterMove(ServerLevel from, ServerLevel to, BlockState state,
                                         BlockPos oldPos, BlockPos newPos) {
        BlockEntity rawBE = SableCompat.resolveBlockEntity(to, newPos);
        if (!(rawBE instanceof ScreenBlockEntity sbe)) {
            EvansComputerMod.LOGGER.warn("[sable] screen afterMove {} -> {}: destination BE missing (got {})",
                    oldPos, newPos, rawBE);
            return;
        }
        Vec3i delta = newPos.subtract(oldPos);
        sbe.translateReferences(delta);
        EvansComputerMod.LOGGER.info("[sable] screen afterMove {} -> {}: translated refs by {}",
                oldPos, newPos, delta);
    }

    // ================== Cable ==================

    /**
     * Re-evaluate all six connection booleans on the moved cable. Sable's
     * incremental placement path only fires {@code updateShape} for the
     * single face that a specific neighbor update touches, leaving faces
     * that should have changed but weren't poked pinned to the stale
     * pre-move world-side value.
     */
    public static void onCableAfterMove(ServerLevel from, ServerLevel to, BlockState state,
                                        BlockPos oldPos, BlockPos newPos) {
        BlockState current = to.getBlockState(newPos);
        if (!(current.getBlock() instanceof NetworkCableBlock)) return;
        BlockState recomputed = current
                .setValue(NetworkCableBlock.NORTH,
                        NetworkCableBlock.canConnectToFace(to.getBlockState(newPos.north()),
                                net.minecraft.core.Direction.SOUTH, to, newPos.north()))
                .setValue(NetworkCableBlock.SOUTH,
                        NetworkCableBlock.canConnectToFace(to.getBlockState(newPos.south()),
                                net.minecraft.core.Direction.NORTH, to, newPos.south()))
                .setValue(NetworkCableBlock.EAST,
                        NetworkCableBlock.canConnectToFace(to.getBlockState(newPos.east()),
                                net.minecraft.core.Direction.WEST, to, newPos.east()))
                .setValue(NetworkCableBlock.WEST,
                        NetworkCableBlock.canConnectToFace(to.getBlockState(newPos.west()),
                                net.minecraft.core.Direction.EAST, to, newPos.west()))
                .setValue(NetworkCableBlock.UP,
                        NetworkCableBlock.canConnectToFace(to.getBlockState(newPos.above()),
                                net.minecraft.core.Direction.DOWN, to, newPos.above()))
                .setValue(NetworkCableBlock.DOWN,
                        NetworkCableBlock.canConnectToFace(to.getBlockState(newPos.below()),
                                net.minecraft.core.Direction.UP, to, newPos.below()));
        if (!recomputed.equals(current)) {
            to.setBlock(newPos, recomputed, Block.UPDATE_ALL);
        }
        // Defer cable-network recomputation to the next server tick: sable's
        // moveBlocks loop may still be mid-flight when this fires, so any
        // registerTerminal() call made by a terminal's afterMove that lands
        // earlier in the same loop would walk cables that haven't yet been
        // placed. One deferred invalidate per group covers every ordering.
        to.getServer().execute(() -> {
            com.example.evanscomputermod.computer.CableNetworkManager mgr =
                    com.example.evanscomputermod.computer.CableNetworkManager.getInstance();
            if (mgr != null) mgr.invalidateCache();
        });
    }

    // ================== Interface ==================

    /** Identical connection refresh for the {@code InterfaceBlock}'s 6 bools. */
    public static void onInterfaceAfterMove(ServerLevel from, ServerLevel to, BlockState state,
                                            BlockPos oldPos, BlockPos newPos) {
        // InterfaceBlock uses the same 6 PipeBlock-style properties; defer
        // to the cable refresh if it turns out to share canConnectToFace.
        // Auto-connection bools are still handled by vanilla's updateShape on
        // the moved interface itself — what we do need is a cable-network
        // cache invalidate after the group finishes moving, so any terminal
        // whose exit position now lands on this interface is picked up.
        to.getServer().execute(() -> {
            com.example.evanscomputermod.computer.CableNetworkManager mgr =
                    com.example.evanscomputermod.computer.CableNetworkManager.getInstance();
            if (mgr != null) mgr.invalidateCache();
        });
    }

    // ================== Stash GC ==================

    private static void sweep() {
        long now = System.nanoTime();
        STASH.entrySet().removeIf(e -> {
            if (now - e.getValue().capturedAtNanos < STASH_TTL_NANOS) return false;
            StashedTransfer st = e.getValue();
            TerminalBlockEntity.TransferBundle b = st.bundle;
            if (b.computer != null) {
                ComputerRegistry.unregister(st.computerId);
                try {
                    b.computer.interrupt();
                    b.computer.close();
                } catch (Throwable t) {
                    EvansComputerMod.LOGGER.warn("Error shutting down orphaned transfer stash", t);
                }
            }
            return true;
        });
    }
}
//?}
