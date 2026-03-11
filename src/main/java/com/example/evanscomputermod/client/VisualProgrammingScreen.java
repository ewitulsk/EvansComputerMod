package com.example.evanscomputermod.client;

import com.example.evanscomputermod.client.VisualBlockRegistry.BlockDef;
import com.example.evanscomputermod.client.VisualBlockRegistry.Category;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;

import java.util.ArrayList;
import java.util.List;

/**
 * Client-side visual programming screen.
 * Provides a zoomable/pannable canvas with draggable function blocks
 * and an overlay palette sidebar.
 */
public class VisualProgrammingScreen extends Screen {

    // Layout constants
    private static final int PALETTE_WIDTH = 180;
    private static final int PALETTE_PADDING = 8;
    private static final int BLOCK_HEIGHT = 28;
    private static final int BLOCK_PADDING = 4;
    private static final int CATEGORY_HEADER_HEIGHT = 20;

    // Colors
    private static final int BACKGROUND_COLOR = 0xFF1E1E2E;
    private static final int PALETTE_BG_COLOR = 0xEE2A2A3E;
    private static final int PALETTE_BORDER_COLOR = 0xFF444466;
    private static final int CANVAS_GRID_COLOR = 0xFF262638;
    private static final int BLOCK_BORDER_COLOR = 0xFF000000;
    private static final int BLOCK_TEXT_COLOR = 0xFFFFFFFF;
    private static final int CATEGORY_TEXT_COLOR = 0xFFCCCCCC;
    private static final int BLOCK_SHADOW_COLOR = 0x40000000;
    private static final int TOGGLE_BTN_WIDTH = 20;
    private static final int TOGGLE_BTN_HEIGHT = 20;
    private static final int TOGGLE_BTN_COLOR = 0xFF3A3A52;
    private static final int TOGGLE_BTN_HOVER_COLOR = 0xFF4A4A66;

    // Zoom limits
    private static final float MIN_ZOOM = 0.25f;
    private static final float MAX_ZOOM = 3.0f;

    private final BlockPos terminalPos;

    // Canvas state
    private final List<PlacedBlock> placedBlocks = new ArrayList<>();
    private float canvasOffsetX = 0;
    private float canvasOffsetY = 0;
    private float canvasZoom = 1.0f;

    // Interaction state
    private PlacedBlock draggingBlock = null;
    private float dragOffsetX, dragOffsetY;
    private boolean isPanning = false;
    private double panStartX, panStartY;
    private float panStartOffsetX, panStartOffsetY;

    // Palette ghost — block picked from palette, follows cursor until placed
    private BlockDef ghostBlock = null;
    private int ghostColor = 0;

    // Palette visibility
    private boolean paletteVisible = true;

    // Palette scroll
    private int paletteScrollOffset = 0;

    public VisualProgrammingScreen(BlockPos terminalPos) {
        super(Component.translatable("screen.evanscomputermod.visual_editor"));
        this.terminalPos = terminalPos;
    }

    @Override
    protected void init() {
        super.init();
    }

    // --- Coordinate conversion ---

    /** Convert screen X to canvas X. */
    private float screenToCanvasX(double screenX) {
        return (float) ((screenX - canvasOffsetX) / canvasZoom);
    }

    /** Convert screen Y to canvas Y. */
    private float screenToCanvasY(double screenY) {
        return (float) ((screenY - canvasOffsetY) / canvasZoom);
    }

    /** Convert canvas X to screen X. */
    private float canvasToScreenX(float canvasX) {
        return canvasX * canvasZoom + canvasOffsetX;
    }

    /** Convert canvas Y to screen Y. */
    private float canvasToScreenY(float canvasY) {
        return canvasY * canvasZoom + canvasOffsetY;
    }

    // --- Rendering ---

    @Override
    public void render(GuiGraphics gfx, int mouseX, int mouseY, float partialTick) {
        // Background
        gfx.fill(0, 0, this.width, this.height, BACKGROUND_COLOR);

        // Canvas grid (full screen)
        renderCanvasGrid(gfx);

        // Placed blocks (in canvas space, zoomed)
        for (PlacedBlock block : placedBlocks) {
            int sx = (int) canvasToScreenX(block.x);
            int sy = (int) canvasToScreenY(block.y);
            renderBlock(gfx, block.label, block.color, sx, sy, canvasZoom, block == draggingBlock);
        }

        // Palette overlay (on top of canvas)
        if (paletteVisible) {
            renderPalette(gfx, mouseX, mouseY);
        }

        // Toggle button
        renderToggleButton(gfx, mouseX, mouseY);

        // Ghost block following cursor (at 1x scale)
        if (ghostBlock != null) {
            renderBlock(gfx, ghostBlock.label(), ghostColor,
                    mouseX - 50, mouseY - BLOCK_HEIGHT / 2, 1.0f, true);
        }

        // Zoom indicator
        String zoomText = String.format("%.0f%%", canvasZoom * 100);
        gfx.drawString(this.font, zoomText, this.width - this.font.width(zoomText) - 6, 6, 0xFF888888);
    }

