package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.network.TerminalInputPacket;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FontDescription;
import net.minecraft.network.chat.Style;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.Inventory;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import com.example.evanscomputermod.client.TerminalGraphicsTexture;
import net.minecraft.client.renderer.RenderPipelines;

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
    private static final int CURSOR_COLOR = 0xFF00FF00;       // Green cursor
    private static final int BORDER_COLOR = 0xFF333355;       // Border color

    // 16-color ANSI palette (ARGB format)
    private static final int[] PALETTE = {
        0xFF000000, 0xFFAA0000, 0xFF00AA00, 0xFFAA5500,
        0xFF0000AA, 0xFFAA00AA, 0xFF00AAAA, 0xFFAAAAAA,
        0xFF555555, 0xFFFF5555, 0xFF55FF55, 0xFFFFFF55,
        0xFF5555FF, 0xFFFF55FF, 0xFF55FFFF, 0xFFFFFFFF
    };
    
    // Custom font resource location and style
    private static final Identifier TERMINAL_FONT =
            Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal");
    private static final Style TERMINAL_STYLE = Style.EMPTY.withFont(new FontDescription.Resource(TERMINAL_FONT));

    // Pre-cached Component objects for all printable ASCII characters (0x20-0x7E).
    // Avoids creating new Component + String objects per cell per frame.
    private static final Component[] CHAR_COMPONENTS = new Component[128];
    static {
        for (int ch = 0x20; ch < 0x7F; ch++) {
            CHAR_COMPONENTS[ch] = Component.literal(String.valueOf((char) ch)).withStyle(TERMINAL_STYLE);
        }
    }

    // Fixed character cell size matching the bitmap font (terminal_font.png is 128x256,
    // 16 chars per row = 8px wide, height: 16 in terminal.json = 16px tall).
    // We hardcode this instead of using font.width() because Minecraft's auto-width
    // detection gives variable per-glyph widths for bitmap fonts, breaking our fixed grid.
    private static final int FONT_CELL_WIDTH = 8;
    private static final int FONT_CELL_HEIGHT = 16;
    private static final int FONT_ASCENT = 14;  // from terminal.json — must match
    // Vertical offset applied to fill() rectangles so they align with where
    // drawString actually renders glyphs.  Minecraft's bitmap font renderer
    // positions glyphs above the Y coordinate by an amount related to ascent.
    // This value is tuned empirically to match the terminal font.
    private static final int GLYPH_Y_OFFSET = 4;

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

    // Graphics texture for pixel-based rendering
    private TerminalGraphicsTexture gfxTexture;
    private int lastGfxDisplayMode = 0;
    private int lastSeenPixelDirty = -1;
    private int lastSeenPaletteDirty = -1;
    
    public TerminalScreen(TerminalMenu menu, Inventory playerInventory, Component title) {
        super(menu, playerInventory, title);
        // Disable inventory label rendering
        this.inventoryLabelY = Integer.MAX_VALUE;
        this.titleLabelY = Integer.MAX_VALUE;
    }
    
    @Override
    protected void init() {
        super.init();
        
        // Fixed cell dimensions matching the bitmap font (terminal_font.png: 8x16 per glyph).
        // Don't use font.width() (variable per glyph) or font.lineHeight (returns 9, too small).
        int baseCharWidth = FONT_CELL_WIDTH;
        int baseCharHeight = FONT_CELL_HEIGHT;
        
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
        
        // Center the terminal on screen
        this.leftPos = (this.width - screenWidth) / 2;
        this.topPos = (this.height - screenHeight) / 2;
        
        EvansComputerMod.LOGGER.debug("Terminal screen initialized: {}x{} at scale {}",
                screenWidth, screenHeight, scale);

        // Request a keyframe from the server on screen open
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te != null) {
            ClientPacketDistributor.sendToServer(
                    new TerminalReadyPacket(te.getBlockPos(), 0));
        }
    }
    
    @Override
    public void extractRenderState(GuiGraphicsExtractor extractor, int mouseX, int mouseY, float partialTick) {
        // Update cursor blink
        cursorBlinkTimer++;
        if (cursorBlinkTimer >= 10) {  // Blink every 10 ticks
            cursorBlinkTimer = 0;
            cursorVisible = !cursorVisible;
        }

        // Render background darkening
        this.extractBackground(extractor, mouseX, mouseY, partialTick);

        // Render terminal
        renderTerminal(extractor);

        // Don't call super.extractRenderState() to avoid rendering inventory slots
    }

    @Override
    public void extractBackground(GuiGraphicsExtractor extractor, int mouseX, int mouseY, float partialTick) {
        // Background is rendered in extractRenderState() method
    }
    
    /**
     * Renders the terminal display using the memory-mapped framebuffer.
     * Each cell has a character, foreground color, and background color.
     * Supports text-only (mode 0), graphics-only (mode 1), and overlay (mode 2).
     */
    private void renderTerminal(GuiGraphicsExtractor gfx) {
        int x = this.leftPos;
        int y = this.topPos;

        // Draw terminal background
        gfx.fill(x, y, x + screenWidth, y + screenHeight, BACKGROUND_COLOR);

        // Draw border
        gfx.fill(x, y, x + screenWidth, y + 2, BORDER_COLOR);
        gfx.fill(x, y + screenHeight - 2, x + screenWidth, y + screenHeight, BORDER_COLOR);
        gfx.fill(x, y, x + 2, y + screenHeight, BORDER_COLOR);
        gfx.fill(x + screenWidth - 2, y, x + screenWidth, y + screenHeight, BORDER_COLOR);

        TerminalBlockEntity te = menu.getBlockEntity();
        com.example.evanscomputermod.computer.TerminalDisplay display = te.getDisplay();

        int termWidth = display.getWidth();
        int termHeight = display.getHeight();
        int baseCharWidth = FONT_CELL_WIDTH;
        int baseCharHeight = FONT_CELL_HEIGHT;
        int displayMode = display.getDisplayMode();

        // Manage graphics texture lifecycle
        updateGraphicsTexture(display, displayMode);

        // Render with scaling
        gfx.pose().pushMatrix();
        int textX = x + PADDING;
        int textY = y + PADDING;
        gfx.pose().translate(textX, textY);
        gfx.pose().scale(scale, scale);

        // --- Graphics layer (modes 1 and 2) ---
        if (displayMode >= 1 && gfxTexture != null) {
            renderGraphicsQuad(gfx, termWidth * baseCharWidth, termHeight * baseCharHeight);
        }

        // --- Text layer (modes 0 and 2) ---
        if (displayMode == 0 || displayMode == 2) {
            // Cursor position for inline rendering (inverted video style)
            int cx = te.getCursorX();
            int cy = te.getCursorY();
            boolean showCursor = cursorVisible && te.isCursorVisible();

            boolean isOverlay = displayMode == 2;

            // Render each cell with its color attributes
            for (int row = 0; row < termHeight; row++) {
                int rowY = row * baseCharHeight;
                int glyphY = rowY - GLYPH_Y_OFFSET;
                for (int col = 0; col < termWidth; col++) {
                    byte ch = display.getCharAt(col, row);
                    byte attr = display.getAttrAt(col, row);

                    int fgIdx = attr & 0x0F;
                    int bgIdx = (attr >> 4) & 0x0F;

                    int cellX = col * baseCharWidth;

                    boolean isCursor = showCursor && col == cx && row == cy;

                    // In overlay mode, only draw bg if non-transparent (bgIdx != 0)
                    if (bgIdx != 0 || !isOverlay) {
                        if (bgIdx != 0) {
                            gfx.fill(cellX, rowY, cellX + baseCharWidth, rowY + baseCharHeight, PALETTE[bgIdx]);
                        }
                    }
                    if (isCursor) {
                        gfx.fill(cellX, glyphY, cellX + baseCharWidth, glyphY + baseCharHeight, CURSOR_COLOR);
                    }

                    if (ch > 0x20 && ch < 0x7F) {
                        int charColor = isCursor ? PALETTE[0] : PALETTE[fgIdx];
                        gfx.text(this.font, CHAR_COMPONENTS[ch], cellX, rowY, charColor, false);
                    }
                }
            }
        }

        gfx.pose().popMatrix();
    }

    /**
     * Create, resize, or destroy the graphics texture as needed.
     */
    private void updateGraphicsTexture(com.example.evanscomputermod.computer.TerminalDisplay display, int displayMode) {
        if (displayMode >= 1) {
            int gfxW = display.getGfxWidth();
            int gfxH = display.getGfxHeight();
            if (gfxW > 0 && gfxH > 0) {
                if (gfxTexture == null) {
                    gfxTexture = new TerminalGraphicsTexture(gfxW, gfxH);
                    lastSeenPixelDirty = -1;
                    lastSeenPaletteDirty = -1;
                } else if (gfxTexture.getWidth() != gfxW || gfxTexture.getHeight() != gfxH) {
                    gfxTexture.resize(gfxW, gfxH);
                    lastSeenPixelDirty = -1;
                    lastSeenPaletteDirty = -1;
                }
                // Only upload to GPU when pixel data or palette actually changed
                int pixDirty = display.getPixelDirtyCounter();
                int palDirty = display.getPaletteDirtyCounter();
                if (pixDirty != lastSeenPixelDirty || palDirty != lastSeenPaletteDirty) {
                    gfxTexture.updateFull(display.getPixelData(), display.getPalette());
                    lastSeenPixelDirty = pixDirty;
                    lastSeenPaletteDirty = palDirty;
                }
            }
        } else if (gfxTexture != null) {
            gfxTexture.close();
            gfxTexture = null;
        }
        lastGfxDisplayMode = displayMode;
    }

    /**
     * Render the graphics framebuffer as a textured quad covering the terminal area.
     * DynamicTexture already uses NEAREST filtering by default.
     */
    private void renderGraphicsQuad(GuiGraphicsExtractor gfx, int termPixelW, int termPixelH) {
        int gfxW = gfxTexture.getWidth();
        int gfxH = gfxTexture.getHeight();
        gfx.blit(RenderPipelines.GUI_TEXTURED,
                gfxTexture.getTextureId(),
                0, 0,           // screen position (relative to matrix)
                0.0f, 0.0f,     // UV start
                termPixelW,     // screen width
                termPixelH,     // screen height
                gfxW, gfxH,     // source width/height
                gfxW, gfxH      // texture total width/height
        );
    }
    
    @Override
    public boolean keyPressed(KeyEvent event) {
        int keyCode = event.key();
        int scanCode = event.scancode();
        int modifiers = event.modifiers();
        boolean ctrlPressed = (modifiers & 2) != 0;  // GLFW_MOD_CONTROL = 2

        // Handle Escape - close the screen
        if (keyCode == 256) {  // Escape
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
    public boolean charTyped(CharacterEvent event) {
        char codePoint = (char) event.codepoint();
        // Send printable characters
        if (codePoint >= 32 && codePoint < 127) {
            sendInput(String.valueOf(codePoint));
            return true;
        }
        return super.charTyped(event);
    }
    
    @Override
    public boolean mouseScrolled(double mouseX, double mouseY, double scrollX, double scrollY) {
        // Scrollback is now managed by the Rust VTE (TODO: send scroll input to WASM)
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
        
        int baseCharHeight = FONT_CELL_HEIGHT;

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
        com.example.evanscomputermod.computer.TerminalDisplay display = te.getDisplay();
        StringBuilder lineSb = new StringBuilder();
        for (int col = 0; col < display.getWidth(); col++) {
            byte ch = display.getCharAt(col, charY);
            lineSb.append(ch >= 0x20 && ch < 0x7F ? (char) ch : ' ');
        }
        String line = lineSb.toString();
        
        double relativeX = (mouseX - textX);
        if (scale < 1.0f) {
            relativeX /= scale;  // Convert to unscaled coordinates
        }
        
        // Use fixed cell width for click detection (monospace grid)
        int charX = Math.min((int)(relativeX / FONT_CELL_WIDTH), TerminalBlockEntity.TERMINAL_WIDTH - 1);
        charX = Math.max(0, charX);
        charX = Math.min(charX, TerminalBlockEntity.TERMINAL_WIDTH - 1);
        
        return new int[]{charX, charY};
    }
    
    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean focused) {
        double mouseX = event.x();
        double mouseY = event.y();
        int button = event.button();
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
        return super.mouseClicked(event, focused);
    }
    
    @Override
    public boolean mouseDragged(MouseButtonEvent event, double dragX, double dragY) {
        double mouseX = event.x();
        double mouseY = event.y();
        int button = event.button();
        if (button == 0 && isSelecting) {
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                hasSelection = (selectionStartX != selectionEndX || selectionStartY != selectionEndY);
                return true;
            }
        }
        return super.mouseDragged(event, dragX, dragY);
    }
    
    @Override
    public boolean mouseReleased(MouseButtonEvent event) {
        double mouseX = event.x();
        double mouseY = event.y();
        int button = event.button();
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
        return super.mouseReleased(event);
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
        com.example.evanscomputermod.computer.TerminalDisplay display = te.getDisplay();

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
            startX = Math.min(selectionStartX, selectionEndX);
            endX = Math.max(selectionStartX, selectionEndX);
        }

        StringBuilder sb = new StringBuilder();

        for (int row = startY; row <= endY; row++) {
            // Build line from framebuffer cells
            StringBuilder lineSb = new StringBuilder();
            for (int col = 0; col < display.getWidth(); col++) {
                byte ch = display.getCharAt(col, row);
                lineSb.append(ch >= 0x20 && ch < 0x7F ? (char) ch : ' ');
            }
            String line = lineSb.toString();
            
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
        ClientPacketDistributor.sendToServer(new TerminalInputPacket(
                menu.getBlockEntity().getBlockPos(),
                input,
                java.util.Optional.empty()
        ));
        
        // Don't update locally - let the server/WASM handle all input and sync back
    }
    
    @Override
    public void onClose() {
        if (gfxTexture != null) {
            gfxTexture.close();
            gfxTexture = null;
        }
        super.onClose();
    }

    @Override
    public boolean isPauseScreen() {
        return false;  // Don't pause the game when terminal is open
    }
}
