package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.network.TerminalInputPacket;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.entity.player.Inventory;
import net.neoforged.neoforge.network.PacketDistributor;

/**
 * Client-side screen for the Terminal.
 * Renders the 80x24 terminal buffer and handles keyboard input.
 * Terminal size is dynamically calculated to fit the screen.
 */
public class TerminalScreen extends AbstractContainerScreen<TerminalMenu> {
    
    // Static configuration
    private static final int PADDING = 8;      // Padding around the terminal
    private static final float MAX_SCREEN_USAGE = 0.85f;  // Use at most 85% of screen
    
    // Colors (ARGB format)
    private static final int BACKGROUND_COLOR = 0xFF1A1A2E;  // Dark blue-black
    private static final int TEXT_COLOR = 0xFF00FF00;         // Green (classic terminal)
    private static final int CURSOR_COLOR = 0xFF00FF00;       // Green cursor
    private static final int BORDER_COLOR = 0xFF333355;       // Border color
    
    // Custom font resource location
    private static final ResourceLocation TERMINAL_FONT = 
            ResourceLocation.fromNamespaceAndPath(CustomWorldMod.MODID, "terminal");
    
    // Dynamic dimensions (calculated in init())
    private int charWidth;
    private int charHeight;
    private int terminalPixelWidth;
    private int terminalPixelHeight;
    private int screenWidth;
    private int screenHeight;
    private float scale = 1.0f;
    
    // Cursor blink timer
    private int cursorBlinkTimer = 0;
    private boolean cursorVisible = true;
    
    public TerminalScreen(TerminalMenu menu, Inventory playerInventory, Component title) {
        super(menu, playerInventory, title);
        // Disable inventory label rendering
        this.inventoryLabelY = Integer.MAX_VALUE;
        this.titleLabelY = Integer.MAX_VALUE;
    }
    
    @Override
    protected void init() {
        super.init();
        
        // Get actual font dimensions
        int baseCharWidth = this.font.width("M");  // Monospace reference character
        int baseCharHeight = this.font.lineHeight;
        
        // Calculate ideal terminal size at full scale
        int idealWidth = (TerminalBlockEntity.TERMINAL_WIDTH * baseCharWidth) + (PADDING * 2);
        int idealHeight = (TerminalBlockEntity.TERMINAL_HEIGHT * baseCharHeight) + (PADDING * 2);
        
        // Calculate scale to fit within the allowed screen space
        float maxWidth = this.width * MAX_SCREEN_USAGE;
        float maxHeight = this.height * MAX_SCREEN_USAGE;
        
        float scaleX = maxWidth / idealWidth;
        float scaleY = maxHeight / idealHeight;
        
        // Use the smaller scale to ensure both dimensions fit, cap at 1.0 (don't upscale)
        scale = Math.min(1.0f, Math.min(scaleX, scaleY));
        
        // Apply scale to character dimensions
        charWidth = Math.max(1, (int)(baseCharWidth * scale));
        charHeight = Math.max(1, (int)(baseCharHeight * scale));
        
        // Recalculate terminal dimensions with scaled characters
        terminalPixelWidth = TerminalBlockEntity.TERMINAL_WIDTH * charWidth;
        terminalPixelHeight = TerminalBlockEntity.TERMINAL_HEIGHT * charHeight;
        screenWidth = terminalPixelWidth + (PADDING * 2);
        screenHeight = terminalPixelHeight + (PADDING * 2);
        
        // Update image dimensions
        this.imageWidth = screenWidth;
        this.imageHeight = screenHeight;
        
        // Center the terminal on screen
        this.leftPos = (this.width - screenWidth) / 2;
        this.topPos = (this.height - screenHeight) / 2;
        
        CustomWorldMod.LOGGER.debug("Terminal screen initialized: {}x{} at scale {}", 
                screenWidth, screenHeight, scale);
    }
    
    @Override
    public void render(GuiGraphics guiGraphics, int mouseX, int mouseY, float partialTick) {
        // Update cursor blink
        cursorBlinkTimer++;
        if (cursorBlinkTimer >= 10) {  // Blink every 10 ticks
            cursorBlinkTimer = 0;
            cursorVisible = !cursorVisible;
        }
        
        // Render background darkening
        this.renderBackground(guiGraphics, mouseX, mouseY, partialTick);
        
        // Render terminal
        renderTerminal(guiGraphics);
        
        // Don't call super.render() to avoid rendering inventory slots
    }
    
    @Override
    protected void renderBg(GuiGraphics guiGraphics, float partialTick, int mouseX, int mouseY) {
        // Background is rendered in render() method
    }
    
