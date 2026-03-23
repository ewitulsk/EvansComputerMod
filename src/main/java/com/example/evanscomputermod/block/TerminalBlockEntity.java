package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.*;
import com.example.evanscomputermod.config.DisplayConfig;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
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
import net.minecraft.world.level.block.state.BlockState;

import javax.annotation.Nullable;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Block Entity for the Terminal block.
 * Implements {@link IComputerHost} to provide a computer embedding context.
 * Delegates display to {@link TerminalDisplay} and runtime to {@link ComputerInstance}.
 */
public class TerminalBlockEntity extends BlockEntity implements MenuProvider, IComputerHost {

    // Terminal dimensions (standard terminal size)
    public static final int TERMINAL_WIDTH = 80;
    public static final int TERMINAL_HEIGHT = 24;

    // Display state (extracted into reusable component)
    private final TerminalDisplay display = new TerminalDisplay(TERMINAL_WIDTH, TERMINAL_HEIGHT);

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

    // Attached display (discovered from adjacent blocks)
    @Nullable
    private BlockPos attachedDisplayPos;
    @Nullable
    private transient DisplayBlockEntity cachedDisplay;

    public TerminalBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.TERMINAL_BLOCK_ENTITY.get(), pos, state);
        this.computerId = UUID.randomUUID();
        display.clearBuffer();
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

    @Override
    public void syncToClients() {
        if (level != null && level.getServer() != null) {
            level.getServer().execute(() -> {
                setChanged();
                if (level != null && !level.isClientSide) {
                    level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
                }
            });
        }
    }

    @Override
    @Nullable
    public ITerminalOutput getTerminalOutput() {
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

    @Override
    @Nullable
    public IFramebufferHost getAttachedDisplay() {
        if (attachedDisplayPos == null) return null;
        // Use cached reference if available and still valid
        if (cachedDisplay != null && !cachedDisplay.isRemoved()
                && cachedDisplay.getBlockPos().equals(attachedDisplayPos)) {
            return cachedDisplay;
        }
        // Re-resolve from world
        if (level != null && level.getBlockEntity(attachedDisplayPos) instanceof DisplayBlockEntity display) {
            cachedDisplay = display;
            return display;
        }
        // Display is gone
        attachedDisplayPos = null;
        cachedDisplay = null;
        return null;
    }

    // ==================== Display Delegation ====================

    /**
     * Sets the computer ID (used when restoring from NBT).
     */
    public void setComputerId(UUID computerId) {
        this.computerId = computerId;
    }

    public void clearBuffer() {
        display.clearBuffer();
    }

    public void write(String text) {
        display.write(text);
        setChanged();
        if (level != null && !level.isClientSide) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    public char getChar(int x, int y) {
        return display.getChar(x, y);
    }

    public String getLine(int y) {
        return display.getLine(y);
    }

    public int getScrollbackSize() {
        return display.getScrollbackSize();
    }

    public String getScrollbackLine(int index) {
        return display.getScrollbackLine(index);
    }

    public void clearScrollback() {
        display.clearScrollback();
    }

    public String getBufferAsString() {
        return display.getBufferAsString();
    }

    public void setBufferFromString(String content) {
        display.setBufferFromString(content);
    }

    public int getCursorX() { return display.getCursorX(); }
    public int getCursorY() { return display.getCursorY(); }
    public void setCursor(int x, int y) {
        display.setCursor(x, y);
    }

    // ==================== WASM Lifecycle ====================

    /**
     * Initializes the computer and loads the configured WASM module.
     * Called when the terminal is first opened.
     */
    public void initializeWasm() {
        if (level == null || level.isClientSide) {
            return;
        }

        // Check if computer is faulted and needs reset
        if (wasmInitialized && computer != null && computer.isFaulted()) {
            EvansComputerMod.LOGGER.info("Resetting faulted WASM terminal");
            computer.close();
            computer = null;
            wasmInitialized = false;
            wasmLoading = false;
            clearBuffer();
        }

        if (wasmInitialized || wasmLoading) {
            return;
        }

        // Scan for adjacent display before starting WASM
        scanForDisplay();

        wasmLoading = true;
        clearBuffer();
        write("Loading terminal...\n");

        final String moduleToLoad = wasmModule;

        loadingFuture = CompletableFuture.supplyAsync(() -> {
            try {
                EvansComputerMod.LOGGER.info("Starting async WASM loading for module: {}", moduleToLoad);
                ComputerInstance instance = new ComputerInstance(this);
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
            clearBuffer();
            write("Error loading WASM module: " + wasmModule + "\n");

            Throwable cause = error;
            while (cause.getCause() != null) {
                cause = cause.getCause();
            }
            write(cause.getMessage() + "\n");
            write("\nPlace a .wasm file in wasm-bin/ directory.\n");

            setChanged();
            if (level != null && !level.isClientSide) {
                level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
            }
            return;
        }

        computer = instance;
        wasmInitialized = true;
        ComputerRegistry.register(this);

        try {
            clearBuffer();
            computer.executeMain();
            EvansComputerMod.LOGGER.info("Initialized WASM terminal with module: {}", wasmModule);

            computer.startWorkerThread();
            computer.rescanPeripherals();
        } catch (WasmManager.WasmExecutionException e) {
            write("Error executing WASM main: " + e.getMessage() + "\n");
            EvansComputerMod.LOGGER.error("Failed to execute WASM main", e);
        }

        setChanged();
        if (level != null && !level.isClientSide) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    public boolean isWasmLoading() {
        return wasmLoading;
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        ComputerRegistry.unregister(computerId);

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
                EvansComputerMod.LOGGER.error("WASM execution error in terminal", e);
                write("\nFatal WASM error: " + e.getMessage() + "\n");
                write("[Close and reopen terminal to reset]\n");
            }
        }

        setChanged();
        if (level != null && !level.isClientSide) {
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
        scanForDisplay();
    }

    /**
     * Scans adjacent blocks for a DisplayBlockEntity and attaches to it.
     */
    private void scanForDisplay() {
        if (level == null || level.isClientSide) return;

        // If we already have a valid display, keep it
        if (attachedDisplayPos != null) {
            if (level.getBlockEntity(attachedDisplayPos) instanceof DisplayBlockEntity display
                    && display.getControllerPos() != null
                    && display.getControllerPos().equals(worldPosition)) {
                return; // Still connected
            }
            // Lost our display
            attachedDisplayPos = null;
            cachedDisplay = null;
        }

        // Scan adjacent blocks for an uncontrolled display
        for (Direction dir : Direction.values()) {
            BlockPos neighborPos = worldPosition.relative(dir);
            if (level.getBlockEntity(neighborPos) instanceof DisplayBlockEntity display) {
                if (!display.hasController()) {
                    display.setController(worldPosition);
                    attachedDisplayPos = neighborPos;
                    cachedDisplay = display;
                    EvansComputerMod.LOGGER.info("Terminal at {} attached to display at {}", worldPosition, neighborPos);

                    // Queue a display-connect interrupt if computer is running
                    if (computer != null) {
                        computer.queueInterrupt(3, "{\"event\":\"connect\",\"width\":"
                                + display.getDisplayWidth() + ",\"height\":" + display.getDisplayHeight() + "}");
                    }
                    return;
                }
            }
        }
    }

    /**
     * Called by DisplayBlockEntity when it is removed.
     */
    public void onDisplayDetached(BlockPos displayPos) {
        if (displayPos.equals(attachedDisplayPos)) {
            attachedDisplayPos = null;
            cachedDisplay = null;
            if (computer != null) {
                computer.queueInterrupt(3, "{\"event\":\"disconnect\"}");
            }
        }
    }

    private void updateRedstoneInput() {
        if (level == null || level.isClientSide) return;

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

            if (oldPower != redstoneOutput[side] && level != null && !level.isClientSide) {
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
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putUUID("computerId", computerId);
        tag.putString("buffer", display.getBufferAsString());
        tag.putInt("cursorX", display.getCursorX());
        tag.putInt("cursorY", display.getCursorY());
        tag.putBoolean("characterMode", characterMode);
        tag.putString("wasmModule", wasmModule);
        tag.putString("wasmFunction", wasmFunction);
        tag.putIntArray("redstoneOutput", redstoneOutput);

        tag.putInt("scrollbackSize", display.getScrollbackSize());
        String scrollbackData = display.getScrollbackAsString();
        if (!scrollbackData.isEmpty()) {
            tag.putString("scrollback", scrollbackData);
        }
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("computerId")) {
            computerId = tag.getUUID("computerId");
        }
        if (tag.contains("buffer")) {
            display.setBufferFromString(tag.getString("buffer"));
        }
        display.setCursor(tag.getInt("cursorX"), tag.getInt("cursorY"));
        characterMode = tag.getBoolean("characterMode");
        if (tag.contains("wasmModule")) {
            wasmModule = tag.getString("wasmModule");
        }
        if (tag.contains("wasmFunction")) {
            wasmFunction = tag.getString("wasmFunction");
        }
        if (tag.contains("redstoneOutput")) {
            int[] saved = tag.getIntArray("redstoneOutput");
            System.arraycopy(saved, 0, redstoneOutput, 0, Math.min(saved.length, 6));
        }

        if (tag.contains("scrollback")) {
            display.setScrollbackFromString(tag.getString("scrollback"));
        }
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
        if (level == null || level.isClientSide) return;
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
        if (level == null || level.isClientSide) return;
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

    public void openVisualEditor() {
        if (level != null && !level.isClientSide) {
            var packet = new com.example.evanscomputermod.network.OpenVisualEditorPacket(getBlockPos());
            var chunkPos = new net.minecraft.world.level.ChunkPos(getBlockPos());
            net.neoforged.neoforge.network.PacketDistributor.sendToPlayersTrackingChunk(
                    (net.minecraft.server.level.ServerLevel) level, chunkPos, packet);
        }
    }
}