    private void renderCanvasGrid(GuiGraphics gfx) {
        float gridSize = 32 * canvasZoom;
        if (gridSize < 8) return; // Don't render grid when zoomed out too far

        float ox = canvasOffsetX % gridSize;
        float oy = canvasOffsetY % gridSize;

        for (float x = ox; x < this.width; x += gridSize) {
            gfx.fill((int) x, 0, (int) x + 1, this.height, CANVAS_GRID_COLOR);
        }
        for (float y = oy; y < this.height; y += gridSize) {
            gfx.fill(0, (int) y, this.width, (int) y + 1, CANVAS_GRID_COLOR);
        }
    }

    private void renderToggleButton(GuiGraphics gfx, int mouseX, int mouseY) {
        int btnX = paletteVisible ? PALETTE_WIDTH : 0;
        int btnY = 4;
        boolean hovered = mouseX >= btnX && mouseX <= btnX + TOGGLE_BTN_WIDTH
                && mouseY >= btnY && mouseY <= btnY + TOGGLE_BTN_HEIGHT;
        int bgColor = hovered ? TOGGLE_BTN_HOVER_COLOR : TOGGLE_BTN_COLOR;

        gfx.fill(btnX, btnY, btnX + TOGGLE_BTN_WIDTH, btnY + TOGGLE_BTN_HEIGHT, bgColor);
        gfx.fill(btnX, btnY, btnX + TOGGLE_BTN_WIDTH, btnY + 1, PALETTE_BORDER_COLOR);
        gfx.fill(btnX, btnY + TOGGLE_BTN_HEIGHT - 1, btnX + TOGGLE_BTN_WIDTH, btnY + TOGGLE_BTN_HEIGHT, PALETTE_BORDER_COLOR);
        gfx.fill(btnX, btnY, btnX + 1, btnY + TOGGLE_BTN_HEIGHT, PALETTE_BORDER_COLOR);
        gfx.fill(btnX + TOGGLE_BTN_WIDTH - 1, btnY, btnX + TOGGLE_BTN_WIDTH, btnY + TOGGLE_BTN_HEIGHT, PALETTE_BORDER_COLOR);

        String arrow = paletteVisible ? "<<" : ">>";
        int textX = btnX + (TOGGLE_BTN_WIDTH - this.font.width(arrow)) / 2;
        int textY = btnY + (TOGGLE_BTN_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, arrow, textX, textY, BLOCK_TEXT_COLOR);
    }

    private void renderPalette(GuiGraphics gfx, int mouseX, int mouseY) {
        // Semi-transparent palette background (overlay)
        gfx.fill(0, 0, PALETTE_WIDTH, this.height, PALETTE_BG_COLOR);
        gfx.fill(PALETTE_WIDTH - 1, 0, PALETTE_WIDTH, this.height, PALETTE_BORDER_COLOR);

        // Title
        gfx.drawString(this.font, "Function Blocks", PALETTE_PADDING, PALETTE_PADDING, BLOCK_TEXT_COLOR);

        int y = PALETTE_PADDING + 14 - paletteScrollOffset;

        for (Category category : VisualBlockRegistry.getCategories()) {
            // Category header
            y += 6;
            if (y + CATEGORY_HEADER_HEIGHT > 0 && y < this.height) {
                gfx.drawString(this.font, category.name(), PALETTE_PADDING, y + 4, CATEGORY_TEXT_COLOR);
            }
            y += CATEGORY_HEADER_HEIGHT;

            // Blocks in category
            for (BlockDef block : category.blocks()) {
                if (y + BLOCK_HEIGHT > 0 && y < this.height) {
                    int blockWidth = PALETTE_WIDTH - PALETTE_PADDING * 2;
                    boolean hovered = mouseX >= PALETTE_PADDING && mouseX <= PALETTE_PADDING + blockWidth
                            && mouseY >= y && mouseY <= y + BLOCK_HEIGHT;
                    renderPaletteBlock(gfx, block.label(), category.color(), PALETTE_PADDING, y, blockWidth, hovered);
                }
                y += BLOCK_HEIGHT + BLOCK_PADDING;
            }
        }
    }

