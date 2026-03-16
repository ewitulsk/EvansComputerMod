package com.example.evanscomputermod.client;

import com.example.evanscomputermod.client.VisualBlockRegistry.BlockDef;
import com.example.evanscomputermod.client.VisualBlockRegistry.Category;
import com.example.evanscomputermod.client.VisualBlockRegistry.PortDef;
import com.example.evanscomputermod.client.VisualCodeGenerator.BlockInstance;
import com.example.evanscomputermod.client.VisualProgramSerializer.DeserializedProgram;
import com.example.evanscomputermod.client.VisualProgramSerializer.SerializedBlock;
import com.example.evanscomputermod.client.VisualProgramSerializer.SerializedConnection;
import com.example.evanscomputermod.network.LoadVisualProgramPacket;
import com.example.evanscomputermod.network.RequestProgramListPacket;
import com.example.evanscomputermod.network.RunVisualScriptPacket;
import com.example.evanscomputermod.network.SaveVisualProgramPacket;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.neoforged.neoforge.network.PacketDistributor;

import java.util.*;

/**
 * Client-side visual programming screen.
 * Provides a zoomable/pannable canvas with blocks that have typed input/output ports,
 * connection wires between ports, inline value editing, and a Run button that
 * generates Python code and sends it to the server.
 */
public class VisualProgrammingScreen extends Screen {

    // Layout constants
    private static final int PALETTE_WIDTH = 140;
    private static final int PALETTE_PADDING = 6;
    private static final int BLOCK_PADDING = 3;
    private static final int CATEGORY_HEADER_HEIGHT = 18;
    private static final int PALETTE_BLOCK_HEIGHT = 22;

    // Block rendering constants (canvas space, before zoom)
    private static final int BLOCK_MIN_WIDTH = 120;
    private static final int BLOCK_HEADER_HEIGHT = 22;
    private static final int PORT_ROW_HEIGHT = 16;
    private static final int PORT_RADIUS = 5;
    private static final int PORT_HIT_RADIUS = 8;

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

    private static final int FLOW_PORT_COLOR = 0xFFFFFFFF;
    private static final int STRING_PORT_COLOR = 0xFF4EC9B0;
    private static final int NUMBER_PORT_COLOR = 0xFF569CD6;
    private static final int ANY_PORT_COLOR = 0xFFDCDCAA;
    private static final int CONNECTION_COLOR = 0xFFCCCCCC;
    private static final int FLOW_CONNECTION_COLOR = 0xFFFFFFFF;
    private static final int PORT_LABEL_COLOR = 0xFFAAAAAA;

    private static final int RUN_BTN_WIDTH = 60;
    private static final int RUN_BTN_HEIGHT = 24;
    private static final int RUN_BTN_COLOR = 0xFF2E7D32;
    private static final int RUN_BTN_HOVER_COLOR = 0xFF388E3C;

    private static final int SAVE_BTN_WIDTH = 60;
    private static final int SAVE_BTN_HEIGHT = 24;
    private static final int SAVE_BTN_COLOR = 0xFF1565C0;
    private static final int SAVE_BTN_HOVER_COLOR = 0xFF1976D2;

    private static final int LOAD_BTN_WIDTH = 60;
    private static final int LOAD_BTN_HEIGHT = 24;
    private static final int LOAD_BTN_COLOR = 0xFFE65100;
    private static final int LOAD_BTN_HOVER_COLOR = 0xFFEF6C00;

    private static final int BROWSER_WIDTH = 240;
    private static final int BROWSER_ENTRY_HEIGHT = 22;
    private static final int BROWSER_BG_COLOR = 0xEE2A2A3E;
    private static final int BROWSER_ENTRY_COLOR = 0xFF3A3A52;
    private static final int BROWSER_ENTRY_HOVER_COLOR = 0xFF4A4A66;
    private static final int BROWSER_BORDER_COLOR = 0xFF444466;

    private static final int NAME_FIELD_WIDTH = 160;
    private static final int NAME_FIELD_HEIGHT = 18;

    private static final int INPUT_FIELD_WIDTH = 80;
    private static final int INPUT_FIELD_HEIGHT = 14;
    private static final int INPUT_FIELD_BG = 0xFF1A1A2E;
    private static final int INPUT_FIELD_BORDER = 0xFF555577;
    private static final int INPUT_FIELD_TEXT = 0xFFDDDDDD;
    private static final int INPUT_FIELD_DIM_TEXT = 0xFF777799;
    private static final int INPUT_FIELD_ACTIVE_BORDER = 0xFF7777BB;

    // Zoom limits
    private static final float MIN_ZOOM = 0.25f;
    private static final float MAX_ZOOM = 3.0f;

    private final BlockPos terminalPos;

    // Canvas state
    private final List<PlacedBlock> placedBlocks = new ArrayList<>();
    private final List<Connection> connections = new ArrayList<>();
    private int nextBlockId = 1;
    private float canvasOffsetX = 0;
    private float canvasOffsetY = 0;
    private float canvasZoom = 1.0f;

    // Interaction state
    private PlacedBlock draggingBlock = null;
    private float dragOffsetX, dragOffsetY;
    private boolean isPanning = false;
    private double panStartX, panStartY;
    private float panStartOffsetX, panStartOffsetY;

    // Palette ghost
    private BlockDef ghostBlock = null;
    private int ghostColor = 0;

    // Palette visibility & scroll
    private boolean paletteVisible = true;
    private int paletteScrollOffset = 0;

    // Wire dragging state
    private boolean isDraggingWire = false;
    private int wireFromBlockId = -1;
    private String wireFromPort = null;
    private boolean wireFromIsOutput = true;
    private double wireMouseX, wireMouseY;

    // Inline editing state
    private PlacedBlock editingBlock = null;
    private String editingPort = null;
    private String editingValue = "";
    private int editCursorPos = 0;

    // Double-click detection
    private long lastClickTime = 0;
    private double lastClickX = 0, lastClickY = 0;

    // Status message (shown briefly after running)
    private String statusMessage = null;
    private long statusMessageTime = 0;

    // Save/Load state
    private String currentProgramName = "untitled";
    private boolean showProgramBrowser = false;
    private List<String> programList = new ArrayList<>();
    private int browserScrollOffset = 0;
    private boolean editingProgramName = false;
    private String editingNameValue = "";
    private int editNameCursorPos = 0;

    public VisualProgrammingScreen(BlockPos terminalPos) {
        super(Component.translatable("screen.evanscomputermod.visual_editor"));
        this.terminalPos = terminalPos;
    }

    @Override
    protected void init() {
        super.init();
    }

    // --- Coordinate conversion ---

    private float screenToCanvasX(double screenX) {
        return (float) ((screenX - canvasOffsetX) / canvasZoom);
    }

    private float screenToCanvasY(double screenY) {
        return (float) ((screenY - canvasOffsetY) / canvasZoom);
    }

    private float canvasToScreenX(float canvasX) {
        return canvasX * canvasZoom + canvasOffsetX;
    }

    private float canvasToScreenY(float canvasY) {
        return canvasY * canvasZoom + canvasOffsetY;
    }

    // --- Block geometry helpers ---

    /** Calculate the height of a placed block based on its ports. */
    private int getBlockHeight(PlacedBlock block) {
        int dataInputCount = block.definition.dataInputs().size();
        int dataOutputCount = block.definition.dataOutputs().size();
        int rows = Math.max(dataInputCount, dataOutputCount);
        int height = BLOCK_HEADER_HEIGHT + Math.max(rows, 0) * PORT_ROW_HEIGHT + 8;
        // Extra space for flow output labels when there are multiple flow outputs
        if (block.definition.flowOutputs().size() > 1) {
            height += 10;
        }
        return height;
    }

