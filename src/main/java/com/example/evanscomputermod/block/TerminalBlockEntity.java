package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.wasm.TerminalWasmHost;
import com.example.evanscomputermod.wasm.WasmManager;
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
import java.util.ArrayList;
import java.util.List;
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
    
    // Scrollback buffer size (number of lines to keep in history)
    public static final int SCROLLBACK_SIZE = 1000;
    
    // Terminal buffer - stores all characters displayed
    private final char[][] buffer = new char[TERMINAL_HEIGHT][TERMINAL_WIDTH];
    
    // Scrollback buffer - stores lines that scrolled off the top
    private final List<char[]> scrollbackBuffer = new ArrayList<>();
    
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
            EvansComputerMod.LOGGER.info("Resetting faulted WASM terminal");
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
                EvansComputerMod.LOGGER.info("Starting async WASM loading for module: {}", moduleToLoad);
                TerminalWasmHost host = new TerminalWasmHost(this);
                host.loadModule(moduleToLoad);
                EvansComputerMod.LOGGER.info("Async WASM loading complete for module: {}", moduleToLoad);
                return host;
            } catch (WasmManager.WasmExecutionException e) {
                EvansComputerMod.LOGGER.error("Failed to load WASM module in background", e);
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
            EvansComputerMod.LOGGER.info("Initialized WASM terminal with module: {}", wasmModule);
            
            // Start the worker thread for async input processing
            wasmHost.startWorkerThread();
            
            // Scan for peripherals now that the world is fully loaded
            // This ensures peripherals are detected after rejoining the game
            wasmHost.rescanPeripherals();
        } catch (WasmManager.WasmExecutionException e) {
            write("Error executing WASM main: " + e.getMessage() + "\n");
            EvansComputerMod.LOGGER.error("Failed to execute WASM main", e);
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
     * Note: This does NOT clear the scrollback buffer - use clearScrollback() for that.
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
     * The top line is pushed to the scrollback buffer before being discarded.
     */
    private void scrollUp() {
        // Save the top line to scrollback before discarding
        char[] topLine = new char[TERMINAL_WIDTH];
        System.arraycopy(buffer[0], 0, topLine, 0, TERMINAL_WIDTH);
        scrollbackBuffer.add(topLine);
        
        // Limit scrollback buffer size to prevent memory bloat
        while (scrollbackBuffer.size() > SCROLLBACK_SIZE) {
            scrollbackBuffer.remove(0);
        }
        
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
     * Gets the number of lines in the scrollback buffer.
     */
    public int getScrollbackSize() {
        return scrollbackBuffer.size();
    }
    
    /**
     * Gets a line from the scrollback buffer.
     * Index 0 is the oldest line, getScrollbackSize()-1 is the most recent.
     * @param index The scrollback line index
     * @return The line as a string, or empty string if index is out of bounds
     */
    public String getScrollbackLine(int index) {
        if (index >= 0 && index < scrollbackBuffer.size()) {
            return new String(scrollbackBuffer.get(index));
        }
        return "";
    }
    
    /**
     * Clears the scrollback buffer.
     */
    public void clearScrollback() {
        scrollbackBuffer.clear();
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
            EvansComputerMod.LOGGER.info("Ctrl+T detected - interrupting WASM execution");
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
        EvansComputerMod.LOGGER.info("Terminal input: {}", line);
        
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
    
    /**
     * Called when a neighboring block changes.
     * Triggers peripheral rescan in the WASM host.
     */
    public void onNeighborChanged() {
        if (wasmHost != null) {
            wasmHost.rescanPeripherals();
        }
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
        
        // Save scrollback buffer
        tag.putInt("scrollbackSize", scrollbackBuffer.size());
        if (!scrollbackBuffer.isEmpty()) {
            StringBuilder scrollbackData = new StringBuilder();
            for (int i = 0; i < scrollbackBuffer.size(); i++) {
                scrollbackData.append(new String(scrollbackBuffer.get(i)));
                if (i < scrollbackBuffer.size() - 1) {
                    scrollbackData.append("\n");
                }
            }
            tag.putString("scrollback", scrollbackData.toString());
        }
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
        
        // Load scrollback buffer
        scrollbackBuffer.clear();
        if (tag.contains("scrollback")) {
            String data = tag.getString("scrollback");
            String[] lines = data.split("\n", -1);
            for (String line : lines) {
                if (scrollbackBuffer.size() >= SCROLLBACK_SIZE) {
                    break;
                }
                char[] chars = new char[TERMINAL_WIDTH];
                // Pad with spaces or truncate to TERMINAL_WIDTH
                for (int x = 0; x < TERMINAL_WIDTH; x++) {
                    chars[x] = (x < line.length()) ? line.charAt(x) : ' ';
                }
                scrollbackBuffer.add(chars);
            }
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
        return Component.translatable("container.evanscomputermod.terminal");
    }
    
    @Nullable
    @Override
    public AbstractContainerMenu createMenu(int containerId, Inventory playerInventory, Player player) {
        return new TerminalMenu(containerId, playerInventory, this);
    }

    /**
     * Runs a visual script by writing it to visual_program.py and executing it.
     * Called from the RunVisualScriptPacket handler.
     */
    public void runVisualScript(String pythonCode) {
        if (level == null || level.isClientSide) return;
        if (wasmHost == null) return;

        // Write the Python code to the computer's file system
        java.nio.file.Path computerDir = java.nio.file.Paths.get("computer-data", computerId.toString());
        try {
            java.nio.file.Files.createDirectories(computerDir);
            java.nio.file.Files.writeString(computerDir.resolve("visual_program.py"), pythonCode);
        } catch (java.io.IOException e) {
            EvansComputerMod.LOGGER.error("Failed to write visual_program.py", e);
            return;
        }

        // Send the command to run it as terminal input
        wasmHost.sendInput("python visual_program.py\n");
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

    /**
     * Sends a packet to nearby clients to open the visual programming editor.
     * Called from the WASM host when the user types 'visual' in the shell.
     */
    public void openVisualEditor() {
        if (level != null && !level.isClientSide) {
            var packet = new com.example.evanscomputermod.network.OpenVisualEditorPacket(getBlockPos());
            var chunkPos = new net.minecraft.world.level.ChunkPos(getBlockPos());
            net.neoforged.neoforge.network.PacketDistributor.sendToPlayersTrackingChunk(
                    (net.minecraft.server.level.ServerLevel) level, chunkPos, packet);
        }
    }
}