    /**
     * Renders the terminal display.
     */
    private void renderTerminal(GuiGraphics guiGraphics) {
        int x = this.leftPos;
        int y = this.topPos;
        
        // Draw terminal background
        guiGraphics.fill(x, y, x + screenWidth, y + screenHeight, BACKGROUND_COLOR);
        
        // Draw border
        guiGraphics.fill(x, y, x + screenWidth, y + 2, BORDER_COLOR);  // Top
        guiGraphics.fill(x, y + screenHeight - 2, x + screenWidth, y + screenHeight, BORDER_COLOR);  // Bottom
        guiGraphics.fill(x, y, x + 2, y + screenHeight, BORDER_COLOR);  // Left
        guiGraphics.fill(x + screenWidth - 2, y, x + screenWidth, y + screenHeight, BORDER_COLOR);  // Right
        
        // Render terminal text
        int textX = x + PADDING;
        int textY = y + PADDING;
        
        TerminalBlockEntity te = menu.getBlockEntity();
        
        // Use pose stack for scaling if needed
        if (scale < 1.0f) {
            guiGraphics.pose().pushPose();
            guiGraphics.pose().translate(textX, textY, 0);
            guiGraphics.pose().scale(scale, scale, 1.0f);
            
            // Render text at origin (translation already applied)
            int baseCharHeight = this.font.lineHeight;
            for (int row = 0; row < TerminalBlockEntity.TERMINAL_HEIGHT; row++) {
                String line = te.getLine(row);
                guiGraphics.drawString(
                        this.font,
                        line,
                        0,
                        row * baseCharHeight,
                        TEXT_COLOR,
                        false  // No shadow
                );
            }
            
            // Draw cursor (scaled)
            if (cursorVisible) {
                String currentLine = te.getLine(te.getCursorY());
                int cursorCharX = Math.min(te.getCursorX(), currentLine.length());
                int cursorX = this.font.width(currentLine.substring(0, cursorCharX));
                int cursorY = te.getCursorY() * baseCharHeight;
                int cursorWidth = this.font.width("_");  // Use underscore width for cursor
                guiGraphics.fill(cursorX, cursorY, cursorX + cursorWidth, cursorY + baseCharHeight, CURSOR_COLOR);
            }
            
            guiGraphics.pose().popPose();
        } else {
            // No scaling needed, render normally
            for (int row = 0; row < TerminalBlockEntity.TERMINAL_HEIGHT; row++) {
                String line = te.getLine(row);
                guiGraphics.drawString(
                        this.font,
                        line,
                        textX,
                        textY + (row * charHeight),
                        TEXT_COLOR,
                        false  // No shadow
                );
            }
            
            // Draw cursor
            if (cursorVisible) {
                String currentLine = te.getLine(te.getCursorY());
                int cursorCharX = Math.min(te.getCursorX(), currentLine.length());
                int cursorX = textX + this.font.width(currentLine.substring(0, cursorCharX));
                int cursorY = textY + (te.getCursorY() * charHeight);
                int cursorWidth = this.font.width("_");  // Use underscore width for cursor
                guiGraphics.fill(cursorX, cursorY, cursorX + cursorWidth, cursorY + charHeight, CURSOR_COLOR);
            }
        }
    }
    
    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        boolean ctrlPressed = (modifiers & 2) != 0;  // GLFW_MOD_CONTROL = 2
        
        // Handle Escape - close the screen
        if (keyCode == 256) {  // Escape
            this.onClose();
            return true;
        }
        
        // Handle Ctrl+key combinations (send as control characters)
        if (ctrlPressed) {
            char ctrlChar = getCtrlChar(keyCode);
            if (ctrlChar != 0) {
                sendInput(String.valueOf(ctrlChar));
                return true;
            }
        }
        
        // Handle Arrow keys (send as ANSI escape sequences)
        switch (keyCode) {
            case 265:  // Arrow Up
                sendInput("\u001b[A");
                return true;
            case 264:  // Arrow Down
                sendInput("\u001b[B");
                return true;
            case 262:  // Arrow Right
                sendInput("\u001b[C");
                return true;
            case 263:  // Arrow Left
                sendInput("\u001b[D");
                return true;
            case 261:  // Delete
                sendInput("\u001b[3~");
                return true;
            case 257:  // Enter
            case 335:  // Numpad Enter
                sendInput("\n");
                return true;
            case 259:  // Backspace
                sendInput("\b");
                return true;
            case 258:  // Tab
                sendInput("\t");
                return true;
        }
        
        // Let other keys go through to charTyped
        return false;
    }
    
    /**
     * Maps a key code to its corresponding Ctrl character.
     * Returns 0 if the key doesn't have a Ctrl mapping.
     */
    private char getCtrlChar(int keyCode) {
        // Key codes for A-Z are 65-90 in GLFW
        // Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
        switch (keyCode) {
            case 65:  // A - Select All
                return '\u0001';
            case 67:  // C - Copy
                return '\u0003';
            case 68:  // D - Delete line
                return '\u0004';
            case 69:  // E - Exit
                return '\u0005';
            case 70:  // F - Find
                return '\u0006';
            case 75:  // K - Clear line
                return '\u000b';
            case 82:  // R - Run
                return '\u0012';
            case 83:  // S - Save
                return '\u0013';
            case 86:  // V - Paste
                return '\u0016';
            case 88:  // X - Cut
                return '\u0018';
            default:
                return 0;
        }
    }
    
    @Override
    public boolean charTyped(char codePoint, int modifiers) {
        // Send printable characters
        if (codePoint >= 32 && codePoint < 127) {
            sendInput(String.valueOf(codePoint));
            return true;
        }
        return super.charTyped(codePoint, modifiers);
    }
    
    /**
     * Sends input string to the server.
     */
    private void sendInput(String input) {
        // Send to server via packet
        PacketDistributor.sendToServer(new TerminalInputPacket(
                menu.getBlockEntity().getBlockPos(),
                input
        ));
        
        // Don't update locally - let the server/WASM handle all input and sync back
    }
    
    @Override
    public boolean isPauseScreen() {
        return false;  // Don't pause the game when terminal is open
    }
}