    /** Calculate the width of a placed block. */
    private int getBlockWidth(PlacedBlock block) {
        int labelWidth = this.font.width(block.definition.label()) + 20;
        // Account for port labels on both sides
        int maxLeftWidth = 0;
        for (PortDef p : block.definition.dataInputs()) {
            maxLeftWidth = Math.max(maxLeftWidth, this.font.width(p.name()) + INPUT_FIELD_WIDTH + 20);
        }
        int maxRightWidth = 0;
        for (PortDef p : block.definition.dataOutputs()) {
            maxRightWidth = Math.max(maxRightWidth, this.font.width(p.name()) + 20);
        }
        return Math.max(BLOCK_MIN_WIDTH, Math.max(labelWidth, maxLeftWidth + maxRightWidth + 10));
    }

    /** Get the screen position of a specific port on a block. */
    private float[] getPortScreenPos(PlacedBlock block, String portName, boolean isOutput) {
        float bx = canvasToScreenX(block.x);
        float by = canvasToScreenY(block.y);
        int bw = (int) (getBlockWidth(block) * canvasZoom);
        int bh = (int) (getBlockHeight(block) * canvasZoom);

        // Flow input ports: top center
        if (!isOutput) {
            List<PortDef> flowIns = block.definition.flowInputs();
            for (PortDef fi : flowIns) {
                if (fi.name().equals(portName)) {
                    return new float[]{bx + bw / 2f, by};
                }
            }
        }

        // Flow output ports: distributed along bottom
        if (isOutput) {
            List<PortDef> flowOuts = block.definition.flowOutputs();
            int idx = -1;
            for (int i = 0; i < flowOuts.size(); i++) {
                if (flowOuts.get(i).name().equals(portName)) { idx = i; break; }
            }
            if (idx >= 0) {
                float x = bx + bw * (idx + 1f) / (flowOuts.size() + 1f);
                return new float[]{x, by + bh};
            }
        }

        // Data ports
        List<PortDef> ports = isOutput ? block.definition.dataOutputs() : block.definition.dataInputs();
        int index = 0;
        for (PortDef p : ports) {
            if (p.name().equals(portName)) break;
            index++;
        }

        float portY = by + BLOCK_HEADER_HEIGHT * canvasZoom + (index + 0.5f) * PORT_ROW_HEIGHT * canvasZoom;
        float portX = isOutput ? bx + bw : bx;

        return new float[]{portX, portY};
    }

    /** Get the canvas position of a port (unscaled). */
    private float[] getPortCanvasPos(PlacedBlock block, String portName, boolean isOutput) {
        int bw = getBlockWidth(block);
        int bh = getBlockHeight(block);

        // Flow input: top center
        if (!isOutput) {
            for (PortDef fi : block.definition.flowInputs()) {
                if (fi.name().equals(portName)) {
                    return new float[]{block.x + bw / 2f, block.y};
                }
            }
        }

        // Flow outputs: distributed along bottom
        if (isOutput) {
            List<PortDef> flowOuts = block.definition.flowOutputs();
            int idx = -1;
            for (int i = 0; i < flowOuts.size(); i++) {
                if (flowOuts.get(i).name().equals(portName)) { idx = i; break; }
            }
            if (idx >= 0) {
                float x = block.x + bw * (idx + 1f) / (flowOuts.size() + 1f);
                return new float[]{x, block.y + bh};
            }
        }

        List<PortDef> ports = isOutput ? block.definition.dataOutputs() : block.definition.dataInputs();
        int index = 0;
        for (PortDef p : ports) {
            if (p.name().equals(portName)) break;
            index++;
        }

        float portY = block.y + BLOCK_HEADER_HEIGHT + (index + 0.5f) * PORT_ROW_HEIGHT;
        float portX = isOutput ? block.x + bw : block.x;

        return new float[]{portX, portY};
    }

    // --- Rendering ---

    @Override
    public void render(GuiGraphics gfx, int mouseX, int mouseY, float partialTick) {
        // Background
        gfx.fill(0, 0, this.width, this.height, BACKGROUND_COLOR);
        renderCanvasGrid(gfx);

        // Connections (behind blocks)
        for (Connection conn : connections) {
            PlacedBlock fromBlock = getBlockById(conn.fromBlockId);
            PlacedBlock toBlock = getBlockById(conn.toBlockId);
            if (fromBlock == null || toBlock == null) continue;

            float[] from = getPortScreenPos(fromBlock, conn.fromPort, true);
            float[] to = getPortScreenPos(toBlock, conn.toPort, false);

            // Check if this is a flow connection by looking at port types
            PortDef fromPortDef = findPortDef(fromBlock, conn.fromPort, true);
            boolean isFlow = fromPortDef != null && fromPortDef.isFlow();
            int color = isFlow ? FLOW_CONNECTION_COLOR : CONNECTION_COLOR;
            renderBezierCurve(gfx, from[0], from[1], to[0], to[1], color);
        }

        // Wire being dragged
        if (isDraggingWire && wireFromBlockId >= 0) {
            PlacedBlock fromBlock = getBlockById(wireFromBlockId);
            if (fromBlock != null) {
                float[] from = getPortScreenPos(fromBlock, wireFromPort, wireFromIsOutput);
                PortDef wireDef = findPortDef(fromBlock, wireFromPort, wireFromIsOutput);
                int color = (wireDef != null && wireDef.isFlow()) ? FLOW_CONNECTION_COLOR : CONNECTION_COLOR;
                renderBezierCurve(gfx, from[0], from[1], (float) wireMouseX, (float) wireMouseY, color);
            }
        }

        // Placed blocks
        for (PlacedBlock block : placedBlocks) {
            renderCanvasBlock(gfx, block, mouseX, mouseY);
        }

        // Palette overlay
        if (paletteVisible) {
            renderPalette(gfx, mouseX, mouseY);
        }

        // Toggle button
        renderToggleButton(gfx, mouseX, mouseY);

        // Run button
        renderRunButton(gfx, mouseX, mouseY);

        // Save/Load buttons
        renderSaveButton(gfx, mouseX, mouseY);
        renderLoadButton(gfx, mouseX, mouseY);

        // Program name
        renderProgramName(gfx, mouseX, mouseY);

        // Program browser overlay
        if (showProgramBrowser) {
            renderProgramBrowser(gfx, mouseX, mouseY);
        }

        // Ghost block following cursor
        if (ghostBlock != null) {
            renderPaletteBlockItem(gfx, ghostBlock.label(), ghostColor,
                    mouseX - 50, mouseY - PALETTE_BLOCK_HEIGHT / 2, 100, true);
        }

        // Zoom indicator
        String zoomText = String.format("%.0f%%", canvasZoom * 100);
        gfx.drawString(this.font, zoomText, this.width - this.font.width(zoomText) - 6, 6, 0xFF888888);

        // Status message
        if (statusMessage != null) {
            long elapsed = System.currentTimeMillis() - statusMessageTime;
            if (elapsed > 3000) {
                statusMessage = null;
            } else {
                int alpha = (int) (255 * Math.max(0, 1.0 - elapsed / 3000.0));
                int statusColor = (alpha << 24) | 0x00FFFFFF;
                int sw = this.font.width(statusMessage);
                gfx.drawString(this.font, statusMessage,
                        (this.width - sw) / 2, this.height - 30, statusColor);
            }
        }
    }

    private void renderCanvasGrid(GuiGraphics gfx) {
        float gridSize = 32 * canvasZoom;
        if (gridSize < 8) return;

        float ox = canvasOffsetX % gridSize;
        float oy = canvasOffsetY % gridSize;

        for (float x = ox; x < this.width; x += gridSize) {
            gfx.fill((int) x, 0, (int) x + 1, this.height, CANVAS_GRID_COLOR);
        }
        for (float y = oy; y < this.height; y += gridSize) {
            gfx.fill(0, (int) y, this.width, (int) y + 1, CANVAS_GRID_COLOR);
        }
    }

