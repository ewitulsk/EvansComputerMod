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
 * Provides a canvas with draggable function blocks and a palette sidebar.
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
    private static final int PALETTE_BG_COLOR = 0xFF2A2A3E;
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

    private final BlockPos terminalPos;

    // Canvas state
    private final List<PlacedBlock> placedBlocks = new ArrayList<>();
    private float canvasOffsetX = 0;
    private float canvasOffsetY = 0;

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

    /** Returns the left edge of the canvas area (0 when palette hidden, PALETTE_WIDTH when visible). */
    private int paletteLeft() {
        return paletteVisible ? PALETTE_WIDTH : 0;
    }

    @Override
    public void render(GuiGraphics gfx, int mouseX, int mouseY, float partialTick) {
        int pl = paletteLeft();

        // Background
        gfx.fill(0, 0, this.width, this.height, BACKGROUND_COLOR);

        // Canvas grid
        renderCanvasGrid(gfx);

        // Placed blocks
        for (PlacedBlock block : placedBlocks) {
            renderBlock(gfx, block.label, block.color,
                    (int) (block.x + canvasOffsetX) + pl,
                    (int) (block.y + canvasOffsetY),
                    block == draggingBlock);
        }

        // Palette
        if (paletteVisible) {
            renderPalette(gfx, mouseX, mouseY);
        }

        // Toggle button
        renderToggleButton(gfx, mouseX, mouseY);

        // Ghost block following cursor
        if (ghostBlock != null) {
            renderBlock(gfx, ghostBlock.label(), ghostColor,
                    mouseX - 50, mouseY - BLOCK_HEIGHT / 2, true);
        }
    }

    private void renderCanvasGrid(GuiGraphics gfx) {
        int gridSize = 32;
        int startX = paletteLeft();
        int ox = (int) (canvasOffsetX % gridSize);
        int oy = (int) (canvasOffsetY % gridSize);

        for (int x = startX + ox; x < this.width; x += gridSize) {
            gfx.fill(x, 0, x + 1, this.height, CANVAS_GRID_COLOR);
        }
        for (int y = oy; y < this.height; y += gridSize) {
            gfx.fill(startX, y, this.width, y + 1, CANVAS_GRID_COLOR);
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
        // Palette background
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
        // Shadow
        gfx.fill(x + 2, y + 2, x + w + 2, y + BLOCK_HEIGHT + 2, BLOCK_SHADOW_COLOR);
        // Block body
        gfx.fill(x, y, x + w, y + BLOCK_HEIGHT, bgColor);
        // Border
        gfx.fill(x, y, x + w, y + 1, BLOCK_BORDER_COLOR);
        gfx.fill(x, y + BLOCK_HEIGHT - 1, x + w, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x, y, x + 1, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x + w - 1, y, x + w, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        // Label
        int textY = y + (BLOCK_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, x + 8, textY, BLOCK_TEXT_COLOR);
    }

    private void renderBlock(GuiGraphics gfx, String label, int color, int x, int y, boolean elevated) {
        int w = Math.max(100, this.font.width(label) + 20);
        if (elevated) {
            // Larger shadow for elevated/dragging blocks
            gfx.fill(x + 4, y + 4, x + w + 4, y + BLOCK_HEIGHT + 4, BLOCK_SHADOW_COLOR);
        } else {
            gfx.fill(x + 2, y + 2, x + w + 2, y + BLOCK_HEIGHT + 2, BLOCK_SHADOW_COLOR);
        }
        // Block body
        gfx.fill(x, y, x + w, y + BLOCK_HEIGHT, color);
        // Border
        gfx.fill(x, y, x + w, y + 1, BLOCK_BORDER_COLOR);
        gfx.fill(x, y + BLOCK_HEIGHT - 1, x + w, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x, y, x + 1, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x + w - 1, y, x + w, y + BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        // Label centered vertically
        int textY = y + (BLOCK_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, x + 10, textY, BLOCK_TEXT_COLOR);
    }

    // --- Mouse handling ---

    @Override
    public boolean mouseClicked(double mouseX, double mouseY, int button) {
        int pl = paletteLeft();

        // Left click
        if (button == 0) {
            // Check toggle button
            int btnX = paletteVisible ? PALETTE_WIDTH : 0;
            int btnY = 4;
            if (mouseX >= btnX && mouseX <= btnX + TOGGLE_BTN_WIDTH
                    && mouseY >= btnY && mouseY <= btnY + TOGGLE_BTN_HEIGHT) {
                paletteVisible = !paletteVisible;
                return true;
            }

            // If we have a ghost block from palette, place it on the canvas
            if (ghostBlock != null && mouseX > pl) {
                placedBlocks.add(new PlacedBlock(
                        ghostBlock.name(), ghostBlock.label(), ghostColor,
                        (float) (mouseX - pl - canvasOffsetX - 50),
                        (float) (mouseY - canvasOffsetY - BLOCK_HEIGHT / 2)
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
            if (mouseX > pl) {
                for (int i = placedBlocks.size() - 1; i >= 0; i--) {
                    PlacedBlock block = placedBlocks.get(i);
                    int bx = (int) (block.x + canvasOffsetX) + pl;
                    int by = (int) (block.y + canvasOffsetY);
                    int bw = Math.max(100, this.font.width(block.label) + 20);
                    if (mouseX >= bx && mouseX <= bx + bw && mouseY >= by && mouseY <= by + BLOCK_HEIGHT) {
                        draggingBlock = block;
                        dragOffsetX = (float) (mouseX - bx);
                        dragOffsetY = (float) (mouseY - by);
                        // Move to top
                        placedBlocks.remove(i);
                        placedBlocks.add(block);
                        return true;
                    }
                }
            }
        }

        // Right click — delete block on canvas
        if (button == 1 && mouseX > pl) {
            // Cancel ghost if active
            if (ghostBlock != null) {
                ghostBlock = null;
                return true;
            }
            for (int i = placedBlocks.size() - 1; i >= 0; i--) {
                PlacedBlock block = placedBlocks.get(i);
                int bx = (int) (block.x + canvasOffsetX) + pl;
                int by = (int) (block.y + canvasOffsetY);
                int bw = Math.max(100, this.font.width(block.label) + 20);
                if (mouseX >= bx && mouseX <= bx + bw && mouseY >= by && mouseY <= by + BLOCK_HEIGHT) {
                    placedBlocks.remove(i);
                    return true;
                }
            }
        }

        // Middle click — start panning
        if (button == 2 && mouseX > pl) {
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
        if (button == 0 && draggingBlock != null) {
            draggingBlock.x = (float) (mouseX - dragOffsetX - paletteLeft() - canvasOffsetX);
            draggingBlock.y = (float) (mouseY - dragOffsetY - canvasOffsetY);
            return true;
        }
        if (button == 2 && isPanning) {
            canvasOffsetX = panStartOffsetX + (float) (mouseX - panStartX);
            canvasOffsetY = panStartOffsetY + (float) (mouseY - panStartY);
            return true;
        }
        return super.mouseDragged(mouseX, mouseY, button, dragX, dragY);
    }

    @Override
    public boolean mouseReleased(double mouseX, double mouseY, int button) {
        if (button == 0 && draggingBlock != null) {
            draggingBlock = null;
            return true;
        }
        if (button == 2) {
            isPanning = false;
            return true;
        }
        return super.mouseReleased(mouseX, mouseY, button);
    }

    @Override
    public boolean mouseScrolled(double mouseX, double mouseY, double scrollX, double scrollY) {
        if (paletteVisible && mouseX < PALETTE_WIDTH) {
            // Scroll palette
            paletteScrollOffset -= (int) (scrollY * 20);
            paletteScrollOffset = Math.max(0, paletteScrollOffset);
            return true;
        }
        return super.mouseScrolled(mouseX, mouseY, scrollX, scrollY);
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
