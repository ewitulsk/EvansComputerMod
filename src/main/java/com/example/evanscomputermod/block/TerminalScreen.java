package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.network.TerminalInputPacket;
import net.minecraft.client.Minecraft;
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
            ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal");
    
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
    
    // Scroll state - how many lines scrolled back from current (0 = at bottom, showing live terminal)
    private int scrollOffset = 0;
    
    // Text selection state
    private boolean isSelecting = false;
    private boolean hasSelection = false;
    private int selectionStartX = 0;  // Character column
    private int selectionStartY = 0;  // Row (in visible coordinates, 0 = top of visible area)
    private int selectionEndX = 0;
    private int selectionEndY = 0;
    
    // Selection color (semi-transparent blue)
    private static final int SELECTION_COLOR = 0x804444FF;
    
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
        
        // Recalculate terminal dimensions using actual scaled size (ceiling to ensure content fits)
        terminalPixelWidth = (int) Math.ceil(TerminalBlockEntity.TERMINAL_WIDTH * baseCharWidth * scale);
        terminalPixelHeight = (int) Math.ceil(TerminalBlockEntity.TERMINAL_HEIGHT * baseCharHeight * scale);
        screenWidth = terminalPixelWidth + (PADDING * 2);
        screenHeight = terminalPixelHeight + (PADDING * 2);
        
        // Update image dimensions
        this.imageWidth = screenWidth;
        this.imageHeight = screenHeight;
        
        // Center the terminal on screen
        this.leftPos = (this.width - screenWidth) / 2;
        this.topPos = (this.height - screenHeight) / 2;
        
        EvansComputerMod.LOGGER.debug("Terminal screen initialized: {}x{} at scale {}", 
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
     * Gets a line to display, accounting for scroll offset.
     * When scrollOffset > 0, we're viewing history.
     * Row 0 is at the top of the visible area.
     * 
     * @param te The terminal block entity
     * @param visibleRow The row on screen (0 to TERMINAL_HEIGHT-1)
     * @return The line content to display
     */
    private String getDisplayLine(TerminalBlockEntity te, int visibleRow) {
        int scrollbackSize = te.getScrollbackSize();
        
        if (scrollOffset == 0) {
            // Not scrolled - show current buffer
            return te.getLine(visibleRow);
        }
        
        // Calculate which line in the total history to show
        // Total lines = scrollback + current buffer (TERMINAL_HEIGHT lines)
        // When at bottom (scrollOffset=0): show buffer lines 0-23
        // When scrollOffset=1: show scrollback[last] + buffer lines 0-22
        // When scrollOffset=scrollbackSize: show scrollback lines 0-23
        
        // The line index from the end of scrollback
        // scrollOffset tells us how many lines up from the bottom we've scrolled
        int lineFromBottom = (TerminalBlockEntity.TERMINAL_HEIGHT - 1 - visibleRow) + scrollOffset;
        
        if (lineFromBottom < TerminalBlockEntity.TERMINAL_HEIGHT) {
            // This line is in the current buffer
            int bufferRow = TerminalBlockEntity.TERMINAL_HEIGHT - 1 - lineFromBottom;
            return te.getLine(bufferRow);
        } else {
            // This line is in the scrollback buffer
            int scrollbackIndex = scrollbackSize - 1 - (lineFromBottom - TerminalBlockEntity.TERMINAL_HEIGHT);
            if (scrollbackIndex >= 0 && scrollbackIndex < scrollbackSize) {
                return te.getScrollbackLine(scrollbackIndex);
            } else {
                return ""; // Beyond scrollback
            }
        }
    }
    
    /**
     * Renders the selection highlight.
     */
    private void renderSelection(GuiGraphics guiGraphics, int textX, int textY) {
        if (!hasSelection && !isSelecting) {
            return;
        }
        
        // Normalize selection bounds (ensure start is before end)
        int startY = Math.min(selectionStartY, selectionEndY);
        int endY = Math.max(selectionStartY, selectionEndY);
        int startX, endX;
        
        if (selectionStartY < selectionEndY) {
            startX = selectionStartX;
            endX = selectionEndX;
        } else if (selectionStartY > selectionEndY) {
            startX = selectionEndX;
            endX = selectionStartX;
        } else {
            // Same row
            startX = Math.min(selectionStartX, selectionEndX);
            endX = Math.max(selectionStartX, selectionEndX);
        }
        
        int baseCharWidth = this.font.width("M");
        int baseCharHeight = this.font.lineHeight;
        
        for (int row = startY; row <= endY; row++) {
            if (row < 0 || row >= TerminalBlockEntity.TERMINAL_HEIGHT) {
                continue;
            }
            
            int rowStartX = (row == startY) ? startX : 0;
            int rowEndX = (row == endY) ? endX : TerminalBlockEntity.TERMINAL_WIDTH - 1;
            
            // Calculate pixel positions
            int pixelStartX, pixelEndX, pixelY, rowHeight;
            
            if (scale < 1.0f) {
                pixelStartX = textX + (int)(rowStartX * baseCharWidth * scale);
                pixelEndX = textX + (int)((rowEndX + 1) * baseCharWidth * scale);
                pixelY = textY + (int)(row * baseCharHeight * scale);
                rowHeight = (int)(baseCharHeight * scale);
            } else {
                pixelStartX = textX + (rowStartX * charWidth);
                pixelEndX = textX + ((rowEndX + 1) * charWidth);
                pixelY = textY + (row * charHeight);
                rowHeight = charHeight;
            }
            
            // Draw selection highlight
            guiGraphics.fill(pixelStartX, pixelY, pixelEndX, pixelY + rowHeight, SELECTION_COLOR);
        }
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
        
        // Draw selection highlight (before text so text appears on top)
        renderSelection(guiGraphics, textX, textY);
        
        TerminalBlockEntity te = menu.getBlockEntity();
        
        // Use pose stack for scaling if needed
        if (scale < 1.0f) {
            guiGraphics.pose().pushPose();
            guiGraphics.pose().translate(textX, textY, 0);
            guiGraphics.pose().scale(scale, scale, 1.0f);
            
            // Render text at origin (translation already applied)
            int baseCharHeight = this.font.lineHeight;
            for (int row = 0; row < TerminalBlockEntity.TERMINAL_HEIGHT; row++) {
                String line = getDisplayLine(te, row);
                guiGraphics.drawString(
                        this.font,
                        line,
                        0,
                        row * baseCharHeight,
                        TEXT_COLOR,
                        false  // No shadow
                );
            }
            
            // Draw cursor only when not scrolled (cursor is only relevant for live view)
            if (cursorVisible && scrollOffset == 0) {
                String currentLine = te.getLine(te.getCursorY());
                int cursorCharX = Math.min(te.getCursorX(), currentLine.length());
                int cursorX = this.font.width(currentLine.substring(0, cursorCharX));
                int cursorY = te.getCursorY() * baseCharHeight;
                int cursorWidth = this.font.width("_");
                
                // Clamp cursor to terminal bounds (in unscaled coordinates)
                int maxX = (int)(terminalPixelWidth / scale);
                int maxY = (int)(terminalPixelHeight / scale);
                cursorX = Math.min(cursorX, maxX - cursorWidth);
                cursorY = Math.min(cursorY, maxY - baseCharHeight);
                
                guiGraphics.fill(cursorX, cursorY, cursorX + cursorWidth, cursorY + baseCharHeight, CURSOR_COLOR);
            }
            
            guiGraphics.pose().popPose();
        } else {
            // No scaling needed, render normally
            for (int row = 0; row < TerminalBlockEntity.TERMINAL_HEIGHT; row++) {
                String line = getDisplayLine(te, row);
                guiGraphics.drawString(
                        this.font,
                        line,
                        textX,
                        textY + (row * charHeight),
                        TEXT_COLOR,
                        false  // No shadow
                );
            }
            
            // Draw cursor only when not scrolled
            if (cursorVisible && scrollOffset == 0) {
                String currentLine = te.getLine(te.getCursorY());
                int cursorCharX = Math.min(te.getCursorX(), currentLine.length());
                int cursorX = textX + this.font.width(currentLine.substring(0, cursorCharX));
                int cursorY = textY + (te.getCursorY() * charHeight);
                int cursorWidth = this.font.width("_");
                
                // Clamp cursor to terminal bounds
                int maxX = textX + terminalPixelWidth;
                int maxY = textY + terminalPixelHeight;
                cursorX = Math.min(cursorX, maxX - cursorWidth);
                cursorY = Math.min(cursorY, maxY - charHeight);
                
                guiGraphics.fill(cursorX, cursorY, cursorX + cursorWidth, cursorY + charHeight, CURSOR_COLOR);
            }
        }
        
    }
    
    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        boolean ctrlPressed = (modifiers & 2) != 0;  // GLFW_MOD_CONTROL = 2
        
        // Handle Escape - forward to WASM for VIM mode switching
        if (keyCode == 256) {  // Escape
            sendInput("\u001b");
            return true;
        }

        // Handle Ctrl+Q - close the screen
        if (ctrlPressed && keyCode == 81) {  // Q
            this.onClose();
            return true;
        }

        // Handle Ctrl+key combinations
        if (ctrlPressed) {
            // Ctrl+C - Copy to clipboard if there's a selection
            if (keyCode == 67) {  // C
                if (hasSelection) {
                    String selectedText = getSelectedText();
                    if (!selectedText.isEmpty()) {
                        Minecraft.getInstance().keyboardHandler.setClipboard(selectedText);
                        EvansComputerMod.LOGGER.debug("Copied to clipboard: {} chars", selectedText.length());
                    }
                    clearSelection();
                    return true;
                }
                // No selection - send Ctrl+C to WASM (for interrupt, etc.)
                sendInput("\u0003");
                return true;
            }
            
            // Ctrl+V - Paste from clipboard
            if (keyCode == 86) {  // V
                String clipboard = Minecraft.getInstance().keyboardHandler.getClipboard();
                if (clipboard != null && !clipboard.isEmpty()) {
                    // Send clipboard content as input (supports multi-line)
                    sendInput(clipboard);
                    EvansComputerMod.LOGGER.debug("Pasted from clipboard: {} chars", clipboard.length());
                }
                return true;
            }
            
            // Other Ctrl+key combinations - send as control characters
            char ctrlChar = getCtrlChar(keyCode);
            if (ctrlChar != 0) {
                sendInput(String.valueOf(ctrlChar));
                return true;
            }
        }
        
        // Any key press clears selection (except modifiers)
        if (keyCode != 341 && keyCode != 345 && keyCode != 340 && keyCode != 344) {  // Not Ctrl/Shift
            clearSelection();
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
     * Note: Ctrl+C and Ctrl+V are handled separately for clipboard operations.
     */
    private char getCtrlChar(int keyCode) {
        // Key codes for A-Z are 65-90 in GLFW
        // Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
        // Note: C (67) and V (86) are handled separately for copy/paste
        switch (keyCode) {
            case 65:  // A - Select All
                return '\u0001';
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
            case 84:  // T - Terminate/kill program
                return '\u0014';
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
    
    @Override
    public boolean mouseScrolled(double mouseX, double mouseY, double scrollX, double scrollY) {
        TerminalBlockEntity te = menu.getBlockEntity();
        int maxScroll = te.getScrollbackSize();
        
        // Scroll up (positive scrollY) increases offset, scroll down decreases
        // Each scroll tick moves 3 lines
        int scrollAmount = (int) (scrollY * 3);
        scrollOffset = Math.max(0, Math.min(maxScroll, scrollOffset - scrollAmount));
        
        return true;
    }
    
    /**
     * Gets the current scroll offset.
     * @return 0 if at bottom (live view), positive if scrolled back into history
     */
    public int getScrollOffset() {
        return scrollOffset;
    }
    
    /**
     * Resets scroll to the bottom (live view).
     */
    public void scrollToBottom() {
        scrollOffset = 0;
    }
    
    /**
     * Converts mouse coordinates to terminal character position.
     * Uses character-by-character width calculation for accurate positioning
     * with variable-width fonts.
     * @return int[2] with {charX, charY} or null if outside terminal area
     */
    private int[] mouseToCharPos(double mouseX, double mouseY) {
        int textX = this.leftPos + PADDING;
        int textY = this.topPos + PADDING;
        
        // Check if mouse is within terminal text area
        if (mouseX < textX || mouseX >= textX + terminalPixelWidth ||
            mouseY < textY || mouseY >= textY + terminalPixelHeight) {
            return null;
        }
        
        int baseCharHeight = this.font.lineHeight;
        
        // Calculate row (Y is uniform height)
        int charY;
        if (scale < 1.0f) {
            charY = (int) ((mouseY - textY) / (baseCharHeight * scale));
        } else {
            charY = (int) ((mouseY - textY) / charHeight);
        }
        charY = Math.max(0, Math.min(TerminalBlockEntity.TERMINAL_HEIGHT - 1, charY));
        
        // Calculate column by iterating through characters
        // This handles variable-width fonts correctly
        TerminalBlockEntity te = menu.getBlockEntity();
        String line = getDisplayLine(te, charY);
        
        double relativeX = (mouseX - textX);
        if (scale < 1.0f) {
            relativeX /= scale;  // Convert to unscaled coordinates
        }
        
        int charX = 0;
        int accumulatedWidth = 0;
        for (int i = 0; i < line.length() && i < TerminalBlockEntity.TERMINAL_WIDTH; i++) {
            int charPixelWidth = this.font.width(String.valueOf(line.charAt(i)));
            // Click in first half of character = this character, second half = next character
            if (accumulatedWidth + charPixelWidth / 2 > relativeX) {
                break;
            }
            accumulatedWidth += charPixelWidth;
            charX = i + 1;
        }
        charX = Math.min(charX, TerminalBlockEntity.TERMINAL_WIDTH - 1);
        
        return new int[]{charX, charY};
    }
    
    @Override
    public boolean mouseClicked(double mouseX, double mouseY, int button) {
        if (button == 0) {  // Left click
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                // Start selection
                isSelecting = true;
                hasSelection = false;
                selectionStartX = charPos[0];
                selectionStartY = charPos[1];
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                return true;
            }
        }
        return super.mouseClicked(mouseX, mouseY, button);
    }
    
    @Override
    public boolean mouseDragged(double mouseX, double mouseY, int button, double dragX, double dragY) {
        if (button == 0 && isSelecting) {
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                hasSelection = (selectionStartX != selectionEndX || selectionStartY != selectionEndY);
                return true;
            }
        }
        return super.mouseDragged(mouseX, mouseY, button, dragX, dragY);
    }
    
    @Override
    public boolean mouseReleased(double mouseX, double mouseY, int button) {
        if (button == 0 && isSelecting) {
            isSelecting = false;
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                
                // Check if this was a click (not a drag) - set cursor position
                boolean wasClick = (selectionStartX == selectionEndX && selectionStartY == selectionEndY);
                hasSelection = !wasClick;
                
                if (wasClick && scrollOffset == 0) {
                    // Single click - send cursor position to terminal
                    // Only works when not scrolled (can't click in history)
                    // Use ANSI CSI sequence: ESC [ row ; col H (1-based)
                    String cursorPosSequence = String.format("\u001b[%d;%dH", 
                            selectionEndY + 1, selectionEndX + 1);
                    sendInput(cursorPosSequence);
                    EvansComputerMod.LOGGER.debug("Click to cursor: {},{}", selectionEndX, selectionEndY);
                }
            }
            return true;
        }
        return super.mouseReleased(mouseX, mouseY, button);
    }
    
    /**
     * Clears the current selection.
     */
    private void clearSelection() {
        hasSelection = false;
        isSelecting = false;
    }
    
    /**
     * Gets the selected text.
     * @return The selected text, or empty string if no selection
     */
    private String getSelectedText() {
        if (!hasSelection) {
            return "";
        }
        
        TerminalBlockEntity te = menu.getBlockEntity();
        
        // Normalize selection (ensure start is before end)
        int startY = Math.min(selectionStartY, selectionEndY);
        int endY = Math.max(selectionStartY, selectionEndY);
        int startX, endX;
        
        if (selectionStartY < selectionEndY) {
            startX = selectionStartX;
            endX = selectionEndX;
        } else if (selectionStartY > selectionEndY) {
            startX = selectionEndX;
            endX = selectionStartX;
        } else {
            // Same row
            startX = Math.min(selectionStartX, selectionEndX);
            endX = Math.max(selectionStartX, selectionEndX);
        }
        
        StringBuilder sb = new StringBuilder();
        
        for (int row = startY; row <= endY; row++) {
            String line = getDisplayLine(te, row);
            
            int lineStart = (row == startY) ? startX : 0;
            int lineEnd = (row == endY) ? endX + 1 : line.length();
            
            // Clamp to line length
            lineStart = Math.min(lineStart, line.length());
            lineEnd = Math.min(lineEnd, line.length());
            
            if (lineStart < lineEnd) {
                sb.append(line.substring(lineStart, lineEnd));
            }
            
            // Add newline between lines (but not after last line)
            if (row < endY) {
                sb.append("\n");
            }
        }
        
        // Trim trailing spaces from each line but preserve newlines
        String result = sb.toString();
        String[] lines = result.split("\n", -1);
        StringBuilder trimmed = new StringBuilder();
        for (int i = 0; i < lines.length; i++) {
            trimmed.append(lines[i].stripTrailing());
            if (i < lines.length - 1) {
                trimmed.append("\n");
            }
        }
        
        return trimmed.toString();
    }
    
    /**
     * Sends input string to the server.
     */
    private void sendInput(String input) {
        // Auto-scroll to bottom when user types
        scrollOffset = 0;
        
        // Send to server via packet
        PacketDistributor.sendToServer(new TerminalInputPacket(
                menu.getBlockEntity().getBlockPos(),
                input,
                java.util.Optional.empty()
        ));
        
        // Don't update locally - let the server/WASM handle all input and sync back
    }
    
    @Override
    public boolean isPauseScreen() {
        return false;  // Don't pause the game when terminal is open
    }

    @Override
    public boolean shouldCloseOnEsc() {
        return false;  // ESC is forwarded to WASM for VIM — use Ctrl+Q to close
    }
}