    private void renderCanvasBlock(GuiGraphics gfx, PlacedBlock block, int mouseX, int mouseY) {
        float sx = canvasToScreenX(block.x);
        float sy = canvasToScreenY(block.y);
        int bw = (int) (getBlockWidth(block) * canvasZoom);
        int bh = (int) (getBlockHeight(block) * canvasZoom);

        int ix = (int) sx;
        int iy = (int) sy;

        // Shadow
        int shadowOff = (block == draggingBlock) ? 4 : 2;
        gfx.fill(ix + shadowOff, iy + shadowOff, ix + bw + shadowOff, iy + bh + shadowOff, BLOCK_SHADOW_COLOR);

        // Block body
        gfx.fill(ix, iy, ix + bw, iy + bh, block.color);

        // Border
        gfx.fill(ix, iy, ix + bw, iy + 1, BLOCK_BORDER_COLOR);
        gfx.fill(ix, iy + bh - 1, ix + bw, iy + bh, BLOCK_BORDER_COLOR);
        gfx.fill(ix, iy, ix + 1, iy + bh, BLOCK_BORDER_COLOR);
        gfx.fill(ix + bw - 1, iy, ix + bw, iy + bh, BLOCK_BORDER_COLOR);

        // Header label
        gfx.pose().pushPose();
        gfx.pose().translate(ix + 8 * canvasZoom, iy + 4 * canvasZoom, 0);
        gfx.pose().scale(canvasZoom, canvasZoom, 1.0f);
        gfx.drawString(this.font, block.definition.label(), 0, 0, BLOCK_TEXT_COLOR);
        gfx.pose().popPose();

        // Header separator line
        int headerBottom = (int) (iy + BLOCK_HEADER_HEIGHT * canvasZoom);
        gfx.fill(ix, headerBottom - 1, ix + bw, headerBottom, BLOCK_BORDER_COLOR);

        // Flow input ports
        for (PortDef fp : block.definition.flowInputs()) {
            float[] pos = getPortScreenPos(block, fp.name(), false);
            renderFlowPort(gfx, (int) pos[0], (int) pos[1], true);
        }

        // Flow output ports (with labels when multiple)
        List<PortDef> flowOuts = block.definition.flowOutputs();
        for (PortDef fp : flowOuts) {
            float[] pos = getPortScreenPos(block, fp.name(), true);
            renderFlowPort(gfx, (int) pos[0], (int) pos[1], false);

            // Render label below the port if there are multiple flow outputs
            if (flowOuts.size() > 1) {
                gfx.pose().pushPose();
                int labelW = this.font.width(fp.name());
                gfx.pose().translate(pos[0] - labelW * canvasZoom * 0.7f / 2, pos[1] + 2 * canvasZoom, 0);
                gfx.pose().scale(canvasZoom * 0.7f, canvasZoom * 0.7f, 1.0f);
                gfx.drawString(this.font, fp.name(), 0, 0, FLOW_PORT_COLOR);
                gfx.pose().popPose();
            }
        }

        // Data input ports
        List<PortDef> dataInputs = block.definition.dataInputs();
        for (int i = 0; i < dataInputs.size(); i++) {
            PortDef port = dataInputs.get(i);
            float[] pos = getPortScreenPos(block, port.name(), false);
            int portColor = getPortColor(port.type());
            renderDataPort(gfx, (int) pos[0], (int) pos[1], portColor);

            // Port label
            gfx.pose().pushPose();
            float labelX = pos[0] + PORT_RADIUS * canvasZoom + 3 * canvasZoom;
            float labelY = pos[1] - this.font.lineHeight * canvasZoom / 2;
            gfx.pose().translate(labelX, labelY, 0);
            gfx.pose().scale(canvasZoom, canvasZoom, 1.0f);
            gfx.drawString(this.font, port.name(), 0, 0, PORT_LABEL_COLOR);
            gfx.pose().popPose();

            // Inline value field (only if not connected)
            if (!isInputConnected(block.id, port.name())) {
                String value = block.inputValues.getOrDefault(port.name(),
                        port.defaultValue() != null ? port.defaultValue() : "");
                boolean isEditing = editingBlock == block && port.name().equals(editingPort);

                float fieldX = labelX + this.font.width(port.name()) * canvasZoom + 4 * canvasZoom;
                float fieldY = pos[1] - INPUT_FIELD_HEIGHT * canvasZoom / 2;
                int fw = (int) (INPUT_FIELD_WIDTH * canvasZoom);
                int fh = (int) (INPUT_FIELD_HEIGHT * canvasZoom);

                gfx.fill((int) fieldX, (int) fieldY, (int) fieldX + fw, (int) fieldY + fh, INPUT_FIELD_BG);
                gfx.fill((int) fieldX, (int) fieldY, (int) fieldX + fw, (int) fieldY + 1,
                        isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);
                gfx.fill((int) fieldX, (int) fieldY + fh - 1, (int) fieldX + fw, (int) fieldY + fh,
                        isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);
                gfx.fill((int) fieldX, (int) fieldY, (int) fieldX + 1, (int) fieldY + fh,
                        isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);
                gfx.fill((int) fieldX + fw - 1, (int) fieldY, (int) fieldX + fw, (int) fieldY + fh,
                        isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);

                String displayValue = isEditing ? editingValue : value;
                // Use dim color for default/unedited values, bright for active editing
                boolean isDefaultValue = !isEditing && port.defaultValue() != null && value.equals(port.defaultValue());
                int textColor = isEditing ? INPUT_FIELD_TEXT : (isDefaultValue ? INPUT_FIELD_DIM_TEXT : INPUT_FIELD_TEXT);

                gfx.pose().pushPose();
                float textY2 = fieldY + (fh - this.font.lineHeight * canvasZoom) / 2;
                gfx.pose().translate(fieldX + 3 * canvasZoom, textY2, 0);
                gfx.pose().scale(canvasZoom, canvasZoom, 1.0f);
                gfx.drawString(this.font, displayValue, 0, 0, textColor);
                // Cursor blink
                if (isEditing && (System.currentTimeMillis() / 500) % 2 == 0) {
                    int cursorX = this.font.width(displayValue.substring(0, Math.min(editCursorPos, displayValue.length())));
                    gfx.fill(cursorX, 0, cursorX + 1, this.font.lineHeight, INPUT_FIELD_TEXT);
                }
                gfx.pose().popPose();
            }
        }

        // Data output ports
        List<PortDef> dataOutputs = block.definition.dataOutputs();
        for (int i = 0; i < dataOutputs.size(); i++) {
            PortDef port = dataOutputs.get(i);
            float[] pos = getPortScreenPos(block, port.name(), true);
            int portColor = getPortColor(port.type());
            renderDataPort(gfx, (int) pos[0], (int) pos[1], portColor);

            // Port label (right-aligned)
            gfx.pose().pushPose();
            int labelWidth = this.font.width(port.name());
            float labelX = pos[0] - PORT_RADIUS * canvasZoom - 3 * canvasZoom - labelWidth * canvasZoom;
            float labelY = pos[1] - this.font.lineHeight * canvasZoom / 2;
            gfx.pose().translate(labelX, labelY, 0);
            gfx.pose().scale(canvasZoom, canvasZoom, 1.0f);
            gfx.drawString(this.font, port.name(), 0, 0, PORT_LABEL_COLOR);
            gfx.pose().popPose();
        }
    }

    private void renderFlowPort(GuiGraphics gfx, int cx, int cy, boolean isInput) {
        // Draw as a small triangle/diamond shape for flow ports
        int r = (int) (PORT_RADIUS * canvasZoom);
        // Simplified: draw as a filled circle
        gfx.fill(cx - r, cy - r, cx + r, cy + r, FLOW_PORT_COLOR);
        gfx.fill(cx - r + 1, cy - r + 1, cx + r - 1, cy + r - 1, isInput ? 0xFF333355 : FLOW_PORT_COLOR);
    }

    private void renderDataPort(GuiGraphics gfx, int cx, int cy, int color) {
        int r = (int) (PORT_RADIUS * canvasZoom);
        gfx.fill(cx - r, cy - r, cx + r, cy + r, color);
    }

