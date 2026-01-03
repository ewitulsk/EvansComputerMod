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
    private String wasmModule = "terminal";
    private String wasmFunction = "main";
    
    // WASM host for executing terminal programs (server-side only)
    @Nullable
    private TerminalWasmHost wasmHost;
    private boolean wasmInitialized = false;
    
    public TerminalBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.TERMINAL_BLOCK_ENTITY.get(), pos, state);
        clearBuffer();
    }
    
    /**
     * Initializes the WASM host and loads the configured module.
     * Called when the terminal is first opened.
     */
    public void initializeWasm() {
        if (wasmInitialized || level == null || level.isClientSide) {
            return;
        }
        
        wasmInitialized = true;
        
        try {
            wasmHost = new TerminalWasmHost(this);
            wasmHost.loadModule(wasmModule);
            
            // Execute the main function
            wasmHost.executeMain();
            
            CustomWorldMod.LOGGER.info("Initialized WASM terminal with module: {}", wasmModule);
            
        } catch (WasmManager.WasmExecutionException e) {
            // Write error to terminal
            write("Error loading WASM module: " + wasmModule + "\n");
            write(e.getMessage() + "\n");
            write("\nPlace a .wasm file in wasm-bin/ directory.\n");
            CustomWorldMod.LOGGER.error("Failed to initialize WASM terminal", e);
        }
    }
    
    /**
     * Shuts down the WASM host when the block entity is removed.
     */
    @Override
    public void setRemoved() {
        super.setRemoved();
        if (wasmHost != null) {
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
     * Handles character input from the user.
     */
    public void onCharInput(char c) {
        if (characterMode) {
            // In character mode, echo immediately and process
            writeChar(c);
            // TODO: Send to WASM for processing
        } else {
            // In line mode, buffer the input
            if (c == '\n' || c == '\r') {
                // Submit the line
                String line = inputLine.toString();
                inputLine = new StringBuilder();
                write("\n");
                // TODO: Send line to WASM for processing
                onLineInput(line);
            } else if (c == '\b') {
                // Backspace in line mode
                if (inputLine.length() > 0) {
                    inputLine.deleteCharAt(inputLine.length() - 1);
                    writeChar('\b');
                }
            } else {
                inputLine.append(c);
                writeChar(c);
            }
        }
        setChanged();
    }
    
    /**
     * Called when a complete line is entered (in line mode).
     */
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
    
    // NBT serialization
    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putString("buffer", getBufferAsString());
        tag.putInt("cursorX", cursorX);
        tag.putInt("cursorY", cursorY);
        tag.putBoolean("characterMode", characterMode);
        tag.putString("wasmModule", wasmModule);
        tag.putString("wasmFunction", wasmFunction);
    }
    
    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
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
