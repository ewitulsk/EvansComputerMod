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
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
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
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;

import org.jspecify.annotations.Nullable;
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

    // Per-client delta sync state for the screen cluster display
    private final Map<UUID, ClientSyncState> screenClientSyncStates = new ConcurrentHashMap<>();

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
     * Sync display to all tracking players using delta protocol.
     * Each player has independent sync state for optimal bandwidth.
     */
    private void syncDeltaToClients(ServerLevel serverLevel) {
        List<ServerPlayer> players = serverLevel.getPlayers(
                p -> p.distanceToSqr(worldPosition.getX(), worldPosition.getY(), worldPosition.getZ()) < 64 * 64
        );

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
        TerminalDeltaPacket packet = TerminalDeltaPacket.createKeyframe(
                worldPosition, display, gen, state.deflater, TerminalDeltaPacket.TARGET_TERMINAL);
        PacketDistributor.sendToPlayer(player, packet);
        state.tracker.commitShadow(display);
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

        List<ServerPlayer> players = serverLevel.getPlayers(
                p -> p.distanceToSqr(worldPosition.getX(), worldPosition.getY(), worldPosition.getZ()) < 64 * 64
        );

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
        TerminalDeltaPacket packet = TerminalDeltaPacket.createKeyframe(
                worldPosition, sd, gen, state.deflater, TerminalDeltaPacket.TARGET_SCREEN);
        PacketDistributor.sendToPlayer(player, packet);
        state.tracker.commitShadow(sd);
        state.markKeyframeSent(gen);
    }

    private void sendScreenDelta(ServerPlayer player, ClientSyncState state, TerminalDisplay sd) {
        FramebufferDiffTracker.TextDelta textDelta = state.tracker.computeTextDelta(sd);
        FramebufferDiffTracker.GfxDelta gfxDelta = state.tracker.computeGfxDelta(sd);

        int mode = sd.getDisplayMode();
        int shadowMode = state.tracker.getShadowDisplayMode();
        if (mode != shadowMode) {
            sendScreenKeyframe(player, state, sd);
            return;
        }

        boolean textChanged = !textDelta.changedRowIndices().isEmpty() || textDelta.scrollOffset() != 0;
        boolean gfxChanged = gfxDelta != null && (!gfxDelta.changedTileIndices().isEmpty() || gfxDelta.paletteChanged());
        if (!textChanged && !gfxChanged) return;

        TerminalDeltaPacket packet = TerminalDeltaPacket.createDelta(
                worldPosition, textDelta.generation(), mode,
                textDelta, gfxDelta, sd.getWidth(), state.deflater,
                TerminalDeltaPacket.TARGET_SCREEN);
        PacketDistributor.sendToPlayer(player, packet);
        state.tracker.commitShadow(sd);
        state.markSent(textDelta.generation());
    }

    private void sendDelta(ServerPlayer player, ClientSyncState state) {
        FramebufferDiffTracker.TextDelta textDelta = state.tracker.computeTextDelta(display);
        FramebufferDiffTracker.GfxDelta gfxDelta = null;

        int mode = display.getDisplayMode();
        int shadowMode = state.tracker.getShadowDisplayMode();
        // Compute gfx delta if graphics mode is active or was just deactivated
        if (mode >= 1 || shadowMode >= 1) {
            gfxDelta = state.tracker.computeGfxDelta(display);
        }

        // Display mode changed — send a keyframe so client gets full GFX state
        // (delta packets don't include gfxWidth/gfxHeight, so the client can't
        // allocate pixel buffers from a delta alone)
        boolean modeChanged = mode != shadowMode;
        if (modeChanged) {
            sendKeyframe(player, state);
            return;
        }

        // Skip if nothing changed
        boolean textChanged = !textDelta.changedRowIndices().isEmpty() || textDelta.scrollOffset() != 0;
        boolean gfxChanged = gfxDelta != null && (!gfxDelta.changedTileIndices().isEmpty() || gfxDelta.paletteChanged());
        if (!textChanged && !gfxChanged) return;

        TerminalDeltaPacket packet = TerminalDeltaPacket.createDelta(
                worldPosition, textDelta.generation(), mode,
                textDelta, gfxDelta, display.getWidth(), state.deflater,
                TerminalDeltaPacket.TARGET_TERMINAL);
        PacketDistributor.sendToPlayer(player, packet);
        state.tracker.commitShadow(display);
        state.markSent(textDelta.generation());
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
        if (isRemoved()) {
            if (instance != null) {
                instance.close();
            }
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
            computer.rescanPeripherals();
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
        if (computer != null) {
            computer.rescanPeripherals();
        }
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
                setScreenActiveBlockState(member, false);
            }
        }

        if (result == null) {
            screenDisplay = null;
            screenClusterInfo = null;
            screenClientSyncStates.clear();
            if (computer != null) {
                computer.writeScreenHeader(0, 0);
            }
            setChanged();
            return;
        }

        // Rectangle is valid — set up cluster
        int gfxW = result.cols() * SCREEN_TILE_GFX_W;
        int gfxH = result.rows() * SCREEN_TILE_GFX_H;
        ScreenClusterInfo info = new ScreenClusterInfo(
                result.anchor(), result.facing(), result.cols(), result.rows(),
                gfxW, gfxH, result.members());
        screenClusterInfo = info;

        // (Re)allocate display if dimensions changed
        TerminalDisplay sd = screenDisplay;
        if (sd == null || sd.getGfxWidth() != gfxW || sd.getGfxHeight() != gfxH) {
            sd = new TerminalDisplay(1, 1);
            screenDisplay = sd;
            screenClientSyncStates.clear();
        }

        // Tell every member its cluster role
        for (BlockPos member : result.members()) {
            if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                sbe.setClusterMembership(
                        worldPosition,
                        result.anchor(),
                        result.cols(),
                        result.rows(),
                        member.equals(result.anchor()));
            }
            setScreenActiveBlockState(member, true);
        }

        // Write dimensions into the WASM screen header so the Rust OS sees them.
        if (computer != null) {
            computer.writeScreenHeader(gfxW, gfxH);
        }

        setChanged();
    }

    /**
     * Flip a screen block's {@link ScreenBlock#ACTIVE} blockstate property.
     * No-op if the target block isn't a ScreenBlock or already has the
     * desired value (guards against redundant updates that would spam
     * neighbor change events).
     */
    private void setScreenActiveBlockState(BlockPos pos, boolean active) {
        if (level == null) return;
        BlockState state = level.getBlockState(pos);
        if (!(state.getBlock() instanceof ScreenBlock)) return;
        if (state.getValue(ScreenBlock.ACTIVE) == active) return;
        level.setBlock(pos, state.setValue(ScreenBlock.ACTIVE, active), 3);
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
            var chunkPos = net.minecraft.world.level.ChunkPos.containing(getBlockPos());
            net.neoforged.neoforge.network.PacketDistributor.sendToPlayersTrackingChunk(
                    (net.minecraft.server.level.ServerLevel) level, chunkPos, packet);
        }
    }
}
