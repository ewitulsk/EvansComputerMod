package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.*;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.UUIDUtil;
import net.minecraft.nbt.CompoundTag;
//? if >=26.1 {
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
//?}
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.state.BlockState;

import com.example.evanscomputermod.computer.ClientSyncState;
import com.example.evanscomputermod.computer.FramebufferDiffTracker;
import com.example.evanscomputermod.network.MouseInputPacket;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;

import org.jetbrains.annotations.Nullable;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Block Entity for the Terminal block.
 * Implements {@link IComputerHost} to provide a computer embedding context.
 * Delegates display to {@link TerminalDisplay} and runtime to {@link ComputerInstance}.
 */
public class TerminalBlockEntity extends BlockEntity implements MenuProvider, IComputerHost {

    // Terminal dimensions (framebuffer size)
    public static final int TERMINAL_WIDTH = 160;
    public static final int TERMINAL_HEIGHT = 50;

    // Per-tile resolution for in-world Screen blocks.
    public static final int SCREEN_TILE_TEXT_W = 32;
    public static final int SCREEN_TILE_TEXT_H = 18;
    public static final int SCREEN_TILE_GFX_W = 128;
    public static final int SCREEN_TILE_GFX_H = 72;
    /**
     * Maximum pixel dimensions for a screen cluster framebuffer. Cap is
     * applied uniformly with aspect-ratio preservation in
     * {@link #rescanScreenCluster}; clusters bigger than this in world
     * space stretch the capped image to fill via the renderer's UV-quad.
     *
     * <p>Sized for full-color (RGBA8888) playback: {@code 640 × 360 × 4
     * = 921 600 bytes} of pixel data fits inside the kernel-wasm
     * SCREEN_GFX_BASE region (now relocated to 0x300000 with a 1 MiB
     * carve-out, see {@code rust/operating-system/rust/.cargo/config.toml}).
     */
    public static final int SCREEN_MAX_GFX_W = 640;
    public static final int SCREEN_MAX_GFX_H = 360;

    // Framebuffer display state — stores cell data read from WASM memory
    private final TerminalDisplay display = new TerminalDisplay(TERMINAL_WIDTH, TERMINAL_HEIGHT);

    // Screen cluster display (populated when an in-world screen cluster is attached).
    @Nullable
    private volatile TerminalDisplay screenDisplay;

    /** Snapshot of the currently attached screen cluster. */
    public record ScreenClusterInfo(
            BlockPos anchor,
            Direction facing,
            int cols,
            int rows,
            int gfxWidth,
            int gfxHeight,
            java.util.List<BlockPos> members
    ) {}

    @Nullable
    private volatile ScreenClusterInfo screenClusterInfo;

    // True when the screen cluster is powered on. Independent of cluster
    // validity: a valid cluster with `screenPowered == false` still
    // displays the "no signal" inactive face texture and the BER skips
    // rendering. Flipped by the Rust OS via the `screen_set_power` host
    // function (and implicitly by `screen::init()`).
    private volatile boolean screenPowered = false;

    // Per-client delta sync state for the screen cluster display
    private final Map<UUID, ClientSyncState> screenClientSyncStates = new ConcurrentHashMap<>();

    // Scratch buffers used by syncScreenDeltaToClients to snapshot the live
    // screen pixel/palette arrays under synchronized(screenDisplay). Diffing
    // and shadow commit run against these private copies so the WASM worker
    // thread can't mid-write the arrays we're reading. Sized once per
    // cluster resize (in rescanScreenCluster).
    @Nullable
    private byte[] screenSnapshotPixels;
    private final int[] screenSnapshotPalette = new int[256];
    private final int[] screenSnapshotDims = new int[2];

    // Current input line (for line-by-line input mode)
    private StringBuilder inputLine = new StringBuilder();

    // Input mode: true = character mode, false = line mode
    private boolean characterMode = false;

    // Which WASM module to execute when terminal opens
    private String wasmModule = "terminal_os";
    private String wasmFunction = "main";

    // Computer instance for executing programs (server-side only)
    @Nullable
    private volatile ComputerInstance computer;
    private volatile boolean wasmInitialized = false;

    // Whether the computer was running before chunk unload / server stop
    // Used to auto-start on chunk reload
    private boolean wasRunning = false;

    // Async loading state
    private volatile boolean wasmLoading = false;
    @Nullable
    private CompletableFuture<ComputerInstance> loadingFuture;