    private void renderBezierCurve(GuiGraphics gfx, float x1, float y1, float x2, float y2, int color) {
        // Simplified bezier: draw as connected line segments
        int segments = 20;
        float dx = Math.abs(x2 - x1) * 0.5f;

        for (int i = 0; i < segments; i++) {
            float t1 = (float) i / segments;
            float t2 = (float) (i + 1) / segments;

            // Cubic bezier with horizontal control points
            float cx1 = x1 + dx;
            float cy1 = y1;
            float cx2 = x2 - dx;
            float cy2 = y2;

            float px1 = bezier(t1, x1, cx1, cx2, x2);
            float py1 = bezier(t1, y1, cy1, cy2, y2);
            float px2 = bezier(t2, x1, cx1, cx2, x2);
            float py2 = bezier(t2, y1, cy1, cy2, y2);

            // Draw line segment
            drawLine(gfx, (int) px1, (int) py1, (int) px2, (int) py2, color);
        }
    }

    private float bezier(float t, float p0, float p1, float p2, float p3) {
        float u = 1 - t;
        return u * u * u * p0 + 3 * u * u * t * p1 + 3 * u * t * t * p2 + t * t * t * p3;
    }

    private void drawLine(GuiGraphics gfx, int x1, int y1, int x2, int y2, int color) {
        // Bresenham-ish thick line using small fills
        int dx = Math.abs(x2 - x1);
        int dy = Math.abs(y2 - y1);

        if (dx >= dy) {
            // More horizontal
            if (x1 > x2) {
                int tmp = x1; x1 = x2; x2 = tmp;
                tmp = y1; y1 = y2; y2 = tmp;
            }
            for (int x = x1; x <= x2; x++) {
                float t = dx == 0 ? 0 : (float) (x - x1) / dx;
                int y = y1 + (int) ((y2 - y1) * t);
                gfx.fill(x, y, x + 2, y + 2, color);
            }
        } else {
            // More vertical
            if (y1 > y2) {
                int tmp = x1; x1 = x2; x2 = tmp;
                tmp = y1; y1 = y2; y2 = tmp;
            }
            for (int y = y1; y <= y2; y++) {
                float t = dy == 0 ? 0 : (float) (y - y1) / dy;
                int x = x1 + (int) ((x2 - x1) * t);
                gfx.fill(x, y, x + 2, y + 2, color);
            }
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

    private void renderRunButton(GuiGraphics gfx, int mouseX, int mouseY) {
        int btnX = this.width - RUN_BTN_WIDTH - 10;
        int btnY = 30;
        boolean hovered = mouseX >= btnX && mouseX <= btnX + RUN_BTN_WIDTH
                && mouseY >= btnY && mouseY <= btnY + RUN_BTN_HEIGHT;
        int bgColor = hovered ? RUN_BTN_HOVER_COLOR : RUN_BTN_COLOR;

        gfx.fill(btnX, btnY, btnX + RUN_BTN_WIDTH, btnY + RUN_BTN_HEIGHT, bgColor);
        gfx.fill(btnX, btnY, btnX + RUN_BTN_WIDTH, btnY + 1, 0xFF1B5E20);
        gfx.fill(btnX, btnY + RUN_BTN_HEIGHT - 1, btnX + RUN_BTN_WIDTH, btnY + RUN_BTN_HEIGHT, 0xFF1B5E20);
        gfx.fill(btnX, btnY, btnX + 1, btnY + RUN_BTN_HEIGHT, 0xFF1B5E20);
        gfx.fill(btnX + RUN_BTN_WIDTH - 1, btnY, btnX + RUN_BTN_WIDTH, btnY + RUN_BTN_HEIGHT, 0xFF1B5E20);

        String label = "Run ▶";
        int textX = btnX + (RUN_BTN_WIDTH - this.font.width(label)) / 2;
        int textY = btnY + (RUN_BTN_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, textX, textY, BLOCK_TEXT_COLOR);
    }

    private void renderSaveButton(GuiGraphics gfx, int mouseX, int mouseY) {
        int btnX = this.width - RUN_BTN_WIDTH - SAVE_BTN_WIDTH - 20;
        int btnY = 30;
        boolean hovered = mouseX >= btnX && mouseX <= btnX + SAVE_BTN_WIDTH
                && mouseY >= btnY && mouseY <= btnY + SAVE_BTN_HEIGHT;
        int bgColor = hovered ? SAVE_BTN_HOVER_COLOR : SAVE_BTN_COLOR;

        gfx.fill(btnX, btnY, btnX + SAVE_BTN_WIDTH, btnY + SAVE_BTN_HEIGHT, bgColor);
        gfx.fill(btnX, btnY, btnX + SAVE_BTN_WIDTH, btnY + 1, 0xFF0D47A1);
        gfx.fill(btnX, btnY + SAVE_BTN_HEIGHT - 1, btnX + SAVE_BTN_WIDTH, btnY + SAVE_BTN_HEIGHT, 0xFF0D47A1);
        gfx.fill(btnX, btnY, btnX + 1, btnY + SAVE_BTN_HEIGHT, 0xFF0D47A1);
        gfx.fill(btnX + SAVE_BTN_WIDTH - 1, btnY, btnX + SAVE_BTN_WIDTH, btnY + SAVE_BTN_HEIGHT, 0xFF0D47A1);

        String label = "Save";
        int textX = btnX + (SAVE_BTN_WIDTH - this.font.width(label)) / 2;
        int textY = btnY + (SAVE_BTN_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, textX, textY, BLOCK_TEXT_COLOR);
    }

    private void renderLoadButton(GuiGraphics gfx, int mouseX, int mouseY) {
        int btnX = this.width - RUN_BTN_WIDTH - SAVE_BTN_WIDTH - LOAD_BTN_WIDTH - 30;
        int btnY = 30;
        boolean hovered = mouseX >= btnX && mouseX <= btnX + LOAD_BTN_WIDTH
                && mouseY >= btnY && mouseY <= btnY + LOAD_BTN_HEIGHT;
        int bgColor = hovered ? LOAD_BTN_HOVER_COLOR : LOAD_BTN_COLOR;

        gfx.fill(btnX, btnY, btnX + LOAD_BTN_WIDTH, btnY + LOAD_BTN_HEIGHT, bgColor);
        gfx.fill(btnX, btnY, btnX + LOAD_BTN_WIDTH, btnY + 1, 0xFFBF360C);
        gfx.fill(btnX, btnY + LOAD_BTN_HEIGHT - 1, btnX + LOAD_BTN_WIDTH, btnY + LOAD_BTN_HEIGHT, 0xFFBF360C);
        gfx.fill(btnX, btnY, btnX + 1, btnY + LOAD_BTN_HEIGHT, 0xFFBF360C);
        gfx.fill(btnX + LOAD_BTN_WIDTH - 1, btnY, btnX + LOAD_BTN_WIDTH, btnY + LOAD_BTN_HEIGHT, 0xFFBF360C);

        String label = "Load";
        int textX = btnX + (LOAD_BTN_WIDTH - this.font.width(label)) / 2;
        int textY = btnY + (LOAD_BTN_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, textX, textY, BLOCK_TEXT_COLOR);
    }

    private void renderProgramName(GuiGraphics gfx, int mouseX, int mouseY) {
        int fieldX = (this.width - NAME_FIELD_WIDTH) / 2;
        int fieldY = 6;
        boolean isEditing = editingProgramName;

        gfx.fill(fieldX, fieldY, fieldX + NAME_FIELD_WIDTH, fieldY + NAME_FIELD_HEIGHT, INPUT_FIELD_BG);
        gfx.fill(fieldX, fieldY, fieldX + NAME_FIELD_WIDTH, fieldY + 1,
                isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);
        gfx.fill(fieldX, fieldY + NAME_FIELD_HEIGHT - 1, fieldX + NAME_FIELD_WIDTH, fieldY + NAME_FIELD_HEIGHT,
                isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);
        gfx.fill(fieldX, fieldY, fieldX + 1, fieldY + NAME_FIELD_HEIGHT,
                isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);
        gfx.fill(fieldX + NAME_FIELD_WIDTH - 1, fieldY, fieldX + NAME_FIELD_WIDTH, fieldY + NAME_FIELD_HEIGHT,
                isEditing ? INPUT_FIELD_ACTIVE_BORDER : INPUT_FIELD_BORDER);

        String display = isEditing ? editingNameValue : currentProgramName;
        int textColor = isEditing ? INPUT_FIELD_TEXT : (currentProgramName.equals("untitled") ? INPUT_FIELD_DIM_TEXT : INPUT_FIELD_TEXT);
        int textY = fieldY + (NAME_FIELD_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, display, fieldX + 4, textY, textColor);

        if (isEditing && (System.currentTimeMillis() / 500) % 2 == 0) {
            int cursorX = fieldX + 4 + this.font.width(display.substring(0, Math.min(editNameCursorPos, display.length())));
            gfx.fill(cursorX, textY, cursorX + 1, textY + this.font.lineHeight, INPUT_FIELD_TEXT);
        }
    }

    private void renderProgramBrowser(GuiGraphics gfx, int mouseX, int mouseY) {
        int browserHeight = Math.min(BROWSER_ENTRY_HEIGHT * Math.max(programList.size(), 1) + 30, this.height - 60);
        int bx = (this.width - BROWSER_WIDTH) / 2;
        int by = (this.height - browserHeight) / 2;

        // Background
        gfx.fill(bx, by, bx + BROWSER_WIDTH, by + browserHeight, BROWSER_BG_COLOR);
        // Border
        gfx.fill(bx, by, bx + BROWSER_WIDTH, by + 1, BROWSER_BORDER_COLOR);
        gfx.fill(bx, by + browserHeight - 1, bx + BROWSER_WIDTH, by + browserHeight, BROWSER_BORDER_COLOR);
        gfx.fill(bx, by, bx + 1, by + browserHeight, BROWSER_BORDER_COLOR);
        gfx.fill(bx + BROWSER_WIDTH - 1, by, bx + BROWSER_WIDTH, by + browserHeight, BROWSER_BORDER_COLOR);

        // Title
        String title = "Load Program";
        gfx.drawString(this.font, title, bx + (BROWSER_WIDTH - this.font.width(title)) / 2, by + 6, BLOCK_TEXT_COLOR);

        int listY = by + 24 - browserScrollOffset;
        if (programList.isEmpty()) {
            gfx.drawString(this.font, "No saved programs", bx + 10, listY, INPUT_FIELD_DIM_TEXT);
        } else {
            for (int i = 0; i < programList.size(); i++) {
                int entryY = listY + i * BROWSER_ENTRY_HEIGHT;
                if (entryY + BROWSER_ENTRY_HEIGHT < by + 24 || entryY > by + browserHeight) continue;

                boolean hovered = mouseX >= bx + 4 && mouseX <= bx + BROWSER_WIDTH - 4
                        && mouseY >= entryY && mouseY <= entryY + BROWSER_ENTRY_HEIGHT - 2;
                int entryColor = hovered ? BROWSER_ENTRY_HOVER_COLOR : BROWSER_ENTRY_COLOR;
                gfx.fill(bx + 4, entryY, bx + BROWSER_WIDTH - 4, entryY + BROWSER_ENTRY_HEIGHT - 2, entryColor);
                gfx.drawString(this.font, programList.get(i),
                        bx + 10, entryY + (BROWSER_ENTRY_HEIGHT - 2 - this.font.lineHeight) / 2, BLOCK_TEXT_COLOR);
            }
        }
    }

    private void renderPalette(GuiGraphics gfx, int mouseX, int mouseY) {
        gfx.fill(0, 0, PALETTE_WIDTH, this.height, PALETTE_BG_COLOR);
        gfx.fill(PALETTE_WIDTH - 1, 0, PALETTE_WIDTH, this.height, PALETTE_BORDER_COLOR);

        gfx.drawString(this.font, "Block Palette", PALETTE_PADDING, PALETTE_PADDING, BLOCK_TEXT_COLOR);

        int y = PALETTE_PADDING + 14 - paletteScrollOffset;

        for (Category category : VisualBlockRegistry.getCategories()) {
            y += 6;
            if (y + CATEGORY_HEADER_HEIGHT > 0 && y < this.height) {
                gfx.drawString(this.font, category.name(), PALETTE_PADDING, y + 4, CATEGORY_TEXT_COLOR);
            }
            y += CATEGORY_HEADER_HEIGHT;

            for (BlockDef block : category.blocks()) {
                if (y + PALETTE_BLOCK_HEIGHT > 0 && y < this.height) {
                    int blockWidth = PALETTE_WIDTH - PALETTE_PADDING * 2;
                    boolean hovered = mouseX >= PALETTE_PADDING && mouseX <= PALETTE_PADDING + blockWidth
                            && mouseY >= y && mouseY <= y + PALETTE_BLOCK_HEIGHT;
                    renderPaletteBlockItem(gfx, block.label(), category.color(), PALETTE_PADDING, y, blockWidth, hovered);
                }
                y += PALETTE_BLOCK_HEIGHT + BLOCK_PADDING;
            }
        }
    }

    private void renderPaletteBlockItem(GuiGraphics gfx, String label, int color, int x, int y, int w, boolean hovered) {
        int bgColor = hovered ? brighten(color, 30) : color;
        gfx.fill(x + 2, y + 2, x + w + 2, y + PALETTE_BLOCK_HEIGHT + 2, BLOCK_SHADOW_COLOR);
        gfx.fill(x, y, x + w, y + PALETTE_BLOCK_HEIGHT, bgColor);
        gfx.fill(x, y, x + w, y + 1, BLOCK_BORDER_COLOR);
        gfx.fill(x, y + PALETTE_BLOCK_HEIGHT - 1, x + w, y + PALETTE_BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x, y, x + 1, y + PALETTE_BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        gfx.fill(x + w - 1, y, x + w, y + PALETTE_BLOCK_HEIGHT, BLOCK_BORDER_COLOR);
        int textY = y + (PALETTE_BLOCK_HEIGHT - this.font.lineHeight) / 2;
        gfx.drawString(this.font, label, x + 8, textY, BLOCK_TEXT_COLOR);
    }

    // --- Mouse handling ---

    @Override
    public boolean mouseClicked(double mouseX, double mouseY, int button) {
        if (button == 0) {
            // Check program browser clicks first (modal overlay)
            if (showProgramBrowser) {
                int browserHeight = Math.min(BROWSER_ENTRY_HEIGHT * Math.max(programList.size(), 1) + 30, this.height - 60);
                int bx = (this.width - BROWSER_WIDTH) / 2;
                int by = (this.height - browserHeight) / 2;

                if (mouseX >= bx && mouseX <= bx + BROWSER_WIDTH && mouseY >= by && mouseY <= by + browserHeight) {
                    // Click inside browser
                    int listY = by + 24 - browserScrollOffset;
                    for (int i = 0; i < programList.size(); i++) {
                        int entryY = listY + i * BROWSER_ENTRY_HEIGHT;
                        if (mouseY >= entryY && mouseY <= entryY + BROWSER_ENTRY_HEIGHT - 2
                                && mouseX >= bx + 4 && mouseX <= bx + BROWSER_WIDTH - 4) {
                            PacketDistributor.sendToServer(
                                    new LoadVisualProgramPacket(terminalPos, programList.get(i)));
                            showProgramBrowser = false;
                            return true;
                        }
                    }
                    return true; // Consume click inside browser
                } else {
                    // Click outside browser closes it
                    showProgramBrowser = false;
                    return true;
                }
            }

            // Check program name field click
            int nameFieldX = (this.width - NAME_FIELD_WIDTH) / 2;
            int nameFieldY = 6;
            if (mouseX >= nameFieldX && mouseX <= nameFieldX + NAME_FIELD_WIDTH
                    && mouseY >= nameFieldY && mouseY <= nameFieldY + NAME_FIELD_HEIGHT) {
                if (!editingProgramName) {
                    editingProgramName = true;
                    editingNameValue = currentProgramName.equals("untitled") ? "" : currentProgramName;
                    editNameCursorPos = editingNameValue.length();
                    // Cancel any port editing
                    if (editingBlock != null) commitEditing();
                }
                return true;
            } else if (editingProgramName) {
                commitNameEditing();
            }

            // Check Save button
            int saveBtnX = this.width - RUN_BTN_WIDTH - SAVE_BTN_WIDTH - 20;
            int saveBtnY = 30;
            if (mouseX >= saveBtnX && mouseX <= saveBtnX + SAVE_BTN_WIDTH
                    && mouseY >= saveBtnY && mouseY <= saveBtnY + SAVE_BTN_HEIGHT) {
                saveVisualProgram();
                return true;
            }

            // Check Load button
            int loadBtnX = this.width - RUN_BTN_WIDTH - SAVE_BTN_WIDTH - LOAD_BTN_WIDTH - 30;
            int loadBtnY = 30;
            if (mouseX >= loadBtnX && mouseX <= loadBtnX + LOAD_BTN_WIDTH
                    && mouseY >= loadBtnY && mouseY <= loadBtnY + LOAD_BTN_HEIGHT) {
                PacketDistributor.sendToServer(new RequestProgramListPacket(terminalPos));
                showProgramBrowser = true;
                browserScrollOffset = 0;
                return true;
            }

            // Check Run button
            int runBtnX = this.width - RUN_BTN_WIDTH - 10;
            int runBtnY = 30;
            if (mouseX >= runBtnX && mouseX <= runBtnX + RUN_BTN_WIDTH
                    && mouseY >= runBtnY && mouseY <= runBtnY + RUN_BTN_HEIGHT) {
                runVisualProgram();
                return true;
            }

            // Check toggle button
            int btnX = paletteVisible ? PALETTE_WIDTH : 0;
            int btnY = 4;
            if (mouseX >= btnX && mouseX <= btnX + TOGGLE_BTN_WIDTH
                    && mouseY >= btnY && mouseY <= btnY + TOGGLE_BTN_HEIGHT) {
                paletteVisible = !paletteVisible;
                return true;
            }

            // Place ghost block
            if (ghostBlock != null) {
                PlacedBlock newBlock = new PlacedBlock(
                        nextBlockId++, ghostBlock, ghostColor,
                        screenToCanvasX(mouseX) - getBlockWidthForDef(ghostBlock) / 2f,
                        screenToCanvasY(mouseY) - 14
                );
                placedBlocks.add(newBlock);
                ghostBlock = null;
                return true;
            }

            // Check palette clicks
            if (paletteVisible && mouseX < PALETTE_WIDTH) {
                BlockDef clicked = getPaletteBlockAt(mouseX, mouseY);
                if (clicked != null) {
                    ghostBlock = clicked;
                    ghostColor = getCategoryColorForBlock(clicked);
                    return true;
                }
            }

            // Check input field clicks FIRST — before port hit test
            InputFieldHit fieldHit = findInputFieldAt(mouseX, mouseY);
            if (fieldHit != null) {
                startEditing(fieldHit.block, fieldHit.portDef);
                return true;
            }

            // Check port clicks (for wire dragging) — check before block dragging
            PortHitResult portHit = findPortAt(mouseX, mouseY);
            if (portHit != null) {
                isDraggingWire = true;
                wireFromBlockId = portHit.block.id;
                wireFromPort = portHit.portDef.name();
                wireFromIsOutput = portHit.isOutput;
                wireMouseX = mouseX;
                wireMouseY = mouseY;
                return true;
            }

            // Commit any active editing
            if (editingBlock != null) {
                commitEditing();
            }

            // Check block clicks (double-click to edit, single-click to drag)
            long now = System.currentTimeMillis();
            boolean isDoubleClick = (now - lastClickTime < 400)
                    && Math.abs(mouseX - lastClickX) < 5
                    && Math.abs(mouseY - lastClickY) < 5;
            lastClickTime = now;
            lastClickX = mouseX;
            lastClickY = mouseY;

            for (int i = placedBlocks.size() - 1; i >= 0; i--) {
                PlacedBlock block = placedBlocks.get(i);
                float bx = canvasToScreenX(block.x);
                float by = canvasToScreenY(block.y);
                float bw = getBlockWidth(block) * canvasZoom;
                float bh = getBlockHeight(block) * canvasZoom;
                if (mouseX >= bx && mouseX <= bx + bw && mouseY >= by && mouseY <= by + bh) {
                    // Double-click: edit first unconnected data input
                    if (isDoubleClick) {
                        for (PortDef input : block.definition.dataInputs()) {
                            if (!isInputConnected(block.id, input.name())) {
                                startEditing(block, input);
                                return true;
                            }
                        }
                    }
                    // Single-click: drag
                    draggingBlock = block;
                    dragOffsetX = (float) (mouseX - bx);
                    dragOffsetY = (float) (mouseY - by);
                    placedBlocks.remove(i);
                    placedBlocks.add(block);
                    return true;
                }
            }

            // Pan
            if (!(paletteVisible && mouseX < PALETTE_WIDTH)) {
                isPanning = true;
                panStartX = mouseX;
                panStartY = mouseY;
                panStartOffsetX = canvasOffsetX;
                panStartOffsetY = canvasOffsetY;
                return true;
            }
        }

        // Right click — delete block or cancel ghost or delete connection
        if (button == 1) {
            if (ghostBlock != null) {
                ghostBlock = null;
                return true;
            }

            // Check if right-clicking a port to delete its connection
            PortHitResult portHit = findPortAt(mouseX, mouseY);
            if (portHit != null) {
                removeConnectionsAt(portHit.block.id, portHit.portDef.name(), portHit.isOutput);
                return true;
            }

            // Delete block
            for (int i = placedBlocks.size() - 1; i >= 0; i--) {
                PlacedBlock block = placedBlocks.get(i);
                float bx = canvasToScreenX(block.x);
                float by = canvasToScreenY(block.y);
                float bw = getBlockWidth(block) * canvasZoom;
                float bh = getBlockHeight(block) * canvasZoom;
                if (mouseX >= bx && mouseX <= bx + bw && mouseY >= by && mouseY <= by + bh) {
                    // Remove block and all its connections
                    placedBlocks.remove(i);
                    connections.removeIf(c -> c.fromBlockId == block.id || c.toBlockId == block.id);
                    if (editingBlock == block) {
                        editingBlock = null;
                        editingPort = null;
                    }
                    return true;
                }
            }
        }

        // Middle click — pan
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
        if (isDraggingWire) {
            wireMouseX = mouseX;
            wireMouseY = mouseY;
            return true;
        }
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
        if (button == 0 && isDraggingWire) {
            isDraggingWire = false;

            // Check if we released on a compatible port
            PortHitResult portHit = findPortAt(mouseX, mouseY);
            if (portHit != null && portHit.block.id != wireFromBlockId) {
                // Determine from/to (output → input)
                int fromId, toId;
                String fromPort, toPort;

                if (wireFromIsOutput) {
                    fromId = wireFromBlockId;
                    fromPort = wireFromPort;
                    toId = portHit.block.id;
                    toPort = portHit.portDef.name();
                    if (portHit.isOutput) {
                        return true; // Can't connect output to output
                    }
                } else {
                    fromId = portHit.block.id;
                    fromPort = portHit.portDef.name();
                    toId = wireFromBlockId;
                    toPort = wireFromPort;
                    if (!portHit.isOutput) {
                        return true; // Can't connect input to input
                    }
                }

                // Type compatibility check
                PlacedBlock fromBlock = getBlockById(fromId);
                PlacedBlock toBlock = getBlockById(toId);
                if (fromBlock != null && toBlock != null) {
                    PortDef fromDef = findPortDef(fromBlock, fromPort, true);
                    PortDef toDef = findPortDef(toBlock, toPort, false);
                    if (fromDef != null && toDef != null) {
                        boolean compatible = fromDef.type().equals(toDef.type())
                                || "any".equals(fromDef.type()) || "any".equals(toDef.type());
                        if (compatible) {
                            // Remove existing connection to this input port
                            connections.removeIf(c -> c.toBlockId == toId && c.toPort.equals(toPort));
                            // For flow outputs, only allow one outgoing flow connection
                            if (fromDef.isFlow()) {
                                connections.removeIf(c -> c.fromBlockId == fromId && c.fromPort.equals(fromPort));
                            }
                            connections.add(new Connection(fromId, fromPort, toId, toPort));
                        }
                    }
                }
            }
            return true;
        }

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
        if (showProgramBrowser) {
            browserScrollOffset -= (int) (scrollY * 20);
            browserScrollOffset = Math.max(0, browserScrollOffset);
            return true;
        }
        if (paletteVisible && mouseX < PALETTE_WIDTH) {
            paletteScrollOffset -= (int) (scrollY * 20);
            paletteScrollOffset = Math.max(0, paletteScrollOffset);
            return true;
        }

        float oldZoom = canvasZoom;
        canvasZoom *= (float) Math.pow(1.15, scrollY);
        canvasZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, canvasZoom));

        float factor = canvasZoom / oldZoom;
        canvasOffsetX = (float) (mouseX - factor * (mouseX - canvasOffsetX));
        canvasOffsetY = (float) (mouseY - factor * (mouseY - canvasOffsetY));

        return true;
    }

    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        // Program name editing
        if (editingProgramName) {
            if (keyCode == 256) { // Escape — cancel
                editingProgramName = false;
                return true;
            }
            if (keyCode == 257 || keyCode == 335) { // Enter — commit
                commitNameEditing();
                return true;
            }
            if (keyCode == 259) { // Backspace
                if (editNameCursorPos > 0) {
                    editingNameValue = editingNameValue.substring(0, editNameCursorPos - 1) + editingNameValue.substring(editNameCursorPos);
                    editNameCursorPos--;
                }
                return true;
            }
            if (keyCode == 261) { // Delete
                if (editNameCursorPos < editingNameValue.length()) {
                    editingNameValue = editingNameValue.substring(0, editNameCursorPos) + editingNameValue.substring(editNameCursorPos + 1);
                }
                return true;
            }
            if (keyCode == 263) { editNameCursorPos = Math.max(0, editNameCursorPos - 1); return true; }
            if (keyCode == 262) { editNameCursorPos = Math.min(editingNameValue.length(), editNameCursorPos + 1); return true; }
            if (keyCode == 268) { editNameCursorPos = 0; return true; }
            if (keyCode == 269) { editNameCursorPos = editingNameValue.length(); return true; }
            return true;
        }

        if (editingBlock != null) {
            if (keyCode == 256) { // Escape — cancel editing
                editingBlock = null;
                editingPort = null;
                return true;
            }
            if (keyCode == 257 || keyCode == 335) { // Enter — commit
                commitEditing();
                return true;
            }
            if (keyCode == 259) { // Backspace
                if (editCursorPos > 0) {
                    editingValue = editingValue.substring(0, editCursorPos - 1) + editingValue.substring(editCursorPos);
                    editCursorPos--;
                }
                return true;
            }
            if (keyCode == 261) { // Delete
                if (editCursorPos < editingValue.length()) {
                    editingValue = editingValue.substring(0, editCursorPos) + editingValue.substring(editCursorPos + 1);
                }
                return true;
            }
            if (keyCode == 263) { // Left arrow
                editCursorPos = Math.max(0, editCursorPos - 1);
                return true;
            }
            if (keyCode == 262) { // Right arrow
                editCursorPos = Math.min(editingValue.length(), editCursorPos + 1);
                return true;
            }
            if (keyCode == 268) { // Home
                editCursorPos = 0;
                return true;
            }
            if (keyCode == 269) { // End
                editCursorPos = editingValue.length();
                return true;
            }
            return true; // Consume all keys while editing
        }

        if (keyCode == 256) { // Escape
            if (showProgramBrowser) {
                showProgramBrowser = false;
                return true;
            }
            this.onClose();
            return true;
        }
        return super.keyPressed(keyCode, scanCode, modifiers);
    }

