package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.network.MouseInputPacket;
import com.example.evanscomputermod.network.TerminalInputPacket;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import net.minecraft.client.Minecraft;
//? if >=26.1 {
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
//?} else
/*import net.minecraft.client.gui.GuiGraphics;*/
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
//? if >=26.1 {
import net.minecraft.network.chat.FontDescription;
//?}
import net.minecraft.network.chat.Style;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.Inventory;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import com.example.evanscomputermod.client.TerminalGraphicsTexture;
//? if >=26.1 {
import net.minecraft.client.renderer.RenderPipelines;
//?}

/**
 * Client-side screen for the Terminal.
 * Renders the 80x24 terminal buffer and handles keyboard input.
 * Terminal size is dynamically calculated to fit the screen.
 */
public class TerminalScreen extends AbstractContainerScreen<TerminalMenu>
        implements com.example.evanscomputermod.client.ComputerInputScreen {
    
    // Static configuration
    private static final int PADDING = 8;      // Padding around the terminal
    private static final float MAX_SCREEN_USAGE = 0.85f;  // Use at most 85% of screen
    
    // Colors (ARGB format)
    private static final int BACKGROUND_COLOR = 0xFF1A1A2E;  // Dark blue-black
    private static final int CURSOR_COLOR = 0xFF00FF00;       // Green cursor
    private static final int BORDER_COLOR = 0xFF333355;       // Border color

    // 16-color ANSI palette (ARGB format)
    /**
     * The terminal area is the same physical size whatever the panel: 160x50
     * cells of the 8x16 font (Tier 2's grid). A smaller grid (Tier 1's 80x25)
     * is drawn with bigger cells, so a cheap panel looks chunky, not tiny.
     */
    private static final int AREA_W = 160 * 8;
    private static final int AREA_H = 50 * 16;

    private static final int[] PALETTE = {
        0xFF000000, 0xFFAA0000, 0xFF00AA00, 0xFFAA5500,
        0xFF0000AA, 0xFFAA00AA, 0xFF00AAAA, 0xFFAAAAAA,
        0xFF555555, 0xFFFF5555, 0xFF55FF55, 0xFFFFFF55,
        0xFF5555FF, 0xFFFF55FF, 0xFF55FFFF, 0xFFFFFFFF
    };
    
    // Custom font resource location and style
    private static final Identifier TERMINAL_FONT =
            Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal");
    //? if >=26.1 {
    private static final Style TERMINAL_STYLE = Style.EMPTY.withFont(new FontDescription.Resource(TERMINAL_FONT));
    //?} else
    /*private static final Style TERMINAL_STYLE = Style.EMPTY.withFont(TERMINAL_FONT);*/

    // Pre-cached Component objects for all printable ASCII characters (0x20-0x7E).
    // Avoids creating new Component + String objects per cell per frame.
    private static final Component[] CHAR_COMPONENTS = new Component[128];
    /** The same in bold, for cells with the bold flag. */
    private static final Component[] BOLD_CHAR_COMPONENTS = new Component[128];
    static {
        for (int ch = 0x20; ch < 0x7F; ch++) {
            CHAR_COMPONENTS[ch] = Component.literal(String.valueOf((char) ch)).withStyle(TERMINAL_STYLE);
            BOLD_CHAR_COMPONENTS[ch] = Component.literal(String.valueOf((char) ch))
                    .withStyle(TERMINAL_STYLE.withBold(true));
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

    // Mouse capture state. `lastButtonsHeld` is a bitmask (bit0=L, bit1=R,
    // bit2=M) that the guest can read out of the event's `buttons` field.
    // `lastSentFbX/Y` throttles move events — the cursor can travel several
    // sub-pixel steps inside one framebuffer pixel and we don't want to
    // flood the server with duplicates.
    private byte lastButtonsHeld = 0;
    private int lastSentFbX = -1;
    private int lastSentFbY = -1;
    
    // --- Wireless controller toggle ---
    /** A controller in the player's inventory paired with this computer, if any. */
    @org.jetbrains.annotations.Nullable private java.util.UUID pairedController;
    private boolean controllerAutoConnected;

    private boolean controllerOn() {
        return pairedController != null
                && pairedController.equals(com.example.evanscomputermod.controller.client.ControllerClient.guiController());
    }

    /**
     * Whether the controller's bound keys drive it right now. Only while a
     * program shows graphics on the Terminal (display mode 1): at the shell,
     * or with the text console overlaid, the keyboard types as usual, so you
     * can start a game with the controller already on.
     */
    public boolean controllerKeysActive() {
        if (!controllerOn()) return false;
        TerminalBlockEntity te = menu.getBlockEntity();
        return te != null && te.getDisplay().getDisplayMode() == 1;
    }

    @org.jetbrains.annotations.Nullable
    private net.minecraft.world.item.ItemStack pairedControllerStack() {
        var player = net.minecraft.client.Minecraft.getInstance().player;
        TerminalBlockEntity te = menu.getBlockEntity();
        if (player == null || te == null || te.getLevel() == null) return null;
        var inv = player.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            var st = inv.getItem(i);
            if (st.getItem() instanceof com.example.evanscomputermod.controller.WirelessControllerItem
                    && com.example.evanscomputermod.controller.WirelessControllerItem.isPairedWith(st, te.getLevel(), te.getBlockPos())
                    && com.example.evanscomputermod.controller.ControllerData.id(st) != null) {
                return st;
            }
        }
        return null;
    }

    /** The toggle's clickable area, just above the terminal's top-right corner. */
    private int[] controllerToggleRect() {
        int w = 150;
        int y1 = Math.max(12, topPos - 2);
        return new int[]{leftPos + screenWidth - w, y1 - 12, leftPos + screenWidth, y1};
    }

    private void renderControllerToggle(com.example.evanscomputermod.client.GuiGfx g, int mouseX, int mouseY) {
        if (pairedController == null) return;
        int[] r = controllerToggleRect();
        boolean on = controllerOn();
        boolean over = mouseX >= r[0] && mouseX < r[2] && mouseY >= r[1] && mouseY < r[3];
        g.box(r[0], r[1], r[2], r[3], over ? 0xFF30363D : 0xFF161B22, on ? 0xFF3FB950 : 0xFF484F58);
        String label;
        if (!on) {
            label = "Controller: off (click)";
        } else {
            int player = com.example.evanscomputermod.controller.client.ControllerClient.player();
            label = player <= 0
                    ? "Controller: " + com.example.evanscomputermod.controller.client.ControllerClient.statusMessage()
                    : "Controller " + player + (controllerKeysActive() ? ": playing" : ": on (typing)")
                            + com.example.evanscomputermod.controller.ControllerHudText.buttonSuffix(
                                    com.example.evanscomputermod.controller.client.ControllerClient.statusMessage());
        }
        g.centeredText(this.font, label, (r[0] + r[2]) / 2, r[1] + 2, on ? 0xFFE6EDF3 : 0xFF8B949E);
    }

    private boolean clickControllerToggle(double mouseX, double mouseY, int button) {
        if (pairedController == null || button != 0) return false;
        int[] r = controllerToggleRect();
        if (mouseX < r[0] || mouseX >= r[2] || mouseY < r[1] || mouseY >= r[3]) return false;
        com.example.evanscomputermod.controller.client.ControllerClient.setGuiController(
                controllerOn() ? null : pairedController);
        return true;
    }

    /** While the controller is on, its keys drive it instead of typing into the terminal. */
    /** The character a bound key types (its key press was already swallowed). */
    private boolean isControllerChar(char c) {
        if (!controllerKeysActive()) return false;
        String name = c == ' ' ? "key.keyboard.space" : "key.keyboard." + Character.toLowerCase(c);
        var key = com.example.evanscomputermod.controller.client.ControllerClient.key(name);
        return key != null && isControllerKey(key);
    }

    /** Whether a key press belongs to the controller (ControllerClient cancels it before other mods see it). */
    public boolean swallowsControllerKey(int keyCode) {
        return keyCode != 256 // Escape always closes
                && isControllerKey(com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM.getOrCreate(keyCode));
    }

    /** Whether a typed character comes from a controller key. */
    public boolean swallowsControllerChar(char c) {
        return isControllerChar(c);
    }

    private boolean isControllerKey(com.mojang.blaze3d.platform.InputConstants.Key key) {
        if (!controllerKeysActive()) return false;
        var st = pairedControllerStack();
        return st != null && com.example.evanscomputermod.controller.client.ControllerClient.isBound(st, key);
    }

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
        int idealWidth = AREA_W + (PADDING * 2);
        int idealHeight = AREA_H + (PADDING * 2);
        
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
        terminalPixelWidth = (int) Math.ceil(AREA_W * scale);
        terminalPixelHeight = (int) Math.ceil(AREA_H * scale);
        screenWidth = terminalPixelWidth + (PADDING * 2);
        screenHeight = terminalPixelHeight + (PADDING * 2);
        
        // Center the terminal on screen
        this.leftPos = (this.width - screenWidth) / 2;
        this.topPos = (this.height - screenHeight) / 2;
        // The GUI's real extent: overlays (JEI) place themselves beside imageWidth x imageHeight,
        // which was still the default 176x166 and put them over the terminal's right part.
        // On 26.1 the fields are final; ComputerScreenInput still keeps clicks away from overlays.
        //? if <26.1 {
        this.imageWidth = screenWidth;
        this.imageHeight = screenHeight;
        //?}

        EvansComputerMod.LOGGER.debug("Terminal screen initialized: {}x{} at scale {}",
                screenWidth, screenHeight, scale);

        var controllerStack = pairedControllerStack();
        pairedController = controllerStack == null ? null
                : com.example.evanscomputermod.controller.ControllerData.id(controllerStack);
        // A paired controller in the inventory connects as soon as the Terminal
        // opens (init also runs on resize: don't reconnect one switched off).
        if (pairedController != null && !controllerAutoConnected) {
            controllerAutoConnected = true;
            com.example.evanscomputermod.controller.client.ControllerClient.setGuiController(pairedController);
        }

        // Request a keyframe from the server on screen open
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te != null) {
            ClientPacketDistributor.sendToServer(
                    new TerminalReadyPacket(te.getComputerId(),
                            (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, 0));
        }
    }
    
    //? if >=26.1 {
    @Override
    public void extractRenderState(GuiGraphicsExtractor extractor, int mouseX, int mouseY, float partialTick) {
        // Render background darkening
        this.extractBackground(extractor, mouseX, mouseY, partialTick);

        // Render terminal
        renderTerminal(extractor);
        renderControllerToggle(new com.example.evanscomputermod.client.GuiGfx(extractor), mouseX, mouseY);

        // Don't call super.extractRenderState() to avoid rendering inventory slots
    }

    @Override
    public void extractBackground(GuiGraphicsExtractor extractor, int mouseX, int mouseY, float partialTick) {
        // Background is rendered in extractRenderState() method
    }
    //?} else {
    /*@Override
    public void render(net.minecraft.client.gui.GuiGraphics gfx, int mouseX, int mouseY, float partialTick) {
        // In 1.21.1 AbstractContainerScreen calls renderBg then our draw here.
        // We call the base render which paints background (via our renderBg) then slots.
        this.renderBackground(gfx, mouseX, mouseY, partialTick);
        renderTerminal(gfx);
        renderControllerToggle(new com.example.evanscomputermod.client.GuiGfx(gfx), mouseX, mouseY);
    }

    @Override
    protected void renderBg(net.minecraft.client.gui.GuiGraphics gfx, float partialTick, int mouseX, int mouseY) {
        // No vanilla background — the terminal draws its own in render().
    }

    @Override
    protected void renderLabels(net.minecraft.client.gui.GuiGraphics gfx, int mouseX, int mouseY) {
        // Don't render labels.
    }*/
    //?}

    @Override
    protected void containerTick() {
        super.containerTick();
        cursorBlinkTimer++;
        if (cursorBlinkTimer >= 10) {  // Blink every 10 game ticks (~0.5s)
            cursorBlinkTimer = 0;
            cursorVisible = !cursorVisible;
        }
    }
    
    //? if >=26.1 {
    /**
     * Renders the terminal display using the memory-mapped framebuffer (26.1 path).
     */
    private void renderTerminal(GuiGraphicsExtractor gfx) {
        int x = this.leftPos;
        int y = this.topPos;

        // Draw terminal background
        gfx.fill(x, y, x + screenWidth, y + screenHeight, backgroundColor());

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

        updateGraphicsTexture(display, displayMode);

        gfx.pose().pushMatrix();
        int textX = x + PADDING;
        int textY = y + PADDING;
        gfx.pose().translate(textX, textY);
        gfx.pose().scale(scale, scale);

        if (panelVisual() == PanelVisual.NONE) {
            // Headless: the computer runs on (SSH, programs), but there is nothing to see.
            renderNoPanel(new com.example.evanscomputermod.client.GuiGfx(gfx));
        } else {
            if (displayMode >= 1 && gfxTexture != null) {
                renderGraphicsQuad(gfx, AREA_W, AREA_H);
            }

            if (displayMode == 0 || displayMode == 2) {
                renderCellsScaled(gfx, te, display, displayMode == 2);
            }
        }

        gfx.pose().popMatrix();
    }
    //?} else {
    /*// Renders the terminal display using the memory-mapped framebuffer (1.21.1 path).
    private void renderTerminal(net.minecraft.client.gui.GuiGraphics gfx) {
        int x = this.leftPos;
        int y = this.topPos;

        gfx.fill(x, y, x + screenWidth, y + screenHeight, backgroundColor());

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

        updateGraphicsTexture(display, displayMode);

        gfx.pose().pushPose();
        int textX = x + PADDING;
        int textY = y + PADDING;
        gfx.pose().translate((float) textX, (float) textY, 0.0f);
        gfx.pose().scale(scale, scale, 1.0f);

        if (panelVisual() == PanelVisual.NONE) {
            // Headless: the computer runs on (SSH, programs), but there is nothing to see.
            renderNoPanel(new com.example.evanscomputermod.client.GuiGfx(gfx));
        } else {
            if (displayMode >= 1 && gfxTexture != null) {
                renderGraphicsQuad(gfx, AREA_W, AREA_H);
            }

            if (displayMode == 0 || displayMode == 2) {
                renderCellsScaled(gfx, te, display, displayMode == 2);
            }
        }

        gfx.pose().popPose();
    }*/
    //?}

    // ---------------------------------------------------------------- panel

    /** The Terminal's panel, from its block state (the server keeps it in step with the slot). */
    private PanelVisual panelVisual() {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null || !(te.getBlockState().getBlock() instanceof TerminalBlock)) return PanelVisual.COLOR_LCD;
        return te.getBlockState().getValue(TerminalBlock.PANEL);
    }

    /** Phosphor tone of a Tier 1 panel, or null for colour panels. */
    private com.example.evanscomputermod.computer.display.PanelTier.Tone phosphorTone() {
        PanelVisual v = panelVisual();
        return v.tier() == com.example.evanscomputermod.computer.display.PanelTier.PHOSPHOR ? v.tone() : null;
    }

    private int[] palette() {
        var tone = phosphorTone();
        if (tone == null) return PALETTE;
        int[] p = new int[16];
        for (int i = 0; i < 16; i++) p[i] = tone.map(PALETTE[i]);
        return p;
    }

    private int backgroundColor() {
        var tone = phosphorTone();
        return tone != null ? tone.darkArgb() : BACKGROUND_COLOR;
    }

    private int cursorColor() {
        var tone = phosphorTone();
        return tone != null ? tone.argb() : CURSOR_COLOR;
    }

    /**
     * Cells per base pixel: the text grid is stretched to fill the fixed
     * terminal area (2 for an 80x25 grid, 1 for 160x50).
     */
    private static float gridScale(com.example.evanscomputermod.computer.TerminalDisplay display) {
        int w = Math.max(1, display.getWidth()), h = Math.max(1, display.getHeight());
        return Math.min((float) AREA_W / (w * FONT_CELL_WIDTH), (float) AREA_H / (h * FONT_CELL_HEIGHT));
    }

    //? if >=26.1 {
    private void renderCellsScaled(GuiGraphicsExtractor gfx, TerminalBlockEntity te,
                                   com.example.evanscomputermod.computer.TerminalDisplay display, boolean overlay) {
        float gs = gridScale(display);
        gfx.pose().pushMatrix();
        gfx.pose().scale(gs, gs);
        renderCells(new com.example.evanscomputermod.client.GuiGfx(gfx), te, display, overlay);
        gfx.pose().popMatrix();
    }
    //?} else {
    /*private void renderCellsScaled(net.minecraft.client.gui.GuiGraphics gfx, TerminalBlockEntity te,
                                   com.example.evanscomputermod.computer.TerminalDisplay display, boolean overlay) {
        float gs = gridScale(display);
        gfx.pose().pushPose();
        gfx.pose().scale(gs, gs, 1.0f);
        renderCells(new com.example.evanscomputermod.client.GuiGfx(gfx), te, display, overlay);
        gfx.pose().popPose();
    }*/
    //?}

    /** "No display panel" in the middle of the dark screen. */
    private void renderNoPanel(com.example.evanscomputermod.client.GuiGfx g) {
        Component msg = Component.translatable("gui.evanscomputermod.no_panel");
        int w = this.font.width(msg);
        g.text(this.font, msg, (AREA_W - w) / 2, AREA_H / 2 - 4, 0xFF777777, false);
    }

    // Cell flags byte (IFramebufferDisplay): bit 0 bold, 1 underline, 2 blink, 3 inverse.
    private static final int FLAG_BOLD = 0x01;
    private static final int FLAG_UNDERLINE = 0x02;
    private static final int FLAG_BLINK = 0x04;
    private static final int FLAG_INVERSE = 0x08;

    /**
     * Draw the text cells, in base (unscaled) cell units, with their
     * attributes: colours, then the flags. Bold shows as the bright colour in
     * a bold glyph, underline as a line under the glyph, inverse swaps the
     * colours (an unset background becomes the terminal background), and
     * blinking text disappears in the cursor's off phase. In overlay mode
     * (text over graphics) cells without a background stay transparent.
     */
    private void renderCells(com.example.evanscomputermod.client.GuiGfx g, TerminalBlockEntity te,
                             com.example.evanscomputermod.computer.TerminalDisplay display, boolean isOverlay) {
        int termWidth = display.getWidth();
        int termHeight = display.getHeight();
        int baseCharWidth = FONT_CELL_WIDTH;
        int baseCharHeight = FONT_CELL_HEIGHT;
        int cx = te.getCursorX();
        int cy = te.getCursorY();
        boolean showCursor = cursorVisible && te.isCursorVisible();
        boolean blinkOn = cursorVisible;
        int[] pal = palette();

        for (int row = 0; row < termHeight; row++) {
            int rowY = row * baseCharHeight;
            int glyphY = rowY - GLYPH_Y_OFFSET;
            for (int col = 0; col < termWidth; col++) {
                byte ch = display.getCharAt(col, row);
                byte attr = display.getAttrAt(col, row);
                int flags = display.getFlagsAt(col, row);

                int fgIdx = attr & 0x0F;
                int bgIdx = (attr >> 4) & 0x0F;
                boolean bold = (flags & FLAG_BOLD) != 0;
                if (bold && fgIdx < 8) fgIdx += 8; // bold = the bright colour, as most terminals draw it
                int fg = pal[fgIdx];
                int bg = bgIdx != 0 ? pal[bgIdx] : 0;
                if ((flags & FLAG_INVERSE) != 0) {
                    int swap = fg;
                    fg = bgIdx != 0 ? pal[bgIdx] : backgroundColor();
                    bg = swap;
                }
                int cellX = col * baseCharWidth;
                boolean isCursor = showCursor && col == cx && row == cy;
                boolean hidden = (flags & FLAG_BLINK) != 0 && !blinkOn;

                if (bg != 0) {
                    g.fill(cellX, rowY, cellX + baseCharWidth, rowY + baseCharHeight, bg);
                }
                if (isCursor) {
                    g.fill(cellX, glyphY, cellX + baseCharWidth, glyphY + baseCharHeight, cursorColor());
                }
                if (hidden) continue;

                int charColor = isCursor ? pal[0] : fg;
                if (ch > 0x20 && ch < 0x7F) {
                    g.text(this.font, bold ? BOLD_CHAR_COMPONENTS[ch] : CHAR_COMPONENTS[ch], cellX, rowY, charColor, false);
                }
                if ((flags & FLAG_UNDERLINE) != 0) {
                    int lineY = glyphY + FONT_ASCENT;
                    g.fill(cellX, lineY, cellX + baseCharWidth, lineY + 1, charColor);
                }
            }
        }
        if (hasSelection || isSelecting) {
            int[] sel = normalizedSelection();
            for (int row = sel[1]; row <= sel[3] && row < termHeight; row++) {
                int from = row == sel[1] ? sel[0] : 0;
                int to = row == sel[3] ? sel[2] + 1 : termWidth;
                if (to > from) {
                    g.fill(from * baseCharWidth, row * baseCharHeight, to * baseCharWidth, (row + 1) * baseCharHeight, SELECTION_COLOR);
                }
            }
        }
    }

    /** The selection as {startX, startY, endX, endY}, start before end (end inclusive). */
    private int[] normalizedSelection() {
        boolean forward = selectionStartY < selectionEndY
                || (selectionStartY == selectionEndY && selectionStartX <= selectionEndX);
        return forward
                ? new int[]{selectionStartX, selectionStartY, selectionEndX, selectionEndY}
                : new int[]{selectionEndX, selectionEndY, selectionStartX, selectionStartY};
    }

    /**
     * Create, resize, or destroy the graphics texture as needed.
     */
    private void updateGraphicsTexture(com.example.evanscomputermod.computer.TerminalDisplay display, int displayMode) {
        if (displayMode >= 1) {
            int gfxW = display.getGfxWidth();
            int gfxH = display.getGfxHeight();
            if (gfxW > 0 && gfxH > 0) {
                boolean fresh = false;
                if (gfxTexture == null) {
                    gfxTexture = new TerminalGraphicsTexture(gfxW, gfxH);
                    fresh = true;
                } else if (gfxTexture.getWidth() != gfxW || gfxTexture.getHeight() != gfxH) {
                    gfxTexture.resize(gfxW, gfxH);
                    fresh = true;
                }
                // Tier 1: two phosphor tones; swapping the panel re-expands the frame.
                fresh |= gfxTexture.setTone(phosphorTone());
                // Only upload to GPU when pixel data or palette actually changed,
                // and then only the tiles that changed (all of a fresh texture).
                int pixDirty = display.getPixelDirtyCounter();
                int palDirty = display.getPaletteDirtyCounter();
                if (fresh || pixDirty != lastSeenPixelDirty || palDirty != lastSeenPaletteDirty) {
                    gfxTexture.update(display, fresh);
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
     * The largest rectangle with the framebuffer's aspect ratio that fits in an
     * {@code areaW x areaH} area, centred: {@code [x, y, w, h]}. Pixels stay
     * square, so a 3:2 game is letterboxed rather than stretched to the
     * terminal's 16:10.
     */
    static int[] fitRect(int areaW, int areaH, int gfxW, int gfxH) {
        if (gfxW <= 0 || gfxH <= 0) return new int[]{0, 0, areaW, areaH};
        int w = areaW;
        int h = (int) ((long) areaW * gfxH / gfxW);
        if (h > areaH) {
            h = areaH;
            w = (int) ((long) areaH * gfxW / gfxH);
        }
        return new int[]{(areaW - w) / 2, (areaH - h) / 2, w, h};
    }

    //? if >=26.1 {
    /**
     * Render the graphics framebuffer as a textured quad fitted into the terminal area.
     */
    private void renderGraphicsQuad(GuiGraphicsExtractor gfx, int termPixelW, int termPixelH) {
        int gfxW = gfxTexture.getWidth();
        int gfxH = gfxTexture.getHeight();
        int[] r = fitRect(termPixelW, termPixelH, gfxW, gfxH);
        gfx.blit(RenderPipelines.GUI_TEXTURED,
                gfxTexture.getTextureId(),
                r[0], r[1],
                0.0f, 0.0f,
                r[2],
                r[3],
                gfxW, gfxH,
                gfxW, gfxH
        );
    }
    //?} else {
    /*private void renderGraphicsQuad(net.minecraft.client.gui.GuiGraphics gfx, int termPixelW, int termPixelH) {
        int gfxW = gfxTexture.getWidth();
        int gfxH = gfxTexture.getHeight();
        // blit(ResourceLocation, x, y, width, height, uOffset, vOffset, uWidth, vHeight, textureWidth, textureHeight)
        // Draw quad at (termPixelW x termPixelH) sampling full (gfxW x gfxH) texture.
        int[] r = fitRect(termPixelW, termPixelH, gfxW, gfxH);
        gfx.blit(gfxTexture.getTextureId(), r[0], r[1], r[2], r[3], 0.0f, 0.0f, gfxW, gfxH, gfxW, gfxH);
    }*/
    //?}
    
    //? if >=26.1 {
    @Override
    public boolean keyPressed(KeyEvent event) {
        int keyCode = event.key();
        int scanCode = event.scancode();
        int modifiers = event.modifiers();
        return handleKeyPressed(keyCode, scanCode, modifiers);
    }

    @Override
    public boolean keyReleased(KeyEvent event) {
        sendRawKey(event.key(), event.modifiers(), false);
        return super.keyReleased(event);
    }
    //?} else {
    /*@Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        return handleKeyPressed(keyCode, scanCode, modifiers);
    }

    @Override
    public boolean keyReleased(int keyCode, int scanCode, int modifiers) {
        sendRawKey(keyCode, modifiers, false);
        return super.keyReleased(keyCode, scanCode, modifiers);
    }*/
    //?}

    /** GLFW keys currently held, to mark repeats (MC reports repeats as presses). */
    private final java.util.Set<Integer> heldKeys = new java.util.HashSet<>();

    /**
     * The raw key for the GUI (docs/gui/UI_PROTOCOL.md §7): a USB HID usage with the
     * modifier bits. Sent alongside the console's key strings; the server uses
     * whichever matches what the display shows.
     */
    private void sendRawKey(int keyCode, int modifiers, boolean down) {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return;
        int hid = com.example.evanscomputermod.computer.ui.HidKeys.fromGlfw(keyCode);
        byte action;
        if (down) {
            action = heldKeys.add(keyCode) ? com.example.evanscomputermod.network.KeyInputPacket.DOWN
                    : com.example.evanscomputermod.network.KeyInputPacket.REPEAT;
        } else {
            if (!heldKeys.remove(keyCode)) return;
            action = com.example.evanscomputermod.network.KeyInputPacket.UP;
        }
        if (hid == 0) return;
        com.example.evanscomputermod.computer.ui.ClientUiScenes.key(te.getComputerId(),
                com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, hid,
                action != com.example.evanscomputermod.network.KeyInputPacket.UP, System.nanoTime() / 1_000_000L);
        ClientPacketDistributor.sendToServer(com.example.evanscomputermod.network.KeyInputPacket.key(
                te.getComputerId(), (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, action, hid,
                modifiers & com.example.evanscomputermod.computer.ui.HidKeys.MODS_MASK));
    }

    private void sendText(String text) {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return;
        com.example.evanscomputermod.computer.ui.ClientUiScenes.typed(te.getComputerId(),
                com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, text, System.nanoTime() / 1_000_000L);
        ClientPacketDistributor.sendToServer(com.example.evanscomputermod.network.KeyInputPacket.text(
                te.getComputerId(), (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, text));
    }

    /**
     * The one place Esc is decided, for every program and both display modes:
     * plain Esc closes the screen, as on every Minecraft screen; Shift+Esc
     * sends a plain Esc to the computer (programs that use Esc, vim among
     * them, get it this way); Esc with Ctrl or Alt goes to the computer as a
     * chord (Ctrl+Esc opens the desktop's launcher).
     */
    private void handleEscape(int modifiers) {
        boolean shift = (modifiers & 1) != 0, ctrl = (modifiers & 2) != 0, alt = (modifiers & 4) != 0;
        if (!shift && !ctrl && !alt) {
            this.onClose();
            return;
        }
        sendRawKey(256, shift ? modifiers & ~1 : modifiers, true);
        altTyped = false;
        sendInput("\u001b");
    }

    private boolean handleKeyPressed(int keyCode, int scanCode, int modifiers) {
        boolean ctrl = (modifiers & 2) != 0;   // GLFW_MOD_CONTROL
        boolean shift = (modifiers & 1) != 0;  // GLFW_MOD_SHIFT
        boolean alt = (modifiers & 4) != 0;    // GLFW_MOD_ALT
        if ((ctrl && keyCode == 86) || (shift && keyCode == 260)) {
            // A paste key: the GUI's paste reads the real clipboard, sent first.
            pushClipboard();
        }
        if (keyCode == 256) {
            handleEscape(modifiers);
            return true;
        }
        sendRawKey(keyCode, modifiers, true);
        altTyped = false;

        if (isControllerKey(com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM.getOrCreate(keyCode))) {
            return true;
        }

        // Copy: Ctrl+C (with a selection), Ctrl+Shift+C, Ctrl+Insert.
        if ((ctrl && keyCode == 67 && (hasSelection || shift)) || (ctrl && keyCode == 260)) {
            copySelection();
            return true;
        }
        // Paste: Ctrl+V, Ctrl+Shift+V, Shift+Insert.
        if ((ctrl && keyCode == 86) || (shift && keyCode == 260)) {
            pasteClipboard();
            return true;
        }

        // Any other key press clears the selection (not the modifiers themselves).
        if (keyCode < 340 || keyCode > 347) {
            clearSelection();
        }

        String seq = keySequence(keyCode, ctrl, shift, alt);
        if (seq != null) {
            altTyped = alt && keyCode >= 65 && keyCode <= 90;
            sendInput(seq);
            return true;
        }
        // Let other keys go through to charTyped
        return false;
    }

    /** Set by an Alt+letter press (sent as ESC + letter), so its charTyped isn't sent again. */
    private boolean altTyped;

    /**
     * The bytes a key sends, as xterm does: control characters for
     * Ctrl+letters, ESC-prefixed letters for Alt, CSI sequences with the
     * modifier parameter (2 Shift, 3 Alt, 5 Ctrl, ...) for the cursor and
     * editing keys and F1-F12. Null: a printable key (sent by charTyped).
     */
    static String keySequence(int keyCode, boolean ctrl, boolean shift, boolean alt) {
        int mod = 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0);
        if (ctrl && !alt && keyCode >= 65 && keyCode <= 90) {
            return String.valueOf((char) (keyCode - 64));
        }
        if (alt && !ctrl && keyCode >= 65 && keyCode <= 90) {
            char c = (char) (shift ? keyCode : keyCode + 32);
            return "\u001b" + c;
        }
        String csi = switch (keyCode) {
            case 265 -> "A"; // Up
            case 264 -> "B"; // Down
            case 262 -> "C"; // Right
            case 263 -> "D"; // Left
            case 268 -> "H"; // Home
            case 269 -> "F"; // End
            default -> null;
        };
        if (csi != null) return mod == 1 ? "\u001b[" + csi : "\u001b[1;" + mod + csi;
        String tilde = switch (keyCode) {
            case 260 -> "2";  // Insert
            case 261 -> "3";  // Delete
            case 266 -> "5";  // Page Up
            case 267 -> "6";  // Page Down
            case 294 -> "15"; // F5
            case 295 -> "17";
            case 296 -> "18";
            case 297 -> "19";
            case 298 -> "20";
            case 299 -> "21";
            case 300 -> "23";
            case 301 -> "24"; // F12
            default -> null;
        };
        if (tilde != null) return mod == 1 ? "\u001b[" + tilde + "~" : "\u001b[" + tilde + ";" + mod + "~";
        if (keyCode >= 290 && keyCode <= 293) { // F1-F4
            char c = (char) ('P' + keyCode - 290);
            return mod == 1 ? "\u001bO" + c : "\u001b[1;" + mod + c;
        }
        return switch (keyCode) {
            case 257, 335 -> "\n";                       // Enter
            case 259 -> alt ? "\u001b\b" : "\b";       // Backspace (Alt: delete a word)
            case 258 -> shift ? "\u001b[Z" : "\t";      // Tab / Shift+Tab
            default -> null;
        };
    }

    /** Send the player's clipboard to the computer (the GUI pastes it). */
    private void pushClipboard() {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return;
        String clip = Minecraft.getInstance().keyboardHandler.getClipboard();
        if (clip == null) return;
        ClientPacketDistributor.sendToServer(new com.example.evanscomputermod.network.ClipboardPackets.Push(
                te.getComputerId(), (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, clip));
    }

    private void copySelection() {
        if (!hasSelection) return;
        String selectedText = getSelectedText();
        if (!selectedText.isEmpty()) {
            Minecraft.getInstance().keyboardHandler.setClipboard(selectedText);
        }
        clearSelection();
    }

    /** Paste as typed text, marked as a paste (bracketed paste) so a Tab or key in it is only text. */
    private void pasteClipboard() {
        String clipboard = Minecraft.getInstance().keyboardHandler.getClipboard();
        if (clipboard != null && !clipboard.isEmpty()) {
            sendInput(pasteSequence(clipboard));
        }
    }

    static String pasteSequence(String text) {
        return "\u001b[200~" + text.replace("\r\n", "\n").replace('\r', '\n').replace("\u001b", "") + "\u001b[201~";
    }

    //? if >=26.1 {
    @Override
    public boolean charTyped(CharacterEvent event) {
        return handleCharTyped(event.codepoint()) || super.charTyped(event);
    }
    //?} else {
    /*@Override
    public boolean charTyped(char codePoint, int modifiers) {
        return handleCharTyped(codePoint) || super.charTyped(codePoint, modifiers);
    }*/
    //?}

    private boolean handleCharTyped(int cp) {
        char codePoint = (char) cp;
        if (isControllerChar(codePoint)) return true;
        if (altTyped) return true;
        if (cp >= 32 && cp != 127) sendText(Character.toString(cp));
        if (cp >= 32 && cp != 127) {
            // UTF-8 text is fine: the terminal shows the nearest character it can draw.
            sendInput(Character.toString(cp));
            return true;
        }
        return false;
    }

    // ------------------------------------------------------------ ComputerInputScreen
    // ComputerScreenInput hands these the input before other mods' screen listeners (JEI).

    @Override
    public boolean ownsPointer(double x, double y) {
        if (x >= leftPos && x < leftPos + screenWidth && y >= topPos && y < topPos + screenHeight) return true;
        if (pairedController == null) return false;
        int[] r = controllerToggleRect();
        return x >= r[0] && x < r[2] && y >= r[1] && y < r[3];
    }

    @Override
    public boolean computerMousePressed(double x, double y, int button) {
        return handleMouseClicked(x, y, button, () -> false);
    }

    @Override
    public boolean computerMouseReleased(double x, double y, int button) {
        return handleMouseReleased(x, y, button, () -> false);
    }

    @Override
    public boolean computerMouseDragged(double x, double y, int button, double dx, double dy) {
        return handleMouseDragged(x, y, button, () -> false);
    }

    @Override
    public boolean computerMouseScrolled(double x, double y, double scrollX, double scrollY) {
        return mouseScrolled(x, y, scrollX, scrollY);
    }

    @Override
    public boolean computerKeyPressed(int key, int scan, int mods) {
        return handleKeyPressed(key, scan, mods);
    }

    @Override
    public boolean computerKeyReleased(int key, int scan, int mods) {
        sendRawKey(key, mods, false);
        return true;
    }

    @Override
    public boolean computerCharTyped(int codePoint) {
        return handleCharTyped(codePoint);
    }

    @Override
    public boolean swallowsKey(int key) {
        return swallowsControllerKey(key);
    }

    @Override
    public boolean swallowsChar(int codePoint) {
        return swallowsControllerChar((char) codePoint);
    }

    @Override
    public boolean mouseScrolled(double mouseX, double mouseY, double scrollX, double scrollY) {
        if (isMouseCaptureEligible()) {
            int[] fb = mouseToGfxPixel(mouseX, mouseY);
            if (fb != null) {
                byte dir = (byte) (scrollY > 0 ? 1 : -1);
                sendMouseEvent(MouseInputPacket.KIND_SCROLL, fb[0], fb[1], (byte) 0, dir);
                return true;
            }
        }
        // The kernel scrolls its scrollback (Shift+Up/Down), or turns them into
        // arrows for a full-screen program.
        if (scrollY != 0) {
            clearSelection();
            String one = scrollY > 0 ? "\u001b[1;2A" : "\u001b[1;2B";
            sendScroll(one.repeat(3));
        }
        return true;
    }

    /**
     * True when the screen is open AND the display is in graphics mode —
     * the two conditions that authorize forwarding mouse events. The
     * screen-open half is implicit because this method is only reachable
     * on input handlers.
     */
    private boolean isMouseCaptureEligible() {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return false;
        return te.getDisplay().getDisplayMode() >= 1;
    }

    /**
     * Map screen pixel coordinates to framebuffer pixel coordinates.
     * Returns {@code null} if the cursor is outside the graphics quad —
     * the same letterboxed rectangle {@link #renderGraphicsQuad} paints into.
     * Uses the same {@code textX/textY} + {@code terminalPixelWidth/Height}
     * math as {@link #mouseToCharPos}, scaled through the current font
     * scale so sub-scale movement doesn't round to the wrong pixel.
     */
    private int[] mouseToGfxPixel(double mouseX, double mouseY) {
        return mouseToGfxPixel(mouseX, mouseY, false);
    }

    /**
     * As {@link #mouseToGfxPixel(double, double)}; with {@code clamp} a point
     * outside the quad maps to its nearest edge pixel instead of null (a drag
     * that leaves the picture, e.g. a scrollbar thumb pulled past the end,
     * keeps moving to the edge).
     */
    private int[] mouseToGfxPixel(double mouseX, double mouseY, boolean clamp) {
        int textX = this.leftPos + PADDING;
        int textY = this.topPos + PADDING;
        if (!clamp && (mouseX < textX || mouseX >= textX + terminalPixelWidth ||
            mouseY < textY || mouseY >= textY + terminalPixelHeight)) {
            return null;
        }
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return null;
        com.example.evanscomputermod.computer.TerminalDisplay display = te.getDisplay();
        int gfxW = display.getGfxWidth();
        int gfxH = display.getGfxHeight();
        if (gfxW <= 0 || gfxH <= 0) return null;

        int[] r = fitRect(terminalPixelWidth, terminalPixelHeight, gfxW, gfxH);
        double localX = mouseX - textX - r[0];
        double localY = mouseY - textY - r[1];
        if (!clamp && (localX < 0 || localY < 0 || localX >= r[2] || localY >= r[3])) return null;
        int fbX = (int) (localX / r[2] * gfxW);
        int fbY = (int) (localY / r[3] * gfxH);
        if (fbX < 0) fbX = 0; else if (fbX >= gfxW) fbX = gfxW - 1;
        if (fbY < 0) fbY = 0; else if (fbY >= gfxH) fbY = gfxH - 1;
        return new int[]{fbX, fbY};
    }

    @Override
    public void mouseMoved(double mouseX, double mouseY) {
        if (isMouseCaptureEligible()) {
            // While a button is held the drag follows the pointer past the picture's edge.
            int[] fb = mouseToGfxPixel(mouseX, mouseY, lastButtonsHeld != 0);
            if (fb != null && (fb[0] != lastSentFbX || fb[1] != lastSentFbY)) {
                sendMouseEvent(MouseInputPacket.KIND_MOVE, fb[0], fb[1], (byte) 0, (byte) 0);
                lastSentFbX = fb[0];
                lastSentFbY = fb[1];
            }
        }
        super.mouseMoved(mouseX, mouseY);
    }

    /** Map a GLFW mouse button (0=L, 1=R, 2=M) to our `buttons` bitmask bit. */
    private static byte buttonMaskBit(int button) {
        return switch (button) {
            case 0 -> (byte) 0x01;
            case 1 -> (byte) 0x02;
            case 2 -> (byte) 0x04;
            default -> 0;
        };
    }

    /** Send a mouse event packet to the server. */
    private void sendMouseEvent(byte kind, int fbX, int fbY, byte buttonCode, byte scrollDir) {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return;
        // A GUI this client draws shows hover, pressed, scrolls and drags at once (Part F); a wheel
        // over a scroll container is scrolled here and reported instead of sent.
        var scroll = com.example.evanscomputermod.computer.ui.ClientUiScenes.pointer(te.getComputerId(),
                com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, kind, fbX, fbY, buttonCode, scrollDir,
                System.nanoTime() / 1_000_000L);
        if (scroll != null) {
            ClientPacketDistributor.sendToServer(new com.example.evanscomputermod.network.UiScrollPacket(te.getComputerId(),
                    (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN, scroll.surface(), scroll.node(),
                    scroll.x(), scroll.y()));
            return;
        }
        MouseInputPacket pkt = new MouseInputPacket(
                te.getComputerId(),
                (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN,
                kind,
                (short) fbX,
                (short) fbY,
                lastButtonsHeld,
                buttonCode,
                scrollDir
        );
        ClientPacketDistributor.sendToServer(pkt);
    }
    
    /**
     * Where the graphics plane is drawn, in GUI coordinates: {@code {x0, y0, x1, y1}},
     * or null in text mode. For render checks (mapping framebuffer pixels to the screen).
     */
    public float[] gfxRect() {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return null;
        int gfxW = te.getDisplay().getGfxWidth(), gfxH = te.getDisplay().getGfxHeight();
        if (te.getDisplay().getDisplayMode() < 1 || gfxW <= 0 || gfxH <= 0) return null;
        int[] r = fitRect(terminalPixelWidth, terminalPixelHeight, gfxW, gfxH);
        float x = leftPos + PADDING + r[0], y = topPos + PADDING + r[1];
        return new float[]{x, y, x + r[2], y + r[3]};
    }

    /**
     * Where cell ({@code col}, {@code row})'s background is drawn, in GUI
     * coordinates: {@code {x0, y0, x1, y1}}. For render checks.
     */
    public float[] cellRect(int col, int row) {
        TerminalBlockEntity te = menu.getBlockEntity();
        float s = scale * (te != null ? gridScale(te.getDisplay()) : 1f);
        float x = leftPos + PADDING + col * FONT_CELL_WIDTH * s;
        float y = topPos + PADDING + row * FONT_CELL_HEIGHT * s;
        return new float[]{x, y, x + FONT_CELL_WIDTH * s, y + FONT_CELL_HEIGHT * s};
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
        
        TerminalBlockEntity te = menu.getBlockEntity();
        com.example.evanscomputermod.computer.TerminalDisplay display = te.getDisplay();
        // Cells are FONT_CELL_* base pixels, stretched by the grid scale and the GUI scale.
        float s = scale * gridScale(display);
        int charY = (int) ((mouseY - textY) / (FONT_CELL_HEIGHT * s));
        charY = Math.max(0, Math.min(display.getHeight() - 1, charY));
        int charX = (int) ((mouseX - textX) / (FONT_CELL_WIDTH * s));
        charX = Math.max(0, Math.min(display.getWidth() - 1, charX));

        return new int[]{charX, charY};
    }
    
    //? if >=26.1 {
    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean focused) {
        return handleMouseClicked(event.x(), event.y(), event.button(), () -> super.mouseClicked(event, focused));
    }

    @Override
    public boolean mouseDragged(MouseButtonEvent event, double dragX, double dragY) {
        return handleMouseDragged(event.x(), event.y(), event.button(), () -> super.mouseDragged(event, dragX, dragY));
    }

    @Override
    public boolean mouseReleased(MouseButtonEvent event) {
        return handleMouseReleased(event.x(), event.y(), event.button(), () -> super.mouseReleased(event));
    }
    //?} else {
    /*@Override
    public boolean mouseClicked(double mouseX, double mouseY, int button) {
        return handleMouseClicked(mouseX, mouseY, button, () -> super.mouseClicked(mouseX, mouseY, button));
    }

    @Override
    public boolean mouseDragged(double mouseX, double mouseY, int button, double dragX, double dragY) {
        return handleMouseDragged(mouseX, mouseY, button, () -> super.mouseDragged(mouseX, mouseY, button, dragX, dragY));
    }

    @Override
    public boolean mouseReleased(double mouseX, double mouseY, int button) {
        return handleMouseReleased(mouseX, mouseY, button, () -> super.mouseReleased(mouseX, mouseY, button));
    }*/
    //?}

    private boolean handleMouseClicked(double mouseX, double mouseY, int button, java.util.function.BooleanSupplier superCall) {
        if (clickControllerToggle(mouseX, mouseY, button)) return true;
        // In gfx mode, clicks inside the quad forward to the guest instead of starting text selection.
        if (isMouseCaptureEligible()) {
            int[] fb = mouseToGfxPixel(mouseX, mouseY);
            if (fb != null) {
                byte bit = buttonMaskBit(button);
                if (bit != 0) {
                    lastButtonsHeld |= bit;
                    sendMouseEvent(MouseInputPacket.KIND_DOWN, fb[0], fb[1], (byte) button, (byte) 0);
                    return true;
                }
            }
        }

        if (button == 1 && mouseToCharPos(mouseX, mouseY) != null) {
            // Right click: copy the selection, or paste when there is none.
            if (hasSelection) copySelection();
            else pasteClipboard();
            return true;
        }
        if (button == 0) {
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                long now = System.currentTimeMillis();
                boolean again = now - lastClickMs < 400 && charPos[0] == lastClickX && charPos[1] == lastClickY;
                clickCount = again ? clickCount % 3 + 1 : 1;
                lastClickMs = now;
                lastClickX = charPos[0];
                lastClickY = charPos[1];
                if (clickCount >= 2) {
                    selectWordOrLine(charPos[0], charPos[1], clickCount == 3);
                    return true;
                }
                isSelecting = true;
                hasSelection = false;
                selectionStartX = charPos[0];
                selectionStartY = charPos[1];
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                return true;
            }
        }
        return superCall.getAsBoolean();
    }

    private boolean handleMouseDragged(double mouseX, double mouseY, int button, java.util.function.BooleanSupplier superCall) {
        if (button == 0 && isSelecting) {
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                hasSelection = (selectionStartX != selectionEndX || selectionStartY != selectionEndY);
                return true;
            }
        }
        return superCall.getAsBoolean();
    }

    private boolean handleMouseReleased(double mouseX, double mouseY, int button, java.util.function.BooleanSupplier superCall) {
        byte bit = buttonMaskBit(button);
        if (bit != 0 && (lastButtonsHeld & bit) != 0) {
            lastButtonsHeld &= (byte) ~bit;
            if (isMouseCaptureEligible()) {
                int[] fb = mouseToGfxPixel(mouseX, mouseY);
                if (fb == null) {
                    fb = new int[]{Math.max(0, lastSentFbX), Math.max(0, lastSentFbY)};
                }
                sendMouseEvent(MouseInputPacket.KIND_UP, fb[0], fb[1], (byte) button, (byte) 0);
                return true;
            }
        }

        if (button == 0 && isSelecting) {
            isSelecting = false;
            int[] charPos = mouseToCharPos(mouseX, mouseY);
            if (charPos != null) {
                selectionEndX = charPos[0];
                selectionEndY = charPos[1];
                boolean wasClick = (selectionStartX == selectionEndX && selectionStartY == selectionEndY);
                hasSelection = !wasClick;
            }
            return true;
        }
        return superCall.getAsBoolean();
    }
    
    private long lastClickMs;
    private int lastClickX = -1, lastClickY = -1, clickCount;

    /** Double click: the word under the pointer; triple click: the whole row. */
    private void selectWordOrLine(int col, int row, boolean line) {
        TerminalBlockEntity te = menu.getBlockEntity();
        if (te == null) return;
        com.example.evanscomputermod.computer.TerminalDisplay display = te.getDisplay();
        int w = display.getWidth();
        int a = 0, b = w - 1;
        if (!line) {
            if (!isWordChar(display.getCharAt(col, row))) return;
            a = col;
            b = col;
            while (a > 0 && isWordChar(display.getCharAt(a - 1, row))) a--;
            while (b < w - 1 && isWordChar(display.getCharAt(b + 1, row))) b++;
        }
        isSelecting = false;
        hasSelection = true;
        selectionStartX = a;
        selectionStartY = row;
        selectionEndX = b;
        selectionEndY = row;
    }

    /** Characters a double click takes as part of a word: letters, digits and path/address punctuation. */
    private static boolean isWordChar(byte ch) {
        return (ch >= '0' && ch <= '9') || (ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z')
                || ch == '_' || ch == '-' || ch == '.' || ch == '/' || ch == ':' || ch == '~' || ch == '@';
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
        sendScroll(input);
    }

    /** Send keys without leaving the scrollback (the wheel). */
    private void sendScroll(String input) {

        // Send to server via packet
        ClientPacketDistributor.sendToServer(new TerminalInputPacket(
                menu.getBlockEntity().getComputerId(),
                (byte) com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN,
                input
        ));
        
        // Don't update locally - let the server/WASM handle all input and sync back
    }
    
    @Override
    public void onClose() {
        if (controllerOn()) {
            com.example.evanscomputermod.controller.client.ControllerClient.setGuiController(null);
        }
        if (gfxTexture != null) {
            gfxTexture.close();
            gfxTexture = null;
        }
        super.onClose();
    }

    /** Esc closes the screen; Shift+Esc sends Esc to the computer (see handleKeyPressed). */
    @Override
    public boolean shouldCloseOnEsc() {
        return false;
    }

    @Override
    public boolean isPauseScreen() {
        return false;  // Don't pause the game when terminal is open
    }
}
