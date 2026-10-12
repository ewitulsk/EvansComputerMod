package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.*;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;
import com.example.evanscomputermod.computer.peripheral.PeripheralHub;
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.module.InstalledModules;
import com.example.evanscomputermod.module.ModuleBays;
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
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.state.BlockState;

import com.example.evanscomputermod.computer.DisplaySyncHub;
import com.example.evanscomputermod.network.ClientDisplayCache;
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

    // The text grid, graphics limits, Screen tile size and wall cap are not
    // constants any more: they come from the installed display panel's tier
    // (computer/display/PanelTier) and, for a Screen wall, from its lowest
    // block's tier. The kernel boots at 160x50 and is resized to the panel.

    // Framebuffer display state — stores cell data read from WASM memory
    // (its size follows the kernel's framebuffer header).
    private final TerminalDisplay display = new TerminalDisplay(160, 50);

    /**
     * The display panel in the screen slot (a DisplayPanelItem stack; empty =
     * no panel, the computer runs headless). Terminals saved before panels
     * existed have no {@code panel} tag and keep this default: Tier 2, the
     * limits from before tiers.
     */
    private ItemStack panel = defaultPanel();

    /** The panel of a Terminal saved before panels existed: Tier 2. */
    private static ItemStack defaultPanel() {
        return new ItemStack(com.example.evanscomputermod.item.ModItems.COLOR_LCD_PANEL.get());
    }

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
            java.util.List<BlockPos> members,
            /** The wall's tier: its lowest block's. */
            com.example.evanscomputermod.computer.display.PanelTier tier,
            /** A block holding the wall back (lowest tier while others are higher), or null if all match. */
            @Nullable BlockPos limitedBy
    ) {}

    @Nullable
    private volatile ScreenClusterInfo screenClusterInfo;

    // True when the screen cluster is powered on. Independent of cluster
    // validity: a valid cluster with `screenPowered == false` still
    // displays the "no signal" inactive face texture and the BER skips
    // rendering. Flipped by the Rust OS via the `screen_set_power` host
    // function (and implicitly by `screen::init()`).
    private volatile boolean screenPowered = false;

    // Streams the screen cluster display to nearby players (one delta per frame, shared)
    private DisplaySyncHub screenSync = new DisplaySyncHub(DISPLAY_ATTACHED);

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

    // Streams the terminal's own display to players with its GUI open
    private DisplaySyncHub terminalSync = new DisplaySyncHub(DISPLAY_OWN);

    // True while this BE is in the middle of a bulk-move hand-off (e.g. sable
    // assembling the block into a physics sublevel). When set, setRemoved()
    // skips shutdownComputer() so the live ComputerInstance can be adopted by
    // the freshly-created BE at the destination. Cleared by adoptComputer()
    // after the new BE has taken ownership.
    private volatile boolean transferring = false;

    // ==================== Peripherals & module bays ====================

    private final PeripheralHub peripheralHub = new PeripheralHub(new PeripheralHub.Owner() {
        @Override
        public Level level() {
            return level;
        }

        @Override
        public BlockPos pos() {
            return worldPosition;
        }

        @Override
        public Direction facing() {
            return getBlockState().getValue(TerminalBlock.FACING);
        }

        @Override
        public UUID computerId() {
            return computerId;
        }

        @Override
        @Nullable
        public PeripheralEventBus events() {
            ComputerInstance c = computer;
            return c != null ? c.getPeripheralEvents() : null;
        }
    });

    private final com.example.evanscomputermod.controller.WirelessControllerHub controllers =
            new com.example.evanscomputermod.controller.WirelessControllerHub(peripheralHub);

    private final ModuleBays moduleBays = new ModuleBays(new ModuleBays.Owner() {
        @Override
        public Level level() {
            return level;
        }

        @Override
        public com.example.evanscomputermod.radio.api.Pose pose() {
            return com.example.evanscomputermod.module.BlockPoses.of(level, worldPosition);
        }

        @Override
        public BlockPos blockPos() {
            return worldPosition;
        }

        @Override
        public Direction facing() {
            return getBlockState().getValue(TerminalBlock.FACING);
        }

        @Override
        public com.example.evanscomputermod.controller.WirelessControllerHub controllers() {
            return controllers;
        }

        @Override
        public boolean isAlive() {
            return level != null && !isRemoved();
        }

        @Override
        public void onBaysChanged() {
            updateModuleVisuals();
            setChanged();
        }

        @Override
        public void onModuleChanged(String slotName, @Nullable IComputerModule module) {
            peripheralHub.setModule(slotName, module);
        }
    });

    /** Server: bays are live and sides scanned. Started on the first tick in the world, stopped on removal/unload. */
    private boolean peripheralsStarted = false;

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
        if (!peripheralsStarted) {
            startPeripherals();
        }
        moduleBays.tick();
        controllers.tick(System.currentTimeMillis());
        if (computer != null) {
            computer.tickSync();
        }
    }

    // ==================== Peripherals & module bays ====================

    @Override
    public PeripheralHub getPeripheralHub() {
        return peripheralHub;
    }

    /** Wireless controllers connected to this computer. Server side. */
    @Override
    public com.example.evanscomputermod.controller.WirelessControllerHub getControllers() {
        return controllers;
    }

    public ModuleBays getModuleBays() {
        return moduleBays;
    }

    /** Put an item's saved cards and modules into this (freshly placed) computer. */
    public void restoreModules(InstalledModules modules) {
        moduleBays.restore(modules);
        updateModuleVisuals();
        setChanged();
    }

    // ==================== Display panel ====================

    /** The installed display panel (a copy; empty = none). */
    public ItemStack getPanel() {
        return panel.copy();
    }

    /** Tier of the installed panel, or null when the computer runs headless. */
    @Nullable
    public com.example.evanscomputermod.computer.display.PanelTier getPanelTier() {
        return com.example.evanscomputermod.item.DisplayPanelItem.tierOf(panel);
    }

    /**
     * Put {@code stack} in the screen slot (empty: take the panel out) and
     * return what was there. Hot-plug: a running computer's console is resized
     * to the new tier's text grid (no reboot) and a program whose framebuffer
     * no longer fits loses the display. Server side.
     */
    public ItemStack installPanel(ItemStack stack) {
        ItemStack old = panel;
        panel = stack.isEmpty() ? ItemStack.EMPTY : stack.copyWithCount(1);
        updateModuleVisuals();
        applyPanelToComputer();
        setChanged();
        return old;
    }

    /** Put an item's saved panel into this (freshly placed) computer. */
    public void restorePanel(ItemStack stack) {
        panel = stack.isEmpty() ? ItemStack.EMPTY : stack.copyWithCount(1);
        updateModuleVisuals();
        applyPanelToComputer();
        setChanged();
    }

    /**
     * Tell the running computer about the panel: resize the kernel console to
     * the tier's text grid and re-check program framebuffers. Without a panel
     * the console keeps its size (nothing shows it; SSH and programs go on).
     */
    private void applyPanelToComputer() {
        ComputerInstance c = computer;
        if (c == null) return;
        var tier = getPanelTier();
        if (tier != null) {
            var t = tier.modes().terminal();
            c.requestConsoleResize(t.textCols(), t.textRows());
        }
        c.displayDevice(ComputerInstance.GFX_TARGET_TERMINAL).enforceLimits();
        forceNextKeyframe();
    }

    @Override
    @Nullable
    public com.example.evanscomputermod.computer.display.PanelTier displayTier(int displayId) {
        if (displayId == DISPLAY_OWN) return getPanelTier();
        if (displayId == DISPLAY_ATTACHED) {
            ScreenClusterInfo info = screenClusterInfo;
            return info != null ? info.tier() : null;
        }
        return null;
    }

    @Override
    public com.example.evanscomputermod.computer.display.PanelTier.Tone displayTone(int displayId) {
        return displayId == DISPLAY_OWN ? com.example.evanscomputermod.item.DisplayPanelItem.toneOf(panel)
                : com.example.evanscomputermod.computer.display.PanelTier.Tone.GREEN;
    }

    /**
     * Bring bay modules to life and look for block peripherals. Runs from the
     * first server tick rather than onLoad, so it also covers block entities
     * created by a Sable move and neighbours that load in the same tick.
     */
    private void startPeripherals() {
        if (level == null || level.isClientSide() || isRemoved()) return;
        peripheralsStarted = true;
        moduleBays.load();
        peripheralHub.rescanSides();
        updateModuleVisuals(); // the panel too, for block states saved before panels
    }

    /** Take modules out of the world and detach everything. Idempotent. */
    private void stopPeripherals() {
        controllers.disconnectAll();
        if (!peripheralsStarted) return;
        peripheralsStarted = false;
        moduleBays.unload();
        peripheralHub.detachAll();
    }

    /** Make the block state show the installed cards and modules. Server side. */
    private void updateModuleVisuals() {
        if (level == null || level.isClientSide() || isRemoved()) return;
        BlockState current = getBlockState();
        if (!(current.getBlock() instanceof TerminalBlock)) return;
        BlockState next = TerminalBlock.withPanel(TerminalBlock.withModules(current, moduleBays), panel);
        if (next != current) {
            level.setBlock(worldPosition, next, Block.UPDATE_ALL);
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
        terminalSync.forceKeyframe();
        screenSync.forceKeyframe();
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

    /** The screen cluster as the computer sees it: display id {@link #DISPLAY_ATTACHED}. */
    private final IAttachedDisplay attachedScreen = new IAttachedDisplay() {
        @Override
        public TerminalDisplay display() {
            return screenDisplay;
        }

        @Override
        public int gfxWidth() {
            ScreenClusterInfo info = screenClusterInfo;
            return info != null ? info.gfxWidth() : 0;
        }

        @Override
        public int gfxHeight() {
            ScreenClusterInfo info = screenClusterInfo;
            return info != null ? info.gfxHeight() : 0;
        }

        @Override
        public void setPowered(boolean on) {
            setScreenPower(on);
        }

        @Override
        public com.example.evanscomputermod.computer.display.PanelTier tier() {
            ScreenClusterInfo info = screenClusterInfo;
            return info != null ? info.tier() : com.example.evanscomputermod.computer.display.PanelTier.COLOR_LCD;
        }
    };

    /**
     * What the attached Screen wall runs at, for players: size in blocks, tier,
     * resolution, and the block holding it back when the wall is mixed
     * ("Screen wall 2x2: Tier 1 Phosphor, 128x72, limited by the Phosphor
     * Screen at x, y, z"). Null when no wall is attached.
     */
    @Nullable
    public Component describeScreenWall() {
        ScreenClusterInfo info = screenClusterInfo;
        if (info == null) return null;
        Component tierName = Component.translatable("panel_tier.evanscomputermod." + info.tier().id());
        Component base = Component.translatable("message.evanscomputermod.screen_wall",
                info.cols(), info.rows(), info.tier().tier(), tierName, info.gfxWidth(), info.gfxHeight());
        if (info.limitedBy() == null || level == null) return base;
        BlockPos p = info.limitedBy();
        return Component.translatable("message.evanscomputermod.screen_wall_limited", base,
                level.getBlockState(p).getBlock().getName(), p.getX(), p.getY(), p.getZ());
    }

    /**
     * A player clicked the wall's display face at {@code hit} (a point on the
     * face of member {@code block}): on a touch wall (Tier 3) this is a tap,
     * delivered as a pointer down + up at that pixel through the same path as
     * the GUI's {@code MouseInputPacket} (computer UUID, display
     * {@link #DISPLAY_ATTACHED}, the host's range check). Returns the pixel,
     * or null if it wasn't delivered (no touch, not the display face, out of
     * range, nothing captured the pointer still counts as delivered).
     */
    @Nullable
    public int[] touchScreenWall(ServerPlayer player, BlockPos block, Direction face, net.minecraft.world.phys.Vec3 hit) {
        ScreenClusterInfo info = screenClusterInfo;
        ComputerInstance c = computer;
        if (info == null || c == null || face != info.facing() || !info.tier().modes().touch()) return null;
        if (!info.members().contains(block)) return null;
        int[] size = c.touchResolution();
        if (size == null) return null;
        int[] px = wallPixel(info, block, hit, size[0], size[1]);
        for (byte kind : new byte[]{com.example.evanscomputermod.network.MouseInputPacket.KIND_DOWN,
                com.example.evanscomputermod.network.MouseInputPacket.KIND_UP}) {
            boolean down = kind == com.example.evanscomputermod.network.MouseInputPacket.KIND_DOWN;
            var packet = new com.example.evanscomputermod.network.MouseInputPacket(computerId, (byte) DISPLAY_ATTACHED,
                    kind, (short) px[0], (short) px[1], (byte) (down ? 1 : 0), (byte) 0, (byte) 0);
            if (!com.example.evanscomputermod.network.MouseInputPacket.deliver(player, packet)) return null;
        }
        return px;
    }

    /**
     * The pixel under {@code hit} on a {@code w x h} picture stretched over the
     * wall (the renderer draws it over the whole rectangle, top-left at the
     * anchor, u to the viewer's right). See ScreenClusterDiscovery for the axes.
     */
    static int[] wallPixel(ScreenClusterInfo info, BlockPos block, net.minecraft.world.phys.Vec3 hit, int w, int h) {
        Direction u = switch (info.facing()) {
            case NORTH -> Direction.WEST;
            case SOUTH -> Direction.EAST;
            case WEST -> Direction.SOUTH;
            default -> Direction.NORTH;
        };
        BlockPos a = info.anchor();
        int col = dot(block, u) - dot(a, u);
        int row = a.getY() - block.getY();
        double along = u.getAxis().choose(hit.x - block.getX(), hit.y - block.getY(), hit.z - block.getZ());
        double fu = u.getAxisDirection() == Direction.AxisDirection.POSITIVE ? along : 1 - along;
        double fv = 1 - (hit.y - block.getY());
        fu = Math.max(0, Math.min(0.999999, fu));
        fv = Math.max(0, Math.min(0.999999, fv));
        int x = (int) ((col + fu) / info.cols() * w);
        int y = (int) ((row + fv) / info.rows() * h);
        return new int[]{Math.max(0, Math.min(w - 1, x)), Math.max(0, Math.min(h - 1, y))};
    }

    private static int dot(BlockPos p, Direction d) {
        return p.getX() * d.getStepX() + p.getY() * d.getStepY() + p.getZ() * d.getStepZ();
    }

    @Override
    @Nullable
    public IAttachedDisplay getAttachedDisplay() {
        return hasScreenCluster() ? attachedScreen : null;
    }

    @Override
    public boolean acceptsPointerInput() {
        return true;
    }

    /**
     * Where this terminal is in world space: its block centre, projected out
     * of a Sable sub-level's plot when it is on a structure.
     */
    private net.minecraft.world.phys.Vec3 worldCenter() {
        net.minecraft.world.phys.Vec3 p = net.minecraft.world.phys.Vec3.atCenterOf(worldPosition);
        //? if <=1.21.1 {
        if (level != null) p = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, p);
        //?}
        return p;
    }

    /**
     * Own display: a player with this terminal's GUI open, or within 8
     * blocks. Screen cluster: within the 64-block sync radius. Distances are
     * in world space, so a terminal on a Sable structure works too.
     */
    @Override
    public boolean canInteract(ServerPlayer player, int displayId) {
        if (level == null || isRemoved() || player.level() != level) return false;
        if (displayId == DISPLAY_OWN) {
            if (player.containerMenu instanceof TerminalMenu m && m.getBlockEntity() == this) return true;
            return player.position().distanceToSqr(worldCenter()) <= 8 * 8;
        }
        if (displayId == DISPLAY_ATTACHED) {
            return player.position().distanceToSqr(worldCenter()) <= 64 * 64;
        }
        return false;
    }

    @Override
    public void onLinkStateChanged(int index, boolean up) {
        if (index >= 6) return;
        setFaceDisabled(index, !up);
        MinecraftServer server = getServer();
        if (server != null) server.execute(() -> updateDisabledFaces(index, up));
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
     * Players who have this terminal's GUI open: the only place the terminal's
     * own display is drawn, so the only players its frames are sent to.
     */
    private List<ServerPlayer> collectTerminalViewers(ServerLevel serverLevel) {
        return serverLevel.getServer().getPlayerList().getPlayers().stream()
                .filter(p -> p.containerMenu instanceof TerminalMenu m && m.getBlockEntity() == this)
                .toList();
    }

    private static List<UUID> ids(List<ServerPlayer> players) {
        List<UUID> ids = new ArrayList<>(players.size());
        for (ServerPlayer p : players) ids.add(p.getUUID());
        return ids;
    }

    /** Send to a player by id, if still online. */
    private static void sendTo(ServerLevel serverLevel, UUID playerId, TerminalDeltaPacket packet) {
        ServerPlayer player = serverLevel.getServer().getPlayerList().getPlayer(playerId);
        if (player != null) PacketDistributor.sendToPlayer(player, packet);
    }

    /**
     * Sync the terminal's own display to the players with its GUI open. One
     * delta per frame is built and shared by all of them (see
     * {@link DisplaySyncHub}).
     */
    private void syncDeltaToClients(ServerLevel serverLevel) {
        List<UUID> viewers = ids(collectTerminalViewers(serverLevel));
        terminalSync.retainViewers(viewers);
        try {
            terminalSync.sync(computerId, display, viewers, (id, packet) -> sendTo(serverLevel, id, packet));
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error syncing terminal display", e);
            terminalSync.forceKeyframe();
        }
    }

    /**
     * Sync the screen cluster's display to nearby players, as display id
     * {@link #DISPLAY_ATTACHED} so the client files it under the cluster.
     */
    private void syncScreenDeltaToClients(ServerLevel serverLevel) {
        TerminalDisplay sd = screenDisplay;
        if (sd == null || screenClusterInfo == null) return;
        List<UUID> viewers = ids(collectSyncRecipients(serverLevel));
        screenSync.retainViewers(viewers);
        try {
            screenSync.sync(computerId, sd, viewers, (id, packet) -> sendTo(serverLevel, id, packet));
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error syncing screen display", e);
            screenSync.forceKeyframe();
        }
    }

    /** The sync hub of one display (tests play clients through it). */
    public DisplaySyncHub getSyncHub(int displayId) {
        return displayId == DISPLAY_ATTACHED ? screenSync : terminalSync;
    }

    /**
     * Called when a client acknowledges a frame or asks for a keyframe
     * ({@code generation == 0}, e.g. on opening the GUI or when this
     * terminal comes into view).
     */
    @Override
    public void onDisplayAck(ServerPlayer player, int displayId, long ackedGeneration) {
        DisplaySyncHub hub = getSyncHub(displayId);
        if (ackedGeneration != 0) {
            // Frames that changed while this client's ack was outstanding
            // were coalesced, not sent: send them now rather than waiting
            // for the next change (or the 10 s keyframe).
            if (hub.onClientReady(player.getUUID(), ackedGeneration)) syncToClients();
            return;
        }
        // Bootstrap: a keyframe straight away. The screen's bootstrap is handled
        // by the normal syncScreenDeltaToClients path on the next dirty tick.
        if (displayId != DISPLAY_OWN || !(level instanceof ServerLevel serverLevel)) return;
        hub.requestKeyframe(player.getUUID());
        try {
            hub.sync(computerId, display, List.of(player.getUUID()), (id, packet) -> sendTo(serverLevel, id, packet));
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error sending keyframe to {}", player.getName().getString(), e);
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


    // ==================== Display Delegation ====================

    /**
     * Sets the computer ID (used when restoring from NBT).
     */
    public void setComputerId(UUID computerId) {
        this.computerId = computerId;
    }

    /**
     * The terminal's own display. Server: the live display the computer
     * writes. Client: what this client was sent for the computer, from the
     * display cache (keyed by computer id, so it survives this block entity
     * being replaced).
     */
    public TerminalDisplay getDisplay() {
        if (level != null && level.isClientSide()) {
            return ClientDisplayCache.entry(computerId, DISPLAY_OWN).display;
        }
        return display;
    }

    public int getCursorX() { return getDisplay().getCursorX(); }
    public int getCursorY() { return getDisplay().getCursorY(); }
    public boolean isCursorVisible() { return getDisplay().isCursorVisible(); }

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

        // Never two instances of one computer: adopt a live (e.g. headless infrastructure)
        // instance, or wait for one that is booting, instead of starting a second.
        var bootHost = com.example.evanscomputermod.computer.ComputerHost.get(getServer(), computerId);
        if (bootHost.instance() != null) {
            attachLiveComputer(bootHost.instance());
            return;
        }
        if (!bootHost.beginBoot()) {
            bootHost.attach(this);
            return;
        }

        wasmLoading = true;
        EvansComputerMod.LOGGER.info("Loading terminal...");

        final String moduleToLoad = wasmModule;
        byte[][] discoveredMacs = discoverInterfaces();

        loadingFuture = CompletableFuture.supplyAsync(() -> {
            try {
                EvansComputerMod.LOGGER.info("Starting async WASM loading for module: {}", moduleToLoad);
                var ownedHost=com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId);
                ownedHost.attach(this);
                ComputerInstance instance = new ComputerInstance(ownedHost, discoveredMacs);
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
        if (getServer() != null) com.example.evanscomputermod.computer.ComputerHost.get(getServer(), computerId).endBoot();
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
        com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId).track(instance);
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
            applyPanelToComputer();
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
        public final DisplaySyncHub terminalSync;
        public final DisplaySyncHub screenSync;
        public final int[] redstoneOutput;
        public final int[] redstoneInput;
        public final long disabledFacesMask;
        @Nullable public final BlockPos[] interfaceExitPositions;

        public TransferBundle(ComputerInstance computer,
                              boolean wasmInitialized,
                              @Nullable TerminalDisplay screenDisplay,
                              @Nullable ScreenClusterInfo screenClusterInfo,
                              boolean screenPowered,
                              DisplaySyncHub terminalSync,
                              DisplaySyncHub screenSync,
                              int[] redstoneOutput,
                              int[] redstoneInput,
                              long disabledFacesMask,
                              @Nullable BlockPos[] interfaceExitPositions) {
            this.computer = computer;
            this.wasmInitialized = wasmInitialized;
            this.screenDisplay = screenDisplay;
            this.screenClusterInfo = screenClusterInfo;
            this.screenPowered = screenPowered;
            this.terminalSync = terminalSync;
            this.screenSync = screenSync;
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
                terminalSync,
                screenSync,
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
        var ownedHost=com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId);
        ownedHost.attach(this);ownedHost.track(this.computer);
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
        this.terminalSync = bundle.terminalSync;
        this.screenSync = bundle.screenSync;
        terminalSync.forceKeyframe();
        screenSync.forceKeyframe();

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
        applyPanelToComputer();

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
                info.gfxWidth(), info.gfxHeight(), newMembers, info.tier(),
                info.limitedBy() != null ? info.limitedBy().offset(delta) : null);
        screenClusterInfo = translated;

        for (BlockPos member : newMembers) {
            if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                sbe.setClusterMembership(
                        worldPosition, computerId, newAnchor, info.cols(), info.rows(),
                        member.equals(newAnchor), info.tier().tier());
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
        if(getServer()!=null) com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId).forget();
        ComputerRegistry.unregister(computerId, this);

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
        if(level!=null && !level.isClientSide() && !transferring
                && (!level.hasChunkAt(worldPosition) || level.getBlockState(worldPosition).getBlock() instanceof TerminalBlock)
                && detachAlwaysOn()) return;
        // Always release the modules' world registrations (e.g. Create link
        // networks) at this position, including on a Sable move: the
        // destination block entity starts its own from the saved stacks.
        stopPeripherals();
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
        if(level!=null && !level.isClientSide()) {
            // Findable by id from the first tick, booted or not: display and
            // input packets name computers by UUID.
            ComputerRegistry.register(this);
            var host=com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId);
            var saved=host.takeBundle(this);
            if(saved!=null) {transferring=false;adoptComputer(saved);return;}
            if(host.instance()!=null) {attachLiveComputer(host.instance());return;}
            if(host.isBooting()) {host.attach(this);return;}
        }
        if (level != null && level.isClientSide()) {
            // The client update tag carries no framebuffer (see getUpdateTag):
            // ask for a keyframe so a terminal that just came into view shows
            // its screen, not a blank one.
            ClientDisplayCache.invalidate(computerId, DISPLAY_OWN);
            com.example.evanscomputermod.client.ClientPacketHandler.requestKeyframe(computerId);
        }
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
        if(detachAlwaysOn()) return;
        stopPeripherals();
        // Gracefully stop the computer but keep wasRunning true (already saved to NBT)
        // so it auto-starts when the chunk reloads.
        shutdownComputer();
    }

    private boolean hasAlwaysOnModule() {
        for(int slot=0;slot<ModuleBays.SLOTS;slot++) if(moduleBays.getStack(slot).getItem() instanceof com.example.evanscomputermod.module.AlwaysOnModuleItem) return true;
        return false;
    }
    private boolean detachAlwaysOn() {
        if(computer==null || getServer()==null) return false;
        var host=com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId);
        if(!hasAlwaysOnModule() && !host.isInfrastructure()) return false;
        if(!host.canDetach()) return false;
        host.detach(captureForTransfer());stopPeripherals();ComputerRegistry.unregister(computerId, this);
        computer=null;wasmInitialized=false;wasRunning=true;return true;
    }
    public void attachLiveComputer(ComputerInstance live) {
        computer=live;wasmInitialized=true;wasRunning=true;transferring=false;
        var host=com.example.evanscomputermod.computer.ComputerHost.get(getServer(),computerId);host.attach(this);host.track(live);
        discoverInterfaces();ComputerRegistry.register(this);
        var mgr=CableNetworkManager.getInstance();if(mgr!=null) mgr.registerTerminal(worldPosition,level.dimension(),live.getNetworkMacs(),interfaceExitPositions);
        applyPanelToComputer();rescanScreenCluster();forceNextKeyframe();setChanged();
    }

    // ==================== Input Handling ====================

    /** Keys typed into this terminal (what {@code TerminalInputPacket} delivers). */
    public void onStringInput(String input) {
        if (input == null || input.isEmpty() || wasmLoading) {
            return;
        }
        if (computer != null) {
            computer.handleKeyInput(input);
        }
        // No sendBlockUpdated here: input changes nothing in the client update
        // tag, and the screen reaches clients only through the delta protocol
        // (see getUpdateTag).
        setChanged();
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
        updateRedstoneInput();
        rescanScreenCluster();
        if (peripheralsStarted) {
            peripheralHub.rescanSides();
        }
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
            screenSync.reset();
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
        // The wall runs at its lowest block's tier: tile size and cap from it.
        var modes = result.tier().modes();
        int idealW = result.cols() * modes.screenTileW();
        int idealH = result.rows() * modes.screenTileH();
        double scale = Math.min(1.0, Math.min(
                (double) modes.screenMaxW() / idealW,
                (double) modes.screenMaxH() / idealH));
        int gfxW = Math.max(1, (int) Math.round(idealW * scale));
        int gfxH = Math.max(1, (int) Math.round(idealH * scale));
        ScreenClusterInfo info = new ScreenClusterInfo(
                result.anchor(), result.facing(), result.cols(), result.rows(),
                gfxW, gfxH, result.members(), result.tier(), result.limitedBy());
        screenClusterInfo = info;

        // (Re)allocate the display if the cluster's size changed. Reset the
        // power state on any reform so the Rust OS must explicitly turn the
        // monitor back on before content reappears. Compare against the old
        // cluster, not the display: a program that owns the screen sets its
        // own resolution (gba: 240x160), and any neighbour update (a speaker's
        // cone moving) would otherwise blank and power off the screen mid-game.
        TerminalDisplay sd = screenDisplay;
        boolean sameCluster = oldInfo != null && sd != null
                && oldInfo.gfxWidth() == gfxW && oldInfo.gfxHeight() == gfxH
                && oldInfo.tier() == info.tier();
        if (!sameCluster) {
            sd = new TerminalDisplay(1, 1);
            screenDisplay = sd;
            screenSync.reset();
            screenPowered = false;
        }

        // Tell every member its cluster role and its active flag.
        // active = clusterValid && screenPowered. clusterValid is true
        // here because we're in the success branch; screenPowered drives
        // whether the content quad is rendered.
        for (BlockPos member : result.members()) {
            if (level.getBlockEntity(member) instanceof ScreenBlockEntity sbe) {
                sbe.setClusterMembership(
                        worldPosition,
                        computerId,
                        result.anchor(),
                        result.cols(),
                        result.rows(),
                        member.equals(result.anchor()),
                        result.tier().tier());
                sbe.setActive(screenPowered);
            }
        }

        // Write dimensions into the WASM screen header so the Rust OS sees them.
        if (computer != null) {
            computer.writeScreenHeader(gfxW, gfxH);
            // A lower tier may no longer fit a program's framebuffer.
            if (oldInfo != null && oldInfo.tier() != info.tier()) {
                computer.displayDevice(ComputerInstance.GFX_TARGET_SCREEN).enforceLimits();
            }
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
            screenSync.forceKeyframe();
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
        InstalledModules modules = moduleBays.snapshot();
        if (!modules.isEmpty()) output.store("modules", InstalledModules.CODEC, modules);
        output.store("panel", com.example.evanscomputermod.module.InstalledPanel.CODEC,
                new com.example.evanscomputermod.module.InstalledPanel(panel));

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
        moduleBays.restore(input.read("modules", InstalledModules.CODEC).orElse(InstalledModules.EMPTY));
        // No tag: saved before panels existed -> keep the Tier 2 default.
        panel = input.read("panel", com.example.evanscomputermod.module.InstalledPanel.CODEC)
                .map(com.example.evanscomputermod.module.InstalledPanel::panel).orElseGet(TerminalBlockEntity::defaultPanel);
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
        InstalledModules modules = moduleBays.snapshot();
        if (!modules.isEmpty()) {
            InstalledModules.CODEC.encodeStart(registries.createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), modules)
                    .result().ifPresent(t -> tag.put("modules", t));
        }
        com.example.evanscomputermod.module.InstalledPanel.CODEC.encodeStart(
                        registries.createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE),
                        new com.example.evanscomputermod.module.InstalledPanel(panel))
                .result().ifPresent(t -> tag.put("panel", t));

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
        InstalledModules modules = InstalledModules.EMPTY;
        if (tag.contains("modules")) {
            modules = InstalledModules.CODEC.parse(registries.createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), tag.get("modules"))
                    .result().orElse(InstalledModules.EMPTY);
        }
        moduleBays.restore(modules);
        // No tag: saved before panels existed -> keep the Tier 2 default.
        panel = defaultPanel();
        if (tag.contains("panel")) {
            com.example.evanscomputermod.module.InstalledPanel.CODEC.parse(
                            registries.createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), tag.get("panel"))
                    .result().ifPresent(p -> panel = p.panel());
        }
    }*/
    //?}

    // ==================== Network Sync ====================

    @Nullable
    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }

    /**
     * What clients get in block-entity data packets and chunk data. The
     * framebuffer is left out on purpose: a client's screen is written only
     * by the delta protocol (keyframes + deltas against a per-player shadow,
     * see syncDeltaToClients). A full-screen copy arriving here as well
     * replaced the client's screen behind the shadow's back, and the next
     * delta ("scroll up k, rewrite these rows") was then applied to a screen
     * that had already scrolled: output appeared twice until the GUI was
     * reopened (ecm_sync GameTests reproduce it). The framebuffer is still
     * saved to disk (saveAdditional).
     */
    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        CompoundTag tag = saveWithoutMetadata(registries);
        tag.remove("framebuffer");
        return tag;
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

        while (!queue.isEmpty() && macs.size()<32) {
            net.minecraft.core.BlockPos pos = queue.poll();
            for (net.minecraft.core.Direction dir : net.minecraft.core.Direction.values()) {
                net.minecraft.core.BlockPos facePos = pos.relative(dir);
                if (visited.contains(facePos)) continue;
                net.minecraft.world.level.block.Block block = level.getBlockState(facePos).getBlock();
                if (block instanceof com.example.evanscomputermod.block.InterfaceBlock) {
                    visited.add(facePos);
                    queue.add(facePos);
                } else if(macs.size()<32) {
                    // Free face = new interface. Exit position is facePos (the block adjacent to the free face)
                    macs.add(com.example.evanscomputermod.computer.NetworkHub.deriveMac(computerId, macs.size()));
                    exitPosns.add(facePos);
                }
            }
        }

        this.interfaceExitPositions = exitPosns.toArray(new BlockPos[0]);
        return macs.toArray(new byte[0][]);
    }

    /**
     * Look up the interface index (eth0, eth1, ...) whose exit position matches
     * {@code exitPos}, or -1 if no interface exposes that face. Used by the
     * Interface Probe item to map a right-clicked face to an interface name.
     */
    public int findInterfaceIndexByExitPos(BlockPos exitPos) {
        if (interfaceExitPositions == null) return -1;
        for (int i = 0; i < interfaceExitPositions.length; i++) {
            if (interfaceExitPositions[i] != null
                    && interfaceExitPositions[i].equals(exitPos)) return i;
        }
        return -1;
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
}