    @Override
    public boolean charTyped(char codePoint, int modifiers) {
        if (editingProgramName) {
            // Only allow valid filename characters
            if (Character.isLetterOrDigit(codePoint) || codePoint == '_' || codePoint == '-') {
                editingNameValue = editingNameValue.substring(0, editNameCursorPos) + codePoint + editingNameValue.substring(editNameCursorPos);
                editNameCursorPos++;
            }
            return true;
        }
        if (editingBlock != null) {
            editingValue = editingValue.substring(0, editCursorPos) + codePoint + editingValue.substring(editCursorPos);
            editCursorPos++;
            return true;
        }
        return super.charTyped(codePoint, modifiers);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }

    // --- Save/Load ---

    private void commitNameEditing() {
        editingProgramName = false;
        if (!editingNameValue.isEmpty()) {
            currentProgramName = editingNameValue;
        }
    }

    private void saveVisualProgram() {
        if (editingProgramName) commitNameEditing();
        if (currentProgramName.equals("untitled")) {
            // Activate name editing so user can enter a name
            editingProgramName = true;
            editingNameValue = "";
            editNameCursorPos = 0;
            statusMessage = "Enter a program name, then Save again";
            statusMessageTime = System.currentTimeMillis();
            return;
        }

        // Build adapter lists for serialization
        List<VisualProgramSerializer.BlockInfo> blockInfos = new ArrayList<>();
        for (PlacedBlock block : placedBlocks) {
            blockInfos.add(new VisualProgramSerializer.BlockInfo() {
                @Override public int id() { return block.id; }
                @Override public BlockDef definition() { return block.definition; }
                @Override public float x() { return block.x; }
                @Override public float y() { return block.y; }
                @Override public Map<String, String> inputValues() { return block.inputValues; }
            });
        }
        List<VisualProgramSerializer.ConnectionInfo> connInfos = new ArrayList<>();
        for (Connection conn : connections) {
            connInfos.add(new VisualProgramSerializer.ConnectionInfo() {
                @Override public int fromBlockId() { return conn.fromBlockId; }
                @Override public String fromPort() { return conn.fromPort; }
                @Override public int toBlockId() { return conn.toBlockId; }
                @Override public String toPort() { return conn.toPort; }
            });
        }

        String json = VisualProgramSerializer.serialize(
                blockInfos, connInfos, canvasOffsetX, canvasOffsetY, canvasZoom, currentProgramName);
        PacketDistributor.sendToServer(new SaveVisualProgramPacket(terminalPos, currentProgramName, json));

        statusMessage = "Saved: " + currentProgramName;
        statusMessageTime = System.currentTimeMillis();
    }

