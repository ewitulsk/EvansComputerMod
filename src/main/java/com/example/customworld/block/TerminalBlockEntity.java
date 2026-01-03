package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.wasm.TerminalWasmHost;
import com.example.customworld.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

import javax.annotation.Nullable;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Block Entity for the Terminal block.
 * Stores the terminal state including the 80x24 character buffer,
 * cursor position, and manages WASM execution context.
 */
public class TerminalBlockEntity extends BlockEntity implements MenuProvider {
    
    // Terminal dimensions (standard terminal size)
    public static final int TERMINAL_WIDTH = 80;
    public static final int TERMINAL_HEIGHT = 24;
    
    // Terminal buffer - stores all characters displayed
    private final char[][] buffer = new char[TERMINAL_HEIGHT][TERMINAL_WIDTH];
    
    // Cursor position
    private int cursorX = 0;
    private int cursorY = 0;
    
    // Current input line (for line-by-line input mode)
    private StringBuilder inputLine = new StringBuilder();
    
    // Input mode: true = character mode, false = line mode
    private boolean characterMode = false;
    
    // Which WASM module to execute when terminal opens
    private String wasmModule = "terminal_os";
    private String wasmFunction = "main";
    
    // WASM host for executing terminal programs (server-side only)
    @Nullable
    private volatile TerminalWasmHost wasmHost;
    private volatile boolean wasmInitialized = false;
    
    // Async loading state
    private volatile boolean wasmLoading = false;
    @Nullable
    private CompletableFuture<TerminalWasmHost> loadingFuture;
    