    private void renderPaletteBlock(GuiGraphics gfx, String label, int color, int x, int y, int w, boolean hovered) {
        int bgColor = hovered ? brighten(color, 30) : color;
        gfx.fill(x + 2, y + 2, x + w + 2, y + BLOCK_HEIGHT + 2, BLOCK_SHADOW_COLOR);
        gfx.fill(x, y, x + w, y + BLOCK_HEIGHT, bgColor);
        gfx.fill(x, y, x + w, y + 1, BLOCK_BORDER_COLOR);
        gfx.fill(x, y + BLOCK_HEIGHT - 1, x + w, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x, y, x + 1, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x + w - 1, y, x + w, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        int textY = y + (BLOCK_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, x + 8, textY, BLOCK_TEXT_COLOR);
    }

    private void renderBlock(GuiGraphics gfx, String label, int color, int x, int y, float zoom, boolean elevated) {
        int w = (int) (Math.max(100, this.font.width(label) + 20) * zoom);
        int h = (int) (BLOCK_HEIGHT * zoom);
        int shadowOff = elevated ? 4 : 2;
        gfx.fill(x + shadowOff, y + shadowOff, x + w + shadowOff, y + h + shadowOff, BLOCK_SHADOW_COLOR);
        gfx.fill(x, y, x + w, y + h, color);
        gfx.fill(x, y, x + w, y + 1, BLOCK_BORDER_COLOR);
        gfx.fill(x, y + h - 1, x + w, y + h, BLOCK_BORDER_COLOR);
        gfx.fill(x, y, x + 1, y + h, BLOCK_BORDER_COLOR);
        gfx.fill(x + w - 1, y, x + w, y + h, BLOCK_BORDER_COLOR);

        // Scale text with PoseStack
        gfx.pose().pushPose();
        gfx.pose().translate(x + 10 * zoom, y + (h - this.font.lineHeight * zoom) / 2, 0);
        gfx.pose().scale(zoom, zoom, 1.0f);
        gfx.drawString(this.font, label, 0, 0, BLOCK_TEXT_COLOR);
        gfx.pose().popPose();
    }

    // --- Mouse handling ---

    @Override
    public boolean mouseClicked(double mouseX, double mouseY, int button) {
        // Left click
        if (button == 0) {
            // Check toggle button first
            int btnX = paletteVisible ? PALETTE_WIDTH : 0;
            int btnY = 4;
            if (mouseX >= btnX && mouseX <= btnX + TOGGLE_BTN_WIDTH
                    && mouseY >= btnY && mouseY <= btnY + TOGGLE_BTN_HEIGHT) {
                paletteVisible = !paletteVisible;
                return true;
            }

            // If we have a ghost block from palette, place it on the canvas
            if (ghostBlock != null) {
                placedBlocks.add(new PlacedBlock(
                        ghostBlock.name(), ghostBlock.label(), ghostColor,
                        screenToCanvasX(mouseX) - 50,
                        screenToCanvasY(mouseY) - BLOCK_HEIGHT / 2f
                ));
                ghostBlock = null;
                return true;
            }

            // Check if clicking a palette block
            if (paletteVisible && mouseX < PALETTE_WIDTH) {
                BlockDef clicked = getPaletteBlockAt(mouseX, mouseY);
                if (clicked != null) {
                    ghostBlock = clicked;
                    ghostColor = getCategoryColorForBlock(clicked);
                    return true;
                }
            }

            // Check if clicking an existing canvas block (for dragging)
            for (int i = placedBlocks.size() - 1; i >= 0; i--) {
                PlacedBlock block = placedBlocks.get(i);
                float bx = canvasToScreenX(block.x);
                float by = canvasToScreenY(block.y);
                float bw = Math.max(100, this.font.width(block.label) + 20) * canvasZoom;
                float bh = BLOCK_HEIGHT * canvasZoom;
                if (mouseX >= bx && mouseX <= bx + bw && mouseY >= by && mouseY <= by + bh) {
                    draggingBlock = block;
                    dragOffsetX = (float) (mouseX - bx);
                    dragOffsetY = (float) (mouseY - by);
                    // Move to top
                    placedBlocks.remove(i);
                    placedBlocks.add(block);
                    return true;
                }
            }

            // Left-click on empty canvas — start panning
            if (!(paletteVisible && mouseX < PALETTE_WIDTH)) {
                isPanning = true;
                panStartX = mouseX;
                panStartY = mouseY;
                panStartOffsetX = canvasOffsetX;
                panStartOffsetY = canvasOffsetY;
                return true;
            }
        }

        // Right click — delete block on canvas or cancel ghost
        if (button == 1) {
            if (ghostBlock != null) {
                ghostBlock = null;
                return true;
            }
            for (int i = placedBlocks.size() - 1; i >= 0; i--) {
                PlacedBlock block = placedBlocks.get(i);
                float bx = canvasToScreenX(block.x);
                float by = canvasToScreenY(block.y);
                float bw = Math.max(100, this.font.width(block.label) + 20) * canvasZoom;
                float bh = BLOCK_HEIGHT * canvasZoom;
                if (mouseX >= bx && mouseX <= bx + bw && mouseY >= by && mouseY <= by + bh) {
                    placedBlocks.remove(i);
                    return true;
                }
            }
        }

        // Middle click — start panning
        if (button == 2) {
            isPanning = true;
            panStartX = mouseX;
            panStartY = mouseY;
            panStartOffsetX = canvasOffsetX;
            panStartOffsetY = canvasOffsetY;
            return true;
        }

        return super.mouseClicked(mouseX, mouseY, button);
    }

    @Override
    public boolean mouseDragged(double mouseX, double mouseY, int button, double dragX, double dragY) {
        if ((button == 0 || button == 2) && draggingBlock != null) {
            draggingBlock.x = screenToCanvasX(mouseX - dragOffsetX);
            draggingBlock.y = screenToCanvasY(mouseY - dragOffsetY);
            return true;
        }
        if ((button == 0 || button == 2) && isPanning) {
            canvasOffsetX = panStartOffsetX + (float) (mouseX - panStartX);
            canvasOffsetY = panStartOffsetY + (float) (mouseY - panStartY);
            return true;
        }
        return super.mouseDragged(mouseX, mouseY, button, dragX, dragY);
    }

    @Override
    public boolean mouseReleased(double mouseX, double mouseY, int button) {
        if ((button == 0 || button == 2) && draggingBlock != null) {
            draggingBlock = null;
            return true;
        }
        if (button == 0 || button == 2) {
            isPanning = false;
            return true;
        }
        return super.mouseReleased(mouseX, mouseY, button);
    }

    @Override
    public boolean mouseScrolled(double mouseX, double mouseY, double scrollX, double scrollY) {
        // Palette scroll
        if (paletteVisible && mouseX < PALETTE_WIDTH) {
            paletteScrollOffset -= (int) (scrollY * 20);
            paletteScrollOffset = Math.max(0, paletteScrollOffset);
            return true;
        }

        // Canvas zoom — anchor to cursor position
        float oldZoom = canvasZoom;
        canvasZoom *= (float) Math.pow(1.15, scrollY);
        canvasZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, canvasZoom));