    public void onProgramListReceived(List<String> programs) {
        this.programList = new ArrayList<>(programs);
        this.showProgramBrowser = true;
        this.browserScrollOffset = 0;
    }

    public void onProgramLoaded(String json) {
        try {
            DeserializedProgram program = VisualProgramSerializer.deserialize(json, nextBlockId);
            placedBlocks.clear();
            connections.clear();

            for (SerializedBlock sb : program.blocks()) {
                PlacedBlock block = new PlacedBlock(sb.id(), sb.definition(), sb.color(), sb.x(), sb.y());
                block.inputValues.clear();
                block.inputValues.putAll(sb.inputValues());
                placedBlocks.add(block);
            }
            for (SerializedConnection sc : program.connections()) {
                connections.add(new Connection(sc.fromBlockId(), sc.fromPort(), sc.toBlockId(), sc.toPort()));
            }

            nextBlockId = program.newNextBlockId();
            canvasOffsetX = program.canvasOffsetX();
            canvasOffsetY = program.canvasOffsetY();
            canvasZoom = program.zoom();
            currentProgramName = program.name();

            // Reset editing state
            editingBlock = null;
            editingPort = null;
            ghostBlock = null;
            showProgramBrowser = false;

            statusMessage = "Loaded: " + program.name();
            statusMessageTime = System.currentTimeMillis();
        } catch (Exception e) {
            statusMessage = "ERROR: Failed to load program";
            statusMessageTime = System.currentTimeMillis();
        }
    }

