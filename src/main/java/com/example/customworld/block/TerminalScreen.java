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
                int baseCharWidth = this.font.width("M");
                int cursorX = te.getCursorX() * baseCharWidth;
                int cursorY = te.getCursorY() * baseCharHeight;
                guiGraphics.fill(cursorX, cursorY, cursorX + baseCharWidth, cursorY + baseCharHeight, CURSOR_COLOR);
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
                int cursorX = textX + (te.getCursorX() * charWidth);
                int cursorY = textY + (te.getCursorY() * charHeight);
                guiGraphics.fill(cursorX, cursorY, cursorX + charWidth, cursorY + charHeight, CURSOR_COLOR);
            }
        }
    }
    
    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        // Handle special keys
        if (keyCode == 256) {  // Escape
            this.onClose();
            return true;
        }
        
        // Handle Enter
        if (keyCode == 257 || keyCode == 335) {  // Enter or Numpad Enter
            sendCharInput('\n');
            return true;
        }
        
        // Handle Backspace
        if (keyCode == 259) {  // Backspace
            sendCharInput('\b');
            return true;
        }
        
        // Let other keys go through to charTyped
        return false;
    }
    
    @Override
    public boolean charTyped(char codePoint, int modifiers) {
        // Send printable characters
        if (codePoint >= 32 && codePoint < 127) {
            sendCharInput(codePoint);
            return true;
        }
        return super.charTyped(codePoint, modifiers);
    }
    
    /**
     * Sends a character input to the server.
     */
    private void sendCharInput(char c) {
        // Send to server via packet
        PacketDistributor.sendToServer(new TerminalInputPacket(
                menu.getBlockEntity().getBlockPos(),
                c,
                false  // false = character input, not a special key
        ));
        
        // Also update locally for immediate feedback
        menu.getBlockEntity().onCharInput(c);
    }
    
    @Override
    public boolean isPauseScreen() {
        return false;  // Don't pause the game when terminal is open
    }
}