        // Adjust offset so the point under the cursor stays fixed
        float factor = canvasZoom / oldZoom;
        canvasOffsetX = (float) (mouseX - factor * (mouseX - canvasOffsetX));
        canvasOffsetY = (float) (mouseY - factor * (mouseY - canvasOffsetY));

        return true;
    }

    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        if (keyCode == 256) { // Escape
            this.onClose();
            return true;
        }
        return super.keyPressed(keyCode, scanCode, modifiers);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }

    // --- Helpers ---

    private BlockDef getPaletteBlockAt(double mouseX, double mouseY) {
        int y = PALETTE_PADDING + 14 - paletteScrollOffset;
        int blockWidth = PALETTE_WIDTH - PALETTE_PADDING * 2;

        for (Category category : VisualBlockRegistry.getCategories()) {
            y += 6 + CATEGORY_HEADER_HEIGHT;
            for (BlockDef block : category.blocks()) {
                if (mouseX >= PALETTE_PADDING && mouseX <= PALETTE_PADDING + blockWidth
                        && mouseY >= y && mouseY <= y + BLOCK_HEIGHT) {
                    return block;
                }
                y += BLOCK_HEIGHT + BLOCK_PADDING;
            }
        }
        return null;
    }

    private int getCategoryColorForBlock(BlockDef block) {
        for (Category category : VisualBlockRegistry.getCategories()) {
            for (BlockDef b : category.blocks()) {
                if (b == block) {
                    return category.color();
                }
            }
        }
        return 0xFF888888;
    }

    private static int brighten(int color, int amount) {
        int a = (color >> 24) & 0xFF;
        int r = Math.min(255, ((color >> 16) & 0xFF) + amount);
        int g = Math.min(255, ((color >> 8) & 0xFF) + amount);
        int b = Math.min(255, (color & 0xFF) + amount);
        return (a << 24) | (r << 16) | (g << 8) | b;
    }

    /**
     * A function block placed on the canvas.
     */
    private static class PlacedBlock {
        final String name;
        final String label;
        final int color;
        float x, y;

        PlacedBlock(String name, String label, int color, float x, float y) {
            this.name = name;
            this.label = label;
            this.color = color;
            this.x = x;
            this.y = y;
        }
    }
}