    // --- Run ---

    private void runVisualProgram() {
        // Build BlockInstance list
        List<BlockInstance> blockInstances = new ArrayList<>();
        for (PlacedBlock block : placedBlocks) {
            blockInstances.add(new BlockInstance(block.id, block.definition, block.inputValues));
        }

        // Build Connection list
        List<VisualCodeGenerator.Connection> connList = new ArrayList<>();
        for (Connection conn : connections) {
            connList.add(new VisualCodeGenerator.Connection(conn.fromBlockId, conn.fromPort, conn.toBlockId, conn.toPort));
        }

        String code = VisualCodeGenerator.generate(blockInstances, connList);

        if (code.startsWith("ERROR:")) {
            statusMessage = code;
            statusMessageTime = System.currentTimeMillis();
            return;
        }

        // Send to server
        PacketDistributor.sendToServer(new RunVisualScriptPacket(terminalPos, code));

        statusMessage = "Script sent to terminal!";
        statusMessageTime = System.currentTimeMillis();
    }

    // --- Helpers ---

    private PlacedBlock getBlockById(int id) {
        for (PlacedBlock block : placedBlocks) {
            if (block.id == id) return block;
        }
        return null;
    }

    private boolean isInputConnected(int blockId, String portName) {
        for (Connection conn : connections) {
            if (conn.toBlockId == blockId && conn.toPort.equals(portName)) return true;
        }
        return false;
    }