    // Shared executor for background WASM loading (single thread to avoid overload)
    private static final ExecutorService WASM_EXECUTOR = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "WASM-Loader");
        t.setDaemon(true);  // Don't prevent JVM shutdown
        return t;
    });
    
    // Unique computer ID - persists when block is picked up and moved
    private UUID computerId;
    
    // Redstone output power for each of the 6 sides (DOWN, UP, NORTH, SOUTH, WEST, EAST)
    private final int[] redstoneOutput = new int[6];
    
    public TerminalBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.TERMINAL_BLOCK_ENTITY.get(), pos, state);
        this.computerId = UUID.randomUUID();
        clearBuffer();
    }
    
    /**
     * Gets the unique computer ID for this terminal.
     * This ID is used to associate files with this specific computer.
     */
    public UUID getComputerId() {
        return computerId;
    }
    
    /**
     * Sets the computer ID (used when restoring from NBT).
     */
    public void setComputerId(UUID computerId) {
        this.computerId = computerId;
    }
    
    /**
     * Initializes the WASM host and loads the configured module.
     * Called when the terminal is first opened.
     * This method returns quickly - heavy WASM loading happens on a background thread.
     */
    public void initializeWasm() {
        if (level == null || level.isClientSide) {
            return;
        }
        
        // Check if WASM host is faulted and needs reset
        if (wasmInitialized && wasmHost != null && wasmHost.isFaulted()) {
            CustomWorldMod.LOGGER.info("Resetting faulted WASM terminal");
            wasmHost.close();
            wasmHost = null;
            wasmInitialized = false;
            wasmLoading = false;
            clearBuffer();  // Clear the error messages from screen
        }
        
        // Already initialized or currently loading
        if (wasmInitialized || wasmLoading) {
            return;
        }
        
        // Mark as loading and show loading message
        wasmLoading = true;
        clearBuffer();
        write("Loading terminal...\n");
        
        // Capture values needed for the background task
        final String moduleToLoad = wasmModule;
        
        // Submit heavy work to background thread
        loadingFuture = CompletableFuture.supplyAsync(() -> {
            try {
                CustomWorldMod.LOGGER.info("Starting async WASM loading for module: {}", moduleToLoad);
                TerminalWasmHost host = new TerminalWasmHost(this);
                host.loadModule(moduleToLoad);
                CustomWorldMod.LOGGER.info("Async WASM loading complete for module: {}", moduleToLoad);
                return host;
            } catch (WasmManager.WasmExecutionException e) {
                CustomWorldMod.LOGGER.error("Failed to load WASM module in background", e);
                throw new RuntimeException(e);
            }
        }, WASM_EXECUTOR);
        
        // Handle completion on the main server thread
        loadingFuture.whenComplete((host, error) -> {
            // Schedule the completion callback on the main server thread
            if (level != null && level.getServer() != null) {
                level.getServer().execute(() -> onWasmLoadComplete(host, error));
            }
        });
    }
    
    /**
     * Called on the main server thread when WASM loading completes.
     */
    private void onWasmLoadComplete(@Nullable TerminalWasmHost host, @Nullable Throwable error) {
        // Check if the block entity was removed while loading
        if (isRemoved()) {
            if (host != null) {
                host.close();
            }
            return;
        }
        
        wasmLoading = false;
        loadingFuture = null;
        
        if (error != null) {
            // Loading failed
            wasmInitialized = true;  // Mark as initialized to prevent retry loops
            clearBuffer();
            write("Error loading WASM module: " + wasmModule + "\n");
            
            // Unwrap the exception to get the real message
            Throwable cause = error;
            while (cause.getCause() != null) {
                cause = cause.getCause();
            }
            write(cause.getMessage() + "\n");
            write("\nPlace a .wasm file in wasm-bin/ directory.\n");
            
            // Sync error state to client
            setChanged();
            if (level != null && !level.isClientSide) {
                level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
            }
            return;
        }
        
        // Loading succeeded
        wasmHost = host;
        wasmInitialized = true;
        
        try {
            // Clear the loading message and execute main
            clearBuffer();
            wasmHost.executeMain();
            CustomWorldMod.LOGGER.info("Initialized WASM terminal with module: {}", wasmModule);
            
            // Start the worker thread for async input processing
            wasmHost.startWorkerThread();
        } catch (WasmManager.WasmExecutionException e) {
            write("Error executing WASM main: " + e.getMessage() + "\n");
            CustomWorldMod.LOGGER.error("Failed to execute WASM main", e);
        }
        
        // Sync to client
        setChanged();
        if (level != null && !level.isClientSide) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }
    
    /**
     * Returns true if WASM is currently being loaded in the background.
     */
    public boolean isWasmLoading() {
        return wasmLoading;
    }
    
    /**
     * Shuts down the WASM host when the block entity is removed.
     */
    @Override
    public void setRemoved() {
        super.setRemoved();
        
        // Cancel any pending loading
        if (loadingFuture != null) {
            loadingFuture.cancel(true);  // Interrupt if possible
            loadingFuture = null;
        }
        wasmLoading = false;
        
        if (wasmHost != null) {
            // Signal WASM to stop execution before closing
            // This allows any running WASM code to exit gracefully via host function checks
            wasmHost.interrupt();
            wasmHost.close();
            wasmHost = null;
        }
    }
    
    /**
     * Clears the terminal buffer, filling it with spaces.
     */
    public void clearBuffer() {
        for (int y = 0; y < TERMINAL_HEIGHT; y++) {
            for (int x = 0; x < TERMINAL_WIDTH; x++) {
                buffer[y][x] = ' ';
            }
        }
        cursorX = 0;
        cursorY = 0;
    }
    
    /**
     * Writes a string to the terminal at the current cursor position.
     * Handles newlines and wrapping.
     */
    public void write(String text) {
        for (char c : text.toCharArray()) {
            writeChar(c);
        }
        setChanged();
        if (level != null && !level.isClientSide) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }
    
    /**
     * Writes a single character to the terminal.
     */
    private void writeChar(char c) {
        if (c == '\n') {
            newLine();
        } else if (c == '\r') {
            cursorX = 0;
        } else if (c == '\b') {
            // Backspace
            if (cursorX > 0) {
                cursorX--;
                buffer[cursorY][cursorX] = ' ';
            }
        } else {
            if (cursorX >= TERMINAL_WIDTH) {
                newLine();
            }
            buffer[cursorY][cursorX] = c;
            cursorX++;
        }
    }
    
    /**
     * Moves to a new line, scrolling if necessary.
     */
    private void newLine() {
        cursorX = 0;
        cursorY++;
        if (cursorY >= TERMINAL_HEIGHT) {
            scrollUp();
            cursorY = TERMINAL_HEIGHT - 1;
        }
    }
    
    /**
     * Scrolls the terminal buffer up by one line.
     */
    private void scrollUp() {
        // Move all lines up by one
        for (int y = 0; y < TERMINAL_HEIGHT - 1; y++) {
            System.arraycopy(buffer[y + 1], 0, buffer[y], 0, TERMINAL_WIDTH);
        }
        // Clear the bottom line
        for (int x = 0; x < TERMINAL_WIDTH; x++) {
            buffer[TERMINAL_HEIGHT - 1][x] = ' ';
        }
    }
    
    /**
     * Gets the character at a specific position.
     */
    public char getChar(int x, int y) {
        if (x >= 0 && x < TERMINAL_WIDTH && y >= 0 && y < TERMINAL_HEIGHT) {
            return buffer[y][x];
        }
        return ' ';
    }
    
    /**
     * Gets a line of text from the buffer.
     */
    public String getLine(int y) {
        if (y >= 0 && y < TERMINAL_HEIGHT) {
            return new String(buffer[y]);
        }
        return "";
    }
    
    /**
     * Gets the entire buffer as a single string with newlines.
     */
    public String getBufferAsString() {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < TERMINAL_HEIGHT; y++) {
            sb.append(buffer[y]);
            if (y < TERMINAL_HEIGHT - 1) {
                sb.append('\n');
            }
        }
        return sb.toString();
    }
    
    /**
     * Sets the buffer from a string (used for network sync).
     */
    public void setBufferFromString(String content) {
        String[] lines = content.split("\n", -1);
        for (int y = 0; y < TERMINAL_HEIGHT; y++) {
            if (y < lines.length) {
                for (int x = 0; x < TERMINAL_WIDTH; x++) {
                    buffer[y][x] = x < lines[y].length() ? lines[y].charAt(x) : ' ';
                }
            } else {
                for (int x = 0; x < TERMINAL_WIDTH; x++) {
                    buffer[y][x] = ' ';
                }
            }
        }
    }
    
    /**
     * Handles string input from the user (supports escape sequences for special keys).
     * This is the main input handler called by the network packet.
     * Input is processed on a background thread so sleep() doesn't block the game server.
     */
    public void onStringInput(String input) {
        if (input == null || input.isEmpty()) {
            return;
        }
        
        // Ignore input while WASM is still loading
        if (wasmLoading) {
            return;
        }
        
        // Check for Ctrl+T (0x14) - interrupt immediately, don't just queue
        // This allows interrupting infinite loops since the main thread handles this
        if (input.contains("\u0014") && wasmHost != null) {
            CustomWorldMod.LOGGER.info("Ctrl+T detected - interrupting WASM execution");
            wasmHost.interrupt();
            // Still queue the input so the OS can display the interrupt message
        }
        
        // Send to WASM worker thread (queued, non-blocking)
        if (wasmHost != null) {
            try {
                wasmHost.sendInput(input);
            } catch (Throwable e) {
                // Safety net: catch any errors that escape from WASM execution
                // This prevents WASM failures from crashing the game server
                CustomWorldMod.LOGGER.error("WASM execution error in terminal", e);
                write("\nFatal WASM error: " + e.getMessage() + "\n");
                write("[Close and reopen terminal to reset]\n");
            }
        }
        
        setChanged();
        if (level != null && !level.isClientSide) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }
    
    /**
     * Handles single character input from the user.
     * Kept for backwards compatibility and client-side immediate feedback.
     */
    public void onCharInput(char c) {
        onStringInput(String.valueOf(c));
    }
    
    /**
     * Called when a complete line is entered (in line mode).
     * @deprecated Use onStringInput instead - the WASM OS handles input processing.
     */
    @Deprecated
    protected void onLineInput(String line) {
        CustomWorldMod.LOGGER.info("Terminal input: {}", line);
        
        // Send input to WASM if available
        if (wasmHost != null) {
            wasmHost.sendInput(line);
        }
    }
    
    /**
     * Gets the WASM host for this terminal.
     */
    @Nullable
    public TerminalWasmHost getWasmHost() {
        return wasmHost;
    }
    
    // Getters and setters
    public int getCursorX() { return cursorX; }
    public int getCursorY() { return cursorY; }
    public void setCursor(int x, int y) {
        this.cursorX = Math.max(0, Math.min(x, TERMINAL_WIDTH - 1));
        this.cursorY = Math.max(0, Math.min(y, TERMINAL_HEIGHT - 1));
    }
    
    public boolean isCharacterMode() { return characterMode; }
    public void setCharacterMode(boolean characterMode) { this.characterMode = characterMode; }
    
    public String getWasmModule() { return wasmModule; }
    public void setWasmModule(String wasmModule) { this.wasmModule = wasmModule; }
    
    public String getWasmFunction() { return wasmFunction; }
    public void setWasmFunction(String wasmFunction) { this.wasmFunction = wasmFunction; }
    
    /**
     * Sets the redstone output power for a specific side.
     * @param side The side index (0=DOWN, 1=UP, 2=NORTH, 3=SOUTH, 4=WEST, 5=EAST)
     * @param power The power level (0-15)
     */
    public void setRedstoneOutput(int side, int power) {
        if (side >= 0 && side < 6) {
            int oldPower = redstoneOutput[side];
            redstoneOutput[side] = Math.max(0, Math.min(15, power));
            
            // Only update if power actually changed
            if (oldPower != redstoneOutput[side] && level != null && !level.isClientSide) {
                setChanged();
                // Notify neighbors of redstone change
                level.updateNeighborsAt(worldPosition, getBlockState().getBlock());
            }
        }
    }
    
    /**
     * Gets the redstone output power for a specific side.
     * @param side The side index (0=DOWN, 1=UP, 2=NORTH, 3=SOUTH, 4=WEST, 5=EAST)
     * @return The power level (0-15)
     */
    public int getRedstoneOutput(int side) {
        return (side >= 0 && side < 6) ? redstoneOutput[side] : 0;
    }
    
    // NBT serialization
    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putUUID("computerId", computerId);
        tag.putString("buffer", getBufferAsString());
        tag.putInt("cursorX", cursorX);
        tag.putInt("cursorY", cursorY);
        tag.putBoolean("characterMode", characterMode);
        tag.putString("wasmModule", wasmModule);
        tag.putString("wasmFunction", wasmFunction);
        tag.putIntArray("redstoneOutput", redstoneOutput);
    }
    
    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("computerId")) {
            computerId = tag.getUUID("computerId");
        }
        if (tag.contains("buffer")) {
            setBufferFromString(tag.getString("buffer"));
        }
        cursorX = tag.getInt("cursorX");
        cursorY = tag.getInt("cursorY");
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
    }
    
    // Network sync
    @Nullable
    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }
    
    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        return saveWithoutMetadata(registries);
    }
    
    // MenuProvider implementation
    @Override
    public Component getDisplayName() {
        return Component.translatable("container.customworld.terminal");
    }
    
    @Nullable
    @Override
    public AbstractContainerMenu createMenu(int containerId, Inventory playerInventory, Player player) {
        return new TerminalMenu(containerId, playerInventory, this);
    }
}