    // Shared executor for background WASM loading (single thread to avoid overload)
    private static final ExecutorService WASM_EXECUTOR = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "WASM-Loader");
        t.setDaemon(true);
        return t;
    });

    // Unique computer ID - persists when block is picked up and moved
    private UUID computerId;

    // Redstone output power for each of the 6 sides (DOWN, UP, NORTH, SOUTH, WEST, EAST)
    private final int[] redstoneOutput = new int[6];

    // Redstone input power for each of the 6 sides (cached, updated on neighbor change)
    private volatile int[] redstoneInput = new int[6];

    // Bitmask of disabled (link-down) faces
    private volatile long disabledFacesMask = 0L;

    // Exit positions for each network interface (parallel to networkMacs array)
    private BlockPos[] interfaceExitPositions;

    // Per-client sync state for delta protocol
    private final Map<UUID, ClientSyncState> clientSyncStates = new ConcurrentHashMap<>();

    // True while this BE is in the middle of a bulk-move hand-off (e.g. sable
    // assembling the block into a physics sublevel). When set, setRemoved()
    // skips shutdownComputer() so the live ComputerInstance can be adopted by
    // the freshly-created BE at the destination. Cleared by adoptComputer()
    // after the new BE has taken ownership.
    private volatile boolean transferring = false;

    public TerminalBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.TERMINAL_BLOCK_ENTITY.get(), pos, state);
        this.computerId = UUID.randomUUID();
    }

    // ==================== IComputerHost Implementation ====================

    @Override
    public UUID getComputerId() {
        return computerId;
    }

    @Override
    @Nullable
    public MinecraftServer getServer() {
        return level != null ? level.getServer() : null;
    }

    @Override
    public void markDirty() {
        setChanged();
    }

    /**
     * Server-side tick: syncs the framebuffer to clients if the worker thread
     * detected a dirty counter change. Runs every game tick (~50ms) on the server thread.
     */
    public void serverTick() {
        if (computer != null) {
            computer.tickSync();
        }
    }

    /**
     * Provides a server-side ticker for this block entity.
     */
    public static <T extends BlockEntity> BlockEntityTicker<T> createTicker(Level level) {
        if (level.isClientSide()) return null;
        return (lvl, pos, state, blockEntity) -> {
            if (blockEntity instanceof TerminalBlockEntity te) {
                te.serverTick();
            }
        };
    }

    @Override
    public void syncToClients() {
        if (level == null || level.isClientSide()) return;
        level.getServer().execute(() -> {
            setChanged();
            if (level instanceof ServerLevel serverLevel) {
                syncDeltaToClients(serverLevel);
                if (screenDisplay != null) {
                    syncScreenDeltaToClients(serverLevel);
                }
            }
        });
    }

    @Override
    public void forceNextKeyframe() {
        for (com.example.evanscomputermod.computer.ClientSyncState state : clientSyncStates.values()) {
            state.needsKeyframe = true;
        }
        for (com.example.evanscomputermod.computer.ClientSyncState state : screenClientSyncStates.values()) {
            state.needsKeyframe = true;
        }
    }

    // --- Screen cluster display accessors ---

    /** Returns the current screen cluster display, or null if no cluster is attached. */
    @Nullable
    public TerminalDisplay getScreenDisplay() {
        return screenDisplay;
    }

    /** Returns the current screen cluster info, or null if no cluster is attached. */
    @Nullable
    public ScreenClusterInfo getScreenClusterInfo() {
        return screenClusterInfo;
    }

    /** True if this terminal currently has a valid screen cluster attached. */
    public boolean hasScreenCluster() {
        return screenDisplay != null && screenClusterInfo != null;
    }

    /**
     * Resolve the set of players that should receive a delta packet for
     * this terminal. For BEs sitting at regular world positions, the
     * 64-block distance filter is correct. For BEs that have been pulled
     * into a sable physics plot, world coordinates live thousands of blocks
     * away from any player — we fall through to the plot's tracking-player
     * list so the packet still reaches viewers of the rigid body.
     */
    private List<ServerPlayer> collectSyncRecipients(ServerLevel serverLevel) {
        //? if <=1.21.1 {
        List<ServerPlayer> plotPlayers =
                com.example.evanscomputermod.sable.SableCompat.getPlotTrackingPlayers(serverLevel, worldPosition);
        if (plotPlayers != null) return plotPlayers;
        //?}
        return serverLevel.getPlayers(
                p -> p.distanceToSqr(worldPosition.getX(), worldPosition.getY(), worldPosition.getZ()) < 64 * 64
        );
    }

    /**
     * Sync display to all tracking players using delta protocol.
     * Each player has independent sync state for optimal bandwidth.
     */
    private void syncDeltaToClients(ServerLevel serverLevel) {
        List<ServerPlayer> players = collectSyncRecipients(serverLevel);

        // Clean up states for disconnected players
        clientSyncStates.keySet().removeIf(uuid ->
                players.stream().noneMatch(p -> p.getUUID().equals(uuid)));

        for (ServerPlayer player : players) {
            ClientSyncState state = clientSyncStates.computeIfAbsent(player.getUUID(), k -> {
                ClientSyncState s = new ClientSyncState();
                s.init(display);
                return s;
            });

            try {
                if (state.shouldSendKeyframe()) {
                    sendKeyframe(player, state);
                } else if (state.isClientReady()) {
                    sendDelta(player, state);
                }
                // else: client not ready, changes coalesce naturally
            } catch (Exception e) {
                EvansComputerMod.LOGGER.debug("Error syncing delta to {}", player.getName().getString(), e);
                state.needsKeyframe = true;
            }
        }
    }

    private void sendKeyframe(ServerPlayer player, ClientSyncState state) {
        long gen = state.tracker.getGeneration();
        if (gen == 0) gen = 1;
        // Hold the display monitor so the WASM worker thread can't swap
        // out cellData (via setFromBytes) between the packet build and the
        // shadow commit. Without this, the shadow captures a newer frame
        // than the one the client was told about, and intervening cell
        // writes get permanently lost from the delta stream.
        TerminalDeltaPacket packet;
        synchronized (display) {
            packet = TerminalDeltaPacket.createKeyframe(
                    worldPosition, display, gen, state.deflater, TerminalDeltaPacket.TARGET_TERMINAL);
            state.tracker.commitShadow(display);
        }
        PacketDistributor.sendToPlayer(player, packet);
        state.markKeyframeSent(gen);
    }

    /**
     * Sync the screen cluster's display to nearby players, using its own
     * per-client sync state. Uses {@code targetKind=1} on the delta packet
     * so the client routes the payload to the anchor ScreenBlockEntity.
     */
    private void syncScreenDeltaToClients(ServerLevel serverLevel) {
        TerminalDisplay sd = screenDisplay;
        ScreenClusterInfo info = screenClusterInfo;
        if (sd == null || info == null) return;

        List<ServerPlayer> players = collectSyncRecipients(serverLevel);

        screenClientSyncStates.keySet().removeIf(uuid ->
                players.stream().noneMatch(p -> p.getUUID().equals(uuid)));

        for (ServerPlayer player : players) {
            ClientSyncState state = screenClientSyncStates.computeIfAbsent(player.getUUID(), k -> {
                ClientSyncState s = new ClientSyncState();
                s.init(sd);
                return s;
            });

            try {
                if (state.shouldSendKeyframe()) {
                    sendScreenKeyframe(player, state, sd);
                } else if (state.isClientReady()) {
                    sendScreenDelta(player, state, sd);
                }
            } catch (Exception e) {
                EvansComputerMod.LOGGER.debug("Error syncing screen delta to {}", player.getName().getString(), e);
                state.needsKeyframe = true;
            }
        }
    }

    private void sendScreenKeyframe(ServerPlayer player, ClientSyncState state, TerminalDisplay sd) {
        long gen = state.tracker.getGeneration();
        if (gen == 0) gen = 1;
        // Keyframe serialization reads sd's pixel/palette arrays directly.
        // Hold the display monitor for the full build+commit so the WASM
        // worker thread can't race the read. Keyframes are rare (first
        // connect + every 10 s) so the locking window is acceptable.
        TerminalDeltaPacket packet;
        synchronized (sd) {
            packet = TerminalDeltaPacket.createKeyframe(
                    worldPosition, sd, gen, state.deflater, TerminalDeltaPacket.TARGET_SCREEN);
            state.tracker.commitShadow(sd);
        }
        PacketDistributor.sendToPlayer(player, packet);
        state.markKeyframeSent(gen);
    }

    private void sendScreenDelta(ServerPlayer player, ClientSyncState state, TerminalDisplay sd) {
        // Snapshot the live graphics arrays under the display's monitor so
        // the WASM worker thread (which writes via setGfxFromBytes) can't
        // race our read. The scratch buffer is sized to the cluster's
        // current pixel byte count (gfxW * gfxH * bpp) — when a program
        // switches pixel format mid-flight the byte count changes, so we
        // grow the scratch buffer here instead of in rescanScreenCluster.
        int needed;
        int snapshotPixelFormat;
        synchronized (sd) {
            snapshotPixelFormat = sd.getPixelFormat();
            int bpp = (snapshotPixelFormat == TerminalDisplay.PIXEL_FORMAT_RGBA8888) ? 4 : 1;
            needed = sd.getGfxWidth() * sd.getGfxHeight() * bpp;
        }
        if (screenSnapshotPixels == null || screenSnapshotPixels.length < needed) {
            screenSnapshotPixels = new byte[Math.max(needed, 1)];
        }
        byte[] snapshotPixels = screenSnapshotPixels;
        int snapshotMode;
        synchronized (sd) {
            snapshotMode = sd.snapshotGfx(screenSnapshotDims, snapshotPixels, screenSnapshotPalette);
            // Re-read in case it changed between the size check and snapshot.
            snapshotPixelFormat = sd.getPixelFormat();
        }
        int snapshotGfxW = screenSnapshotDims[0];
        int snapshotGfxH = screenSnapshotDims[1];

        // Text cells on a screen cluster are static after init — computing
        // the text delta against the live sd is race-free. Gfx goes against
        // the private snapshot.
        FramebufferDiffTracker.TextDelta textDelta = state.tracker.computeTextDelta(sd);
        FramebufferDiffTracker.GfxDelta gfxDelta = state.tracker.computeGfxDelta(
                snapshotGfxW, snapshotGfxH, snapshotPixels, screenSnapshotPalette,
                snapshotMode, snapshotPixelFormat);

        int shadowMode = state.tracker.getShadowDisplayMode();
        int shadowFormat = state.tracker.getShadowPixelFormat();
        if (snapshotMode != shadowMode || snapshotPixelFormat != shadowFormat) {
            sendScreenKeyframe(player, state, sd);
            return;
        }

        boolean textChanged = !textDelta.changedRowIndices().isEmpty() || textDelta.scrollOffset() != 0;
        boolean gfxChanged = gfxDelta != null && (!gfxDelta.changedTileIndices().isEmpty() || gfxDelta.paletteChanged());
        if (!textChanged && !gfxChanged) return;

        TerminalDeltaPacket packet = TerminalDeltaPacket.createDelta(
                worldPosition, textDelta.generation(), snapshotMode,
                textDelta, gfxDelta, sd.getWidth(), state.deflater,
                TerminalDeltaPacket.TARGET_SCREEN);
        PacketDistributor.sendToPlayer(player, packet);

        // Commit text from sd (race-free) and gfx from the snapshot —
        // otherwise the shadow commit re-reads the racy pixel/palette arrays.
        state.tracker.commitShadowText(sd);
        state.tracker.commitShadowGfx(snapshotGfxW, snapshotGfxH, snapshotPixels,
                screenSnapshotPalette, snapshotMode, snapshotPixelFormat);
        state.markSent(textDelta.generation());
    }

    private void sendDelta(ServerPlayer player, ClientSyncState state) {
        // Hold the display monitor across compute+build+commit. The WASM
        // worker thread calls display.setFromBytes() (which re-allocates
        // cellData atomically) at arbitrary moments. Without the lock, the
        // delta and the subsequent commitShadow can see different cellData
        // arrays — the shadow then represents a frame the client was never
        // told about, and every cell write in the gap is lost forever. On
        // 1.21.1 this manifested as ghost rows and missing output lines
        // when programs like `gfxtest screen` print and scroll rapidly.
        int mode;
        int shadowMode;
        boolean modeChanged;
        FramebufferDiffTracker.TextDelta textDelta;
        FramebufferDiffTracker.GfxDelta gfxDelta;
        int width;
        TerminalDeltaPacket packet;
        synchronized (display) {
            mode = display.getDisplayMode();
            shadowMode = state.tracker.getShadowDisplayMode();
            modeChanged = mode != shadowMode;
            if (modeChanged) {
                // Delegate — sendKeyframe will re-acquire the monitor.
            } else {
                textDelta = state.tracker.computeTextDelta(display);
                gfxDelta = (mode >= 1 || shadowMode >= 1)
                        ? state.tracker.computeGfxDelta(display) : null;

                boolean textChanged = !textDelta.changedRowIndices().isEmpty() || textDelta.scrollOffset() != 0;
                boolean gfxChanged = gfxDelta != null && (!gfxDelta.changedTileIndices().isEmpty() || gfxDelta.paletteChanged());
                if (!textChanged && !gfxChanged) return;

                width = display.getWidth();
                packet = TerminalDeltaPacket.createDelta(
                        worldPosition, textDelta.generation(), mode,
                        textDelta, gfxDelta, width, state.deflater,
                        TerminalDeltaPacket.TARGET_TERMINAL);
                state.tracker.commitShadow(display);
                PacketDistributor.sendToPlayer(player, packet);
                state.markSent(textDelta.generation());
                return;
            }
        }
        // Mode changed — send a keyframe so client gets full GFX state
        // (delta packets don't include gfxWidth/gfxHeight, so the client
        // can't allocate pixel buffers from a delta alone).
        sendKeyframe(player, state);
    }

    /**
     * Called when a client acknowledges a delta packet or requests initial sync.
     * {@code target} selects which sync-state map the ack applies to:
     * {@link TerminalDeltaPacket#TARGET_TERMINAL} for the main terminal GUI,
     * {@link TerminalDeltaPacket#TARGET_SCREEN} for the in-world screen cluster.
     */
    public void onClientReady(UUID playerUuid, byte target, long ackedGeneration) {
        Map<UUID, ClientSyncState> map = (target == TerminalDeltaPacket.TARGET_SCREEN)
                ? screenClientSyncStates
                : clientSyncStates;
        ClientSyncState state = map.get(playerUuid);
        if (state != null) {
            state.onClientReady(ackedGeneration);
        }

        // Generation 0 = client just opened the terminal GUI, needs a bootstrap keyframe.
        // This path only applies to the terminal target — the screen's bootstrap is
        // handled by the normal `syncScreenDeltaToClients` path on the next dirty tick.
        if (target == TerminalDeltaPacket.TARGET_TERMINAL
                && ackedGeneration == 0
                && level instanceof ServerLevel serverLevel) {
            ServerPlayer player = serverLevel.getServer().getPlayerList().getPlayer(playerUuid);
            if (player != null) {
                ClientSyncState syncState = clientSyncStates.computeIfAbsent(playerUuid, k -> {
                    ClientSyncState s = new ClientSyncState();
                    s.init(display);
                    return s;
                });
                syncState.needsKeyframe = true;
                try {
                    sendKeyframe(player, syncState);
                } catch (Exception e) {
                    EvansComputerMod.LOGGER.debug("Error sending keyframe to {}", player.getName().getString(), e);
                }
            }
        }
    }

    @Override
    @Nullable
    public IFramebufferDisplay getFramebufferDisplay() {
        return display;
    }

    @Override
    @Nullable
    public IRedstoneProvider getRedstoneProvider() {
        return new IRedstoneProvider() {
            @Override
            public void setRedstoneOutput(int absoluteSide, int power) {
                TerminalBlockEntity.this.setRedstoneOutput(absoluteSide, power);
            }

            @Override
            public int getRedstoneInput(int absoluteSide) {
                return TerminalBlockEntity.this.getRedstoneInput(absoluteSide);
            }

            @Override
            public Direction relativeToAbsolute(int relativeSide) {
                Direction facing = getBlockState().getValue(TerminalBlock.FACING);
                return switch (relativeSide) {
                    case 0 -> Direction.DOWN;
                    case 1 -> Direction.UP;
                    case 2 -> facing;
                    case 3 -> facing.getOpposite();
                    case 4 -> facing.getCounterClockWise();
                    case 5 -> facing.getClockWise();
                    default -> Direction.NORTH;
                };
            }
        };
    }

    @Override
    @Nullable
    public IWorldAccess getWorldAccess() {
        return new IWorldAccess() {
            @Override
            @Nullable
            public Level getLevel() {
                return TerminalBlockEntity.this.getLevel();
            }

            @Override
            public BlockPos getBlockPos() {
                return TerminalBlockEntity.this.getBlockPos();
            }
        };
    }

    @Override
    @Nullable
    public IVisualProgramming getVisualProgramming() {
        return this::openVisualEditor;
    }

    // ==================== Display Delegation ====================

    /**
     * Sets the computer ID (used when restoring from NBT).
     */
    public void setComputerId(UUID computerId) {
        this.computerId = computerId;
    }

    /** Get the framebuffer display for rendering. */
    public TerminalDisplay getDisplay() {
        return display;
    }

    public int getCursorX() { return display.getCursorX(); }
    public int getCursorY() { return display.getCursorY(); }
    public boolean isCursorVisible() { return display.isCursorVisible(); }

    // ==================== WASM Lifecycle ====================

    /**
     * Initializes the computer and loads the configured WASM module.
     * Called when the terminal is first opened.
     */
    public void initializeWasm() {
        if (level == null || level.isClientSide()) {
            return;
        }

        // Check if computer is faulted and needs reset
        if (wasmInitialized && computer != null && computer.isFaulted()) {
            EvansComputerMod.LOGGER.info("Resetting faulted WASM terminal");
            computer.close();
            computer = null;
            wasmInitialized = false;
            wasmLoading = false;
        }

        if (wasmInitialized || wasmLoading) {
            return;
        }

        wasmLoading = true;
        EvansComputerMod.LOGGER.info("Loading terminal...");

        final String moduleToLoad = wasmModule;
        byte[][] discoveredMacs = discoverInterfaces();

        loadingFuture = CompletableFuture.supplyAsync(() -> {
            try {
                EvansComputerMod.LOGGER.info("Starting async WASM loading for module: {}", moduleToLoad);
                ComputerInstance instance = new ComputerInstance(this, discoveredMacs);
                instance.loadModule(moduleToLoad);
                EvansComputerMod.LOGGER.info("Async WASM loading complete for module: {}", moduleToLoad);
                return instance;
            } catch (WasmManager.WasmExecutionException e) {
                EvansComputerMod.LOGGER.error("Failed to load WASM module in background", e);
                throw new RuntimeException(e);
            }
        }, WASM_EXECUTOR);

        loadingFuture.whenComplete((instance, error) -> {
            if (level != null && level.getServer() != null) {
                level.getServer().execute(() -> onWasmLoadComplete(instance, error));
            }
        });
    }

    private void onWasmLoadComplete(@Nullable ComputerInstance instance, @Nullable Throwable error) {
        if (isRemoved() || computer != null) {
            // Either the BE is gone, or a bulk-move adopt beat us to it and
            // already installed a live instance. Drop this scratch one.
            if (instance != null) {
                instance.close();
            }
            wasmLoading = false;
            loadingFuture = null;
            return;
        }

        wasmLoading = false;
        loadingFuture = null;

        if (error != null) {
            wasmInitialized = true;

            Throwable cause = error;
            while (cause.getCause() != null) {
                cause = cause.getCause();
            }
            EvansComputerMod.LOGGER.error("Error loading WASM module: {} - {}", wasmModule, cause.getMessage());

            setChanged();
            if (level != null && !level.isClientSide()) {
                level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
            }
            return;
        }

        computer = instance;
        wasmInitialized = true;
        ComputerRegistry.register(this);

        // Register with cable network manager for physical network topology
        CableNetworkManager cableMgr = CableNetworkManager.getInstance();
        if (cableMgr != null && level != null) {
            cableMgr.registerTerminal(worldPosition, level.dimension(),
                    computer.getNetworkMacs(), interfaceExitPositions);
        }

        try {
            computer.executeMain();
            EvansComputerMod.LOGGER.info("Initialized WASM terminal with module: {}", wasmModule);

            computer.startWorkerThread();
            rescanScreenCluster();
        } catch (WasmManager.WasmExecutionException e) {
            EvansComputerMod.LOGGER.error("Failed to execute WASM main", e);
        }

        setChanged();
        if (level != null && !level.isClientSide()) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    public boolean isWasmLoading() {
        return wasmLoading;
    }

    // ==================== Bulk-move transfer ====================

    /**
     * Marks the BE as participating in a bulk-move hand-off. While set,
     * {@link #setRemoved()} will not tear down the live ComputerInstance.
     * Paired with {@link #adoptComputer} on the destination BE.
     */
    public void setTransferring(boolean transferring) {
        this.transferring = transferring;
    }

    public boolean isTransferring() {
        return transferring;
    }

    /** Live state snapshot used to hand a computer off to a new BE. */
    public static final class TransferBundle {
        public final ComputerInstance computer;
        public final boolean wasmInitialized;
        @Nullable public final TerminalDisplay screenDisplay;
        @Nullable public final ScreenClusterInfo screenClusterInfo;
        public final boolean screenPowered;
        public final Map<UUID, ClientSyncState> clientSyncStates;
        public final Map<UUID, ClientSyncState> screenClientSyncStates;
        public final int[] redstoneOutput;
        public final int[] redstoneInput;
        public final long disabledFacesMask;
        @Nullable public final BlockPos[] interfaceExitPositions;

        public TransferBundle(ComputerInstance computer,
                              boolean wasmInitialized,
                              @Nullable TerminalDisplay screenDisplay,
                              @Nullable ScreenClusterInfo screenClusterInfo,
                              boolean screenPowered,
                              Map<UUID, ClientSyncState> clientSyncStates,
                              Map<UUID, ClientSyncState> screenClientSyncStates,
                              int[] redstoneOutput,
                              int[] redstoneInput,
                              long disabledFacesMask,
                              @Nullable BlockPos[] interfaceExitPositions) {
            this.computer = computer;
            this.wasmInitialized = wasmInitialized;
            this.screenDisplay = screenDisplay;
            this.screenClusterInfo = screenClusterInfo;
            this.screenPowered = screenPowered;
            this.clientSyncStates = clientSyncStates;
            this.screenClientSyncStates = screenClientSyncStates;
            this.redstoneOutput = redstoneOutput;
            this.redstoneInput = redstoneInput;
            this.disabledFacesMask = disabledFacesMask;
            this.interfaceExitPositions = interfaceExitPositions;
        }
    }

    /**
     * Snapshot the live hot state for transfer to a new BE. Does not mutate
     * this BE beyond flipping {@link #transferring} — callers should invoke
     * this from {@code beforeMove} on the source block, then pass the result
     * to {@link #adoptComputer} on the destination BE from {@code afterMove}.
     */
    @Nullable
    public TransferBundle captureForTransfer() {
        if (computer == null) return null;
        this.transferring = true;
        return new TransferBundle(
                computer,
                wasmInitialized,
                screenDisplay,
                screenClusterInfo,
                screenPowered,
                new ConcurrentHashMap<>(clientSyncStates),
                new ConcurrentHashMap<>(screenClientSyncStates),
                redstoneOutput.clone(),
                redstoneInput.clone(),
                disabledFacesMask,
                interfaceExitPositions != null ? interfaceExitPositions.clone() : null
        );
    }

    /**
     * Mount a live {@link ComputerInstance} into this BE, discarding whatever
     * this BE may have booted from its NBT snapshot. Inverse of
     * {@link #shutdownComputer()}.
     *
     * <p>Called on the destination BE by the bulk-move hook after the source
     * BE has been {@link #captureForTransfer() captured}.
     */
    public void adoptComputer(TransferBundle bundle) {
        if (level == null || level.isClientSide()) return;

        // Discard any NBT-booted instance this BE may have just spun up. Note
        // we don't want shutdownComputer()'s cable-unregister to fire because
        // the MACs are identical to the live instance we're about to mount —
        // that unregister would tear down the still-valid registration.
        if (loadingFuture != null) {
            loadingFuture.cancel(true);
            loadingFuture = null;
        }
        wasmLoading = false;
        if (computer != null && computer != bundle.computer) {
            computer.interrupt();
            computer.close();
        }

        this.computer = bundle.computer;
        this.computer.setHost(this);
        this.wasmInitialized = bundle.wasmInitialized;
        this.wasRunning = true;

        this.screenDisplay = bundle.screenDisplay;
        this.screenClusterInfo = bundle.screenClusterInfo;
        this.screenPowered = bundle.screenPowered;

        // Re-keyframe every tracked client. Sable's move drops the old BE
        // (client side) and creates a fresh one at the plot position — that
        // fresh BE has no display/clientDisplay yet, and the migrated
        // trackers would otherwise think the client was already caught up
        // and skip the keyframe, leaving the screen black.
        this.clientSyncStates.clear();
        for (Map.Entry<UUID, ClientSyncState> e : bundle.clientSyncStates.entrySet()) {
            e.getValue().needsKeyframe = true;
            this.clientSyncStates.put(e.getKey(), e.getValue());
        }
        this.screenClientSyncStates.clear();
        for (Map.Entry<UUID, ClientSyncState> e : bundle.screenClientSyncStates.entrySet()) {
            e.getValue().needsKeyframe = true;
            this.screenClientSyncStates.put(e.getKey(), e.getValue());
        }

        System.arraycopy(bundle.redstoneOutput, 0, this.redstoneOutput, 0,
                Math.min(6, bundle.redstoneOutput.length));
        this.redstoneInput = bundle.redstoneInput.clone();
        this.disabledFacesMask = bundle.disabledFacesMask;
        if (bundle.interfaceExitPositions != null) {
            this.interfaceExitPositions = bundle.interfaceExitPositions.clone();
        }

        // Re-register under the same computerId so remote refs (cables,
        // clients) resolve to this new BE.
        ComputerRegistry.register(this);

        this.transferring = false;

        setChanged();
        if (level instanceof ServerLevel) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    /**
     * Translate the migrated {@link #screenClusterInfo}'s anchor + member
     * positions by {@code delta}, then re-push cluster membership to every
     * translated member so their client-side BE data (ownerTerminal,
     * clusterAnchor) is corrected after a bulk move. Called by the sable
     * hook after {@link #adoptComputer}, as an alternative to a fresh
     * {@link #rescanScreenCluster} — rescanning during a mid-move or
     * against a half-assembled plot can find zero members and null out
     * the live {@code screenDisplay}.
     */
    /**
     * Translate every stashed {@link #interfaceExitPositions} entry by
     * {@code delta}. Called from the sable afterMove deferred task after the
     * InterfaceBlocks themselves have been moved into the plot. The positions
     * stored in {@code interfaceExitPositions} are WORLD-absolute (one per NIC
     * MAC), used by {@link CableNetworkManager} to start its BFS; without
     * translation they point at stale overworld coordinates and the BFS hits
     * air, leaving the moved computer orphaned from its cable network.
     */
    public void translateInterfaceExitPositions(net.minecraft.core.Vec3i delta) {
        if (level == null || level.isClientSide()) return;
        BlockPos[] arr = this.interfaceExitPositions;
        if (arr == null) {
            EvansComputerMod.LOGGER.info(
                    "[sable] translateInterfaceExitPositions {}: no exit positions to translate (delta={})",
                    worldPosition, delta);
            return;
        }
        BlockPos[] translated = new BlockPos[arr.length];
        int nonNull = 0;
        for (int i = 0; i < arr.length; i++) {
            translated[i] = arr[i] == null ? null : arr[i].offset(delta);
            if (translated[i] != null) nonNull++;
        }
        this.interfaceExitPositions = translated;
        EvansComputerMod.LOGGER.info(
                "[sable] translateInterfaceExitPositions {}: translated {}/{} exits by {} (first: {} -> {})",
                worldPosition, nonNull, arr.length, delta,
                arr.length > 0 ? arr[0] : null,
                translated.length > 0 ? translated[0] : null);
    }

    /**
     * Re-register this terminal's NICs with {@link CableNetworkManager} at the
     * current {@code worldPosition} / {@code level.dimension()} /
     * {@link #interfaceExitPositions}. Overwrites any prior registration under
     * the same MAC keys — a no-op if the MAC/position combination is already
     * current. Called from the sable afterMove hook after
     * {@link #adoptComputer} to pick up the post-move exit positions. Also
     * safe to call from other bulk-move paths (vanilla pistons, /setblock).
     */
    public void reregisterWithCableNetwork() {
        if (!(level instanceof ServerLevel sl)) {
            EvansComputerMod.LOGGER.info(
                    "[sable] reregisterWithCableNetwork {}: level not ServerLevel (got {}), skipping",
                    worldPosition, level);
            return;
        }
        if (computer == null || interfaceExitPositions == null) {
            EvansComputerMod.LOGGER.info(
                    "[sable] reregisterWithCableNetwork {}: skip (computer={}, exits={})",
                    worldPosition, computer != null,
                    interfaceExitPositions != null ? interfaceExitPositions.length : "null");
            return;
        }
        CableNetworkManager cableMgr = CableNetworkManager.getInstance();
        if (cableMgr == null) {
            EvansComputerMod.LOGGER.warn(
                    "[sable] reregisterWithCableNetwork {}: CableNetworkManager.getInstance()==null",
                    worldPosition);
            return;
        }
        byte[][] macs = computer.getNetworkMacs();
        EvansComputerMod.LOGGER.info(
                "[sable] reregisterWithCableNetwork {}: dim={}, macs={}, exits={}",
                worldPosition, sl.dimension(),
                macs != null ? macs.length : "null",
                java.util.Arrays.toString(interfaceExitPositions));
        cableMgr.registerTerminal(worldPosition, sl.dimension(),
                macs, interfaceExitPositions);
    }

    public void translateScreenClusterInfo(net.minecraft.core.Vec3i delta) {
        if (level == null || level.isClientSide()) return;
        ScreenClusterInfo info = screenClusterInfo;
        if (info == null) return;
        BlockPos newAnchor = info.anchor().offset(delta);
        java.util.List<BlockPos> newMembers = new java.util.ArrayList<>(info.members().size());
        for (BlockPos m : info.members()) newMembers.add(m.offset(delta));
        ScreenClusterInfo translated = new ScreenClusterInfo(
                newAnchor, info.facing(), info.cols(), info.rows(),
                info.gfxWidth(), info.gfxHeight(), newMembers);
        screenClusterInfo = translated;

        for (BlockPos member : newMembers) {
            if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                sbe.setClusterMembership(
                        worldPosition, newAnchor, info.cols(), info.rows(),
                        member.equals(newAnchor));
                sbe.setActive(screenPowered);
            }
        }
        setChanged();
    }

    /**
     * Gracefully shuts down the computer instance and cleans up resources.
     * Used by both setRemoved() (block broken) and onChunkUnloaded().
     */
    private void shutdownComputer() {
        ComputerRegistry.unregister(computerId);

        // Unregister from cable network manager
        if (computer != null) {
            CableNetworkManager cableMgr = CableNetworkManager.getInstance();
            if (cableMgr != null) {
                cableMgr.unregisterTerminal(computer.getNetworkMacs());
            }
        }

        if (loadingFuture != null) {
            loadingFuture.cancel(true);
            loadingFuture = null;
        }
        wasmLoading = false;

        if (computer != null) {
            computer.interrupt();
            computer.close();
            computer = null;
        }
        wasmInitialized = false;
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (transferring) {
            // Bulk-move hand-off in progress: preserve the live ComputerInstance
            // so the destination BE can adopt it. Keep wasRunning true so if
            // the adopt path fails we still auto-boot from the NBT snapshot.
            return;
        }
        wasRunning = false; // Block broken — don't auto-start
        shutdownComputer();
    }

    @Override
    public void onLoad() {
        super.onLoad();
        if (level != null && !level.isClientSide() && wasRunning) {
            level.getServer().execute(() -> {
                if (!isRemoved() && !wasmInitialized && !wasmLoading) {
                    EvansComputerMod.LOGGER.info("Auto-starting computer {} after chunk load", computerId);
                    initializeWasm();
                }
            });
        }
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
        // Gracefully stop the computer but keep wasRunning true (already saved to NBT)
        // so it auto-starts when the chunk reloads.
        shutdownComputer();
    }

    // ==================== Input Handling ====================

    public void onStringInput(String input) {
        if (input == null || input.isEmpty()) {
            return;
        }

        if (wasmLoading) {
            return;
        }

        if (input.contains("\u0014") && computer != null) {
            EvansComputerMod.LOGGER.info("Ctrl+T detected - interrupting WASM execution");
            computer.interrupt();
            // Queue IRQ_TERMINATE so the Rust OS resets to shell when control returns
            computer.queueInterrupt(15, "{}");
            // Don't send Ctrl+T as regular input — the epoch trap handles stopping execution
            return;
        }

        if (computer != null && computer.isWasmExecuting() && !input.contains("\u0014")) {
            String escaped = input.replace("\\", "\\\\").replace("\"", "\\\"")
                    .replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t");
            computer.queueInterrupt(1, "{\"key\":\"" + escaped + "\"}");
        }

        if (computer != null) {
            try {
                computer.sendInput(input);
            } catch (Throwable e) {
                EvansComputerMod.LOGGER.error("WASM execution error in terminal: {}", e.getMessage(), e);
            }
        }

        setChanged();
        if (level != null && !level.isClientSide()) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    public void onCharInput(char c) {
        onStringInput(String.valueOf(c));
    }

    /**
     * Handle a mouse event forwarded from the client. Gated server-side
     * on mouse capture being enabled and the display being in a
     * graphics mode; both conditions are also enforced on the client
     * but we re-check here so an untrusted packet can't drive input
     * into a program that didn't opt in.
     *
     * <p>The IRQ payload is a fixed 10-byte little-endian tuple shared
     * end-to-end (packet → ring buffer → WASI {@code mouse_poll}).
     */
    public void onMouseEvent(MouseInputPacket pkt) {
        if (computer == null || !computer.isMouseCaptureEnabled()) return;
        if (display.getDisplayMode() < 1) return;
        int gw = display.getGfxWidth();
        int gh = display.getGfxHeight();
        if (gw <= 0 || gh <= 0) return;

        short x = (short) Math.max(0, Math.min(gw - 1, pkt.x()));
        short y = (short) Math.max(0, Math.min(gh - 1, pkt.y()));

        byte[] payload = new byte[10];
        java.nio.ByteBuffer.wrap(payload)
                .order(java.nio.ByteOrder.LITTLE_ENDIAN)
                .put(pkt.kind())
                .putShort(x)
                .putShort(y)
                .put(pkt.buttons())
                .put(pkt.buttonCode())
                .put(pkt.scrollDir());
        computer.queueInterrupt(/*IRQ_MOUSE*/ 4, payload);
    }

    @Deprecated
    protected void onLineInput(String line) {
        EvansComputerMod.LOGGER.info("Terminal input: {}", line);
        if (computer != null) {
            computer.sendInput(line);
        }
    }

    /**
     * Gets the computer instance for this terminal.
     */
    @Nullable
    public ComputerInstance getComputer() {
        return computer;
    }

    // ==================== Neighbor / Redstone ====================

    public void onNeighborChanged() {
        updateRedstoneInput();
        rescanScreenCluster();
    }

    /**
     * Re-discover the attached screen cluster by looking at adjacent blocks.
     * Updates {@link #screenDisplay}, {@link #screenClusterInfo}, and the
     * cluster membership on every member ScreenBlockEntity. No-op on client.
     */
    public void rescanScreenCluster() {
        if (level == null || level.isClientSide()) return;

        Direction terminalFacing = getBlockState().getValue(TerminalBlock.FACING);
        ScreenClusterDiscovery.ClusterResult result =
                ScreenClusterDiscovery.discover(level, worldPosition, terminalFacing);

        // Clear membership on previous-cluster screens that are no longer members
        ScreenClusterInfo oldInfo = screenClusterInfo;
        java.util.Set<BlockPos> oldMembers = oldInfo != null
                ? new java.util.HashSet<>(oldInfo.members())
                : java.util.Collections.emptySet();
        java.util.Set<BlockPos> newMembers = result != null
                ? new java.util.HashSet<>(result.members())
                : java.util.Collections.emptySet();

        for (BlockPos member : oldMembers) {
            if (!newMembers.contains(member)) {
                if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                    sbe.clearCluster();
                }
            }
        }

        if (result == null) {
            screenDisplay = null;
            screenClusterInfo = null;
            screenClientSyncStates.clear();
            screenPowered = false;
            if (computer != null) {
                computer.writeScreenHeader(0, 0);
            }
            setChanged();
            return;
        }

        // Rectangle is valid — set up cluster. Cap the framebuffer at
        // SCREEN_MAX_GFX_W × SCREEN_MAX_GFX_H while preserving the
        // cluster's aspect ratio. The renderer quad is sized from
        // cols × rows world blocks with UVs (0,0)→(1,1), so the GPU
        // stretches the capped texture up to fill the full cluster.
        int idealW = result.cols() * SCREEN_TILE_GFX_W;
        int idealH = result.rows() * SCREEN_TILE_GFX_H;
        double scale = Math.min(1.0, Math.min(
                (double) SCREEN_MAX_GFX_W / idealW,
                (double) SCREEN_MAX_GFX_H / idealH));
        int gfxW = Math.max(1, (int) Math.round(idealW * scale));
        int gfxH = Math.max(1, (int) Math.round(idealH * scale));
        ScreenClusterInfo info = new ScreenClusterInfo(
                result.anchor(), result.facing(), result.cols(), result.rows(),
                gfxW, gfxH, result.members());
        screenClusterInfo = info;

        // (Re)allocate display if dimensions changed. Reset the power
        // state on any reform so the Rust OS must explicitly turn the
        // monitor back on before content reappears.
        TerminalDisplay sd = screenDisplay;
        if (sd == null || sd.getGfxWidth() != gfxW || sd.getGfxHeight() != gfxH) {
            sd = new TerminalDisplay(1, 1);
            screenDisplay = sd;
            screenClientSyncStates.clear();
            screenPowered = false;
            // Size the snapshot scratch buffer for the new cluster. The
            // sync path copies pixel data into this buffer under
            // synchronized(screenDisplay) so the diff runs against a
            // private snapshot. Sized for the worst-case bytes-per-pixel
            // (RGBA8888 = 4) so a later format switch doesn't have to
            // reallocate; sendScreenDelta also grows it on demand if a
            // larger cluster appears.
            int pixBytes = gfxW * gfxH * 4;
            if (screenSnapshotPixels == null || screenSnapshotPixels.length < pixBytes) {
                screenSnapshotPixels = new byte[pixBytes];
            }
        }

        // Tell every member its cluster role and its active flag.
        // active = clusterValid && screenPowered. clusterValid is true
        // here because we're in the success branch; screenPowered drives
        // whether the content quad is rendered.
        for (BlockPos member : result.members()) {
            if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                sbe.setClusterMembership(
                        worldPosition,
                        result.anchor(),
                        result.cols(),
                        result.rows(),
                        member.equals(result.anchor()));
                sbe.setActive(screenPowered);
            }
        }

        // Write dimensions into the WASM screen header so the Rust OS sees them.
        if (computer != null) {
            computer.writeScreenHeader(gfxW, gfxH);
        }

        setChanged();
    }

    /**
     * Re-apply {@code active = clusterValid && screenPowered} to every
     * current cluster member by flipping their BE-level {@code active}
     * flag. Called when the power state flips.
     */
    private void applyClusterActiveState() {
        ScreenClusterInfo info = screenClusterInfo;
        if (info == null || level == null) return;
        boolean active = screenPowered; // cluster is valid iff info != null
        for (BlockPos member : info.members()) {
            if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                sbe.setActive(active);
            }
        }
    }

    /**
     * Set the cluster's powered state. Called from the
     * {@code screen_set_power} host function (on the WASM worker thread).
     * The actual level/BE mutations are scheduled on the server thread
     * so they don't race MC's chunk/BE internal locks — matching the
     * pattern used by {@link #syncToClients}. Without this, the cascade
     * of {@code sendBlockUpdated} → {@code neighborChanged} →
     * {@code rescanScreenCluster} runs on the worker thread and deadlocks.
     */
    public void setScreenPower(boolean powered) {
        if (level == null || level.isClientSide()) return;
        level.getServer().execute(() -> {
            if (screenClusterInfo == null) return;
            if (screenPowered == powered) return;
            screenPowered = powered;
            applyClusterActiveState();
            // Force a keyframe so the client state re-syncs with the new
            // power state (especially important for the first power-on,
            // where the shadow and display have diverged).
            for (ClientSyncState state : screenClientSyncStates.values()) {
                state.needsKeyframe = true;
            }
            setChanged();
        });
    }

    private void updateRedstoneInput() {
        if (level == null || level.isClientSide()) return;

        int[] oldInput = redstoneInput;
        int[] newInput = new int[6];
        boolean changed = false;

        for (Direction dir : Direction.values()) {
            int side = dir.ordinal();
            newInput[side] = level.getSignal(worldPosition.relative(dir), dir);
            if (newInput[side] != oldInput[side]) {
                changed = true;
            }
        }

        redstoneInput = newInput;

        if (changed && computer != null) {
            StringBuilder json = new StringBuilder();
            json.append("{\"sides\":[");
            for (int i = 0; i < 6; i++) {
                if (i > 0) json.append(",");
                json.append(newInput[i]);
            }
            json.append("],\"old_sides\":[");
            for (int i = 0; i < 6; i++) {
                if (i > 0) json.append(",");
                json.append(oldInput[i]);
            }
            json.append("]}");
            computer.queueInterrupt(2, json.toString());
        }
    }

    public int getRedstoneInput(int absoluteSide) {
        return (absoluteSide >= 0 && absoluteSide < 6) ? redstoneInput[absoluteSide] : 0;
    }

    // ==================== Properties ====================

    public boolean isCharacterMode() { return characterMode; }
    public void setCharacterMode(boolean characterMode) { this.characterMode = characterMode; }

    public String getWasmModule() { return wasmModule; }
    public void setWasmModule(String wasmModule) { this.wasmModule = wasmModule; }

    public String getWasmFunction() { return wasmFunction; }
    public void setWasmFunction(String wasmFunction) { this.wasmFunction = wasmFunction; }

    public void setRedstoneOutput(int side, int power) {
        if (side >= 0 && side < 6) {
            int oldPower = redstoneOutput[side];
            redstoneOutput[side] = Math.max(0, Math.min(15, power));

            if (oldPower != redstoneOutput[side] && level != null && !level.isClientSide()) {
                setChanged();
                level.updateNeighborsAt(worldPosition, getBlockState().getBlock());
            }
        }
    }

    public int getRedstoneOutput(int side) {
        return (side >= 0 && side < 6) ? redstoneOutput[side] : 0;
    }

    // ==================== NBT Serialization ====================

    //? if >=26.1 {
    @Override
    protected void saveAdditional(ValueOutput output) {
        super.saveAdditional(output);
        output.store("computerId", UUIDUtil.CODEC, computerId);

        // Save framebuffer as Base64-encoded string
        byte[] fbData = display.toBytes();
        output.putString("framebuffer", java.util.Base64.getEncoder().encodeToString(fbData));

        output.putBoolean("characterMode", characterMode);
        output.putString("wasmModule", wasmModule);
        output.putString("wasmFunction", wasmFunction);
        output.putIntArray("redstoneOutput", redstoneOutput);

        output.putBoolean("wasRunning", wasmInitialized && computer != null && !computer.isFaulted());
    }

    @Override
    protected void loadAdditional(ValueInput input) {
        super.loadAdditional(input);
        input.read("computerId", UUIDUtil.CODEC).ifPresent(id -> computerId = id);

        // Load framebuffer from Base64-encoded string
        input.getString("framebuffer").ifPresent(encoded -> {
            display.setFromBytes(java.util.Base64.getDecoder().decode(encoded));
        });

        characterMode = input.getBooleanOr("characterMode", false);
        wasmModule = input.getStringOr("wasmModule", "terminal_os");
        wasmFunction = input.getStringOr("wasmFunction", "main");
        wasRunning = input.getBooleanOr("wasRunning", false);

        input.getIntArray("redstoneOutput").ifPresent(saved -> {
            System.arraycopy(saved, 0, redstoneOutput, 0, Math.min(saved.length, 6));
        });
    }
    //?} else {
    /*@Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        UUIDUtil.CODEC.encodeStart(net.minecraft.nbt.NbtOps.INSTANCE, computerId)
                .result().ifPresent(t -> tag.put("computerId", t));

        byte[] fbData = display.toBytes();
        tag.putString("framebuffer", java.util.Base64.getEncoder().encodeToString(fbData));

        tag.putBoolean("characterMode", characterMode);
        tag.putString("wasmModule", wasmModule);
        tag.putString("wasmFunction", wasmFunction);
        tag.putIntArray("redstoneOutput", redstoneOutput);

        tag.putBoolean("wasRunning", wasmInitialized && computer != null && !computer.isFaulted());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("computerId")) {
            UUIDUtil.CODEC.parse(net.minecraft.nbt.NbtOps.INSTANCE, tag.get("computerId"))
                    .result().ifPresent(id -> computerId = id);
        }

        if (tag.contains("framebuffer")) {
            String encoded = tag.getString("framebuffer");
            display.setFromBytes(java.util.Base64.getDecoder().decode(encoded));
        }

        if (tag.contains("characterMode")) characterMode = tag.getBoolean("characterMode");
        if (tag.contains("wasmModule")) wasmModule = tag.getString("wasmModule");
        else wasmModule = "terminal_os";
        if (tag.contains("wasmFunction")) wasmFunction = tag.getString("wasmFunction");
        else wasmFunction = "main";
        wasRunning = tag.getBoolean("wasRunning");

        if (tag.contains("redstoneOutput")) {
            int[] saved = tag.getIntArray("redstoneOutput");
            System.arraycopy(saved, 0, redstoneOutput, 0, Math.min(saved.length, 6));
        }
    }*/
    //?}

    // ==================== Network Sync ====================

    @Nullable
    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        return saveWithoutMetadata(registries);
    }

    // ==================== MenuProvider ====================

    @Override
    public Component getDisplayName() {
        return Component.translatable("container.evanscomputermod.terminal");
    }

    @Nullable
    @Override
    public AbstractContainerMenu createMenu(int containerId, Inventory playerInventory, Player player) {
        return new TerminalMenu(containerId, playerInventory, this);
    }

    // ==================== Visual Programming ====================

    public void runVisualScript(String pythonCode) {
        if (level == null || level.isClientSide()) return;
        if (computer == null) return;

        java.nio.file.Path computerDir = java.nio.file.Paths.get("computer-data", computerId.toString());
        try {
            java.nio.file.Files.createDirectories(computerDir);
            java.nio.file.Files.writeString(computerDir.resolve("visual_program.py"), pythonCode);
        } catch (java.io.IOException e) {
            EvansComputerMod.LOGGER.error("Failed to write visual_program.py", e);
            return;
        }

        computer.sendInput("python visual_program.py\n");
    }

    public void saveVisualProgram(String fileName, String jsonContent) {
        if (level == null || level.isClientSide()) return;
        String sanitized = sanitizeVisualFileName(fileName);
        if (sanitized.isEmpty()) return;

        java.nio.file.Path visualDir = java.nio.file.Paths.get("computer-data", computerId.toString(), "visual");
        try {
            java.nio.file.Files.createDirectories(visualDir);
            java.nio.file.Files.writeString(visualDir.resolve(sanitized + ".vpl"), jsonContent);
        } catch (java.io.IOException e) {
            EvansComputerMod.LOGGER.error("Failed to save visual program '{}'", sanitized, e);
        }
    }

    public List<String> listVisualPrograms() {
        java.nio.file.Path visualDir = java.nio.file.Paths.get("computer-data", computerId.toString(), "visual");
        List<String> result = new ArrayList<>();
        if (!java.nio.file.Files.isDirectory(visualDir)) return result;

        try (var stream = java.nio.file.Files.list(visualDir)) {
            stream.filter(p -> p.toString().endsWith(".vpl"))
                  .map(p -> p.getFileName().toString().replace(".vpl", ""))
                  .sorted()
                  .forEach(result::add);
        } catch (java.io.IOException e) {
            EvansComputerMod.LOGGER.error("Failed to list visual programs", e);
        }
        return result;
    }

    @Nullable
    public String loadVisualProgram(String fileName) {
        String sanitized = sanitizeVisualFileName(fileName);
        if (sanitized.isEmpty()) return null;

        java.nio.file.Path file = java.nio.file.Paths.get("computer-data", computerId.toString(), "visual", sanitized + ".vpl");
        if (!java.nio.file.Files.exists(file)) return null;

        try {
            return java.nio.file.Files.readString(file);
        } catch (java.io.IOException e) {
            EvansComputerMod.LOGGER.error("Failed to load visual program '{}'", sanitized, e);
            return null;
        }
    }

    private static String sanitizeVisualFileName(String name) {
        String sanitized = name.replaceAll("[^a-zA-Z0-9_\\-]", "");
        if (sanitized.length() > 64) sanitized = sanitized.substring(0, 64);
        return sanitized;
    }

    // ==================== Interface Discovery & Link State ====================

    private byte[][] discoverInterfaces() {
        java.util.List<byte[]> macs = new java.util.ArrayList<>();
        java.util.List<net.minecraft.core.BlockPos> exitPosns = new java.util.ArrayList<>();

        // Determine screen face (the FACING direction of the terminal)
        net.minecraft.core.Direction screenFace = getBlockState().getValue(TerminalBlock.FACING);

        // Check which faces have InterfaceBlocks
        java.util.Set<net.minecraft.core.Direction> occupiedByInterface = new java.util.HashSet<>();
        if (level != null) {
            for (net.minecraft.core.Direction dir : net.minecraft.core.Direction.values()) {
                net.minecraft.core.BlockPos neighbor = worldPosition.relative(dir);
                if (level.getBlockState(neighbor).getBlock() instanceof com.example.evanscomputermod.block.InterfaceBlock) {
                    occupiedByInterface.add(dir);
                }
            }
        }

        // Built-in interfaces: one per face, EXCLUDING screen face and InterfaceBlock-occupied faces
        for (net.minecraft.core.Direction dir : net.minecraft.core.Direction.values()) {
            if (dir == screenFace) continue;  // Skip screen face
            if (occupiedByInterface.contains(dir)) continue;  // Skip occupied faces
            macs.add(com.example.evanscomputermod.computer.NetworkHub.deriveMac(computerId, macs.size()));
            exitPosns.add(worldPosition.relative(dir));  // Exit is the adjacent block
        }

        if (level == null) {
            this.interfaceExitPositions = exitPosns.toArray(new BlockPos[0]);
            return macs.toArray(new byte[0][]);
        }

        // BFS for attached InterfaceBlocks
        java.util.Set<net.minecraft.core.BlockPos> visited = new java.util.HashSet<>();
        visited.add(worldPosition);
        java.util.Queue<net.minecraft.core.BlockPos> queue = new java.util.LinkedList<>();

        for (net.minecraft.core.Direction dir : occupiedByInterface) {
            net.minecraft.core.BlockPos neighbor = worldPosition.relative(dir);
            queue.add(neighbor);
            visited.add(neighbor);
        }

        while (!queue.isEmpty()) {
            net.minecraft.core.BlockPos pos = queue.poll();
            for (net.minecraft.core.Direction dir : net.minecraft.core.Direction.values()) {
                net.minecraft.core.BlockPos facePos = pos.relative(dir);
                if (visited.contains(facePos)) continue;
                net.minecraft.world.level.block.Block block = level.getBlockState(facePos).getBlock();
                if (block instanceof com.example.evanscomputermod.block.InterfaceBlock) {
                    visited.add(facePos);
                    queue.add(facePos);
                } else {
                    // Free face = new interface. Exit position is facePos (the block adjacent to the free face)
                    macs.add(com.example.evanscomputermod.computer.NetworkHub.deriveMac(computerId, macs.size()));
                    exitPosns.add(facePos);
                }
            }
        }

        this.interfaceExitPositions = exitPosns.toArray(new BlockPos[0]);
        return macs.toArray(new byte[0][]);
    }

    public void updateDisabledFaces(int ifaceIndex, boolean up) {
        if (level == null || level.isClientSide()) return;
        // Re-set our own block state to trigger updateShape() on all adjacent blocks.
        // neighborChanged() alone does NOT trigger updateShape — only setBlock does.
        BlockState state = level.getBlockState(worldPosition);
        level.setBlock(worldPosition, state, 3);
    }

    public boolean isFaceDisabled(int directionOrdinal) {
        return (disabledFacesMask & (1L << directionOrdinal)) != 0;
    }

    public void setFaceDisabled(int directionOrdinal, boolean disabled) {
        if (disabled) {
            disabledFacesMask |= (1L << directionOrdinal);
        } else {
            disabledFacesMask &= ~(1L << directionOrdinal);
        }
    }

    public void openVisualEditor() {
        if (level != null && !level.isClientSide()) {
            var packet = new com.example.evanscomputermod.network.OpenVisualEditorPacket(getBlockPos());
            //? if >=26.1 {
            var chunkPos = net.minecraft.world.level.ChunkPos.containing(getBlockPos());
            //?} else
            /*var chunkPos = new net.minecraft.world.level.ChunkPos(getBlockPos());*/
            net.neoforged.neoforge.network.PacketDistributor.sendToPlayersTrackingChunk(
                    (net.minecraft.server.level.ServerLevel) level, chunkPos, packet);
        }
    }
}