    private void removeConnectionsAt(int blockId, String portName, boolean isOutput) {
        if (isOutput) {
            connections.removeIf(c -> c.fromBlockId == blockId && c.fromPort.equals(portName));
        } else {
            connections.removeIf(c -> c.toBlockId == blockId && c.toPort.equals(portName));
        }
    }

    private PortDef findPortDef(PlacedBlock block, String portName, boolean isOutput) {
        List<PortDef> ports = isOutput ? block.definition.outputs() : block.definition.inputs();
        for (PortDef p : ports) {
            if (p.name().equals(portName)) return p;
        }
        return null;
    }

    private static class PortHitResult {
        final PlacedBlock block;
        final PortDef portDef;
        final boolean isOutput;

        PortHitResult(PlacedBlock block, PortDef portDef, boolean isOutput) {
            this.block = block;
            this.portDef = portDef;
            this.isOutput = isOutput;
        }
    }

    private static class InputFieldHit {
        final PlacedBlock block;
        final PortDef portDef;

        InputFieldHit(PlacedBlock block, PortDef portDef) {
            this.block = block;
            this.portDef = portDef;
        }
    }

    /** Check if the click lands inside any input field. Checked before port hit test. */
    private InputFieldHit findInputFieldAt(double mouseX, double mouseY) {
        for (int i = placedBlocks.size() - 1; i >= 0; i--) {
            PlacedBlock block = placedBlocks.get(i);
            for (PortDef port : block.definition.dataInputs()) {
                if (!isInputConnected(block.id, port.name())) {
                    if (isClickInInputField(block, port, mouseX, mouseY)) {
                        return new InputFieldHit(block, port);
                    }
                }
            }
        }
        return null;
    }

    private PortHitResult findPortAt(double mouseX, double mouseY) {
        float hitRadius = PORT_HIT_RADIUS * canvasZoom;

        for (int i = placedBlocks.size() - 1; i >= 0; i--) {
            PlacedBlock block = placedBlocks.get(i);

            // Check flow inputs
            for (PortDef fp : block.definition.flowInputs()) {
                float[] pos = getPortScreenPos(block, fp.name(), false);
                if (Math.abs(mouseX - pos[0]) < hitRadius && Math.abs(mouseY - pos[1]) < hitRadius) {
                    return new PortHitResult(block, fp, false);
                }
            }
            // Check flow outputs
            for (PortDef fp : block.definition.flowOutputs()) {
                float[] pos = getPortScreenPos(block, fp.name(), true);
                if (Math.abs(mouseX - pos[0]) < hitRadius && Math.abs(mouseY - pos[1]) < hitRadius) {
                    return new PortHitResult(block, fp, true);
                }
            }
            // Check data inputs
            for (PortDef port : block.definition.dataInputs()) {
                float[] pos = getPortScreenPos(block, port.name(), false);
                if (Math.abs(mouseX - pos[0]) < hitRadius && Math.abs(mouseY - pos[1]) < hitRadius) {
                    return new PortHitResult(block, port, false);
                }
            }
            // Check data outputs
            for (PortDef port : block.definition.dataOutputs()) {
                float[] pos = getPortScreenPos(block, port.name(), true);
                if (Math.abs(mouseX - pos[0]) < hitRadius && Math.abs(mouseY - pos[1]) < hitRadius) {
                    return new PortHitResult(block, port, true);
                }
            }
        }
        return null;
    }

    private boolean isClickInInputField(PlacedBlock block, PortDef port, double mouseX, double mouseY) {
        float[] portPos = getPortScreenPos(block, port.name(), false);
        float labelX = portPos[0] + PORT_RADIUS * canvasZoom + 3 * canvasZoom;
        float fieldX = labelX + this.font.width(port.name()) * canvasZoom + 4 * canvasZoom;
        float fieldY = portPos[1] - INPUT_FIELD_HEIGHT * canvasZoom / 2;
        float fw = INPUT_FIELD_WIDTH * canvasZoom;
        float fh = INPUT_FIELD_HEIGHT * canvasZoom;

        return mouseX >= fieldX && mouseX <= fieldX + fw && mouseY >= fieldY && mouseY <= fieldY + fh;
    }

    private void startEditing(PlacedBlock block, PortDef port) {
        if (editingBlock != null) {
            commitEditing();
        }
        editingBlock = block;
        editingPort = port.name();
        editingValue = block.inputValues.getOrDefault(port.name(),
                port.defaultValue() != null ? port.defaultValue() : "");
        editCursorPos = editingValue.length();
    }

    private void commitEditing() {
        if (editingBlock != null && editingPort != null) {
            editingBlock.inputValues.put(editingPort, editingValue);
        }
        editingBlock = null;
        editingPort = null;
    }

    private int getBlockWidthForDef(BlockDef def) {
        int labelWidth = this.font.width(def.label()) + 20;
        return Math.max(BLOCK_MIN_WIDTH, labelWidth);
    }

    private BlockDef getPaletteBlockAt(double mouseX, double mouseY) {
        int y = PALETTE_PADDING + 14 - paletteScrollOffset;
        int blockWidth = PALETTE_WIDTH - PALETTE_PADDING * 2;

        for (Category category : VisualBlockRegistry.getCategories()) {
            y += 6 + CATEGORY_HEADER_HEIGHT;
            for (BlockDef block : category.blocks()) {
                if (mouseX >= PALETTE_PADDING && mouseX <= PALETTE_PADDING + blockWidth
                        && mouseY >= y && mouseY <= y + PALETTE_BLOCK_HEIGHT) {
                    return block;
                }
                y += PALETTE_BLOCK_HEIGHT + BLOCK_PADDING;
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

    private static int getPortColor(String type) {
        return switch (type) {
            case "flow" -> FLOW_PORT_COLOR;
            case "string" -> STRING_PORT_COLOR;
            case "number" -> NUMBER_PORT_COLOR;
            default -> ANY_PORT_COLOR;
        };
    }

    // --- Inner classes ---

    private static class PlacedBlock {
        final int id;
        final BlockDef definition;
        final int color;
        float x, y;
        final Map<String, String> inputValues = new HashMap<>();

        PlacedBlock(int id, BlockDef definition, int color, float x, float y) {
            this.id = id;
            this.definition = definition;
            this.color = color;
            this.x = x;
            this.y = y;

            // Initialize input values from defaults
            for (PortDef input : definition.dataInputs()) {
                if (input.defaultValue() != null) {
                    inputValues.put(input.name(), input.defaultValue());
                }
            }
        }
    }

    private record Connection(int fromBlockId, String fromPort, int toBlockId, String toPort) {}
}
