package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IFramebufferDisplay;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Framebuffer display state — stores the cell buffer read from WASM memory.
 * <p>
 * The WASM OS writes to a memory-mapped framebuffer (64-byte header + cell data).
 * This class stores a copy of that data for rendering and network sync.
 * </p>
 * <p>
 * Each cell is 4 bytes: [character, attribute, flags, reserved].
 * Attribute byte: low nibble = fg color (0-15), high nibble = bg color (0-15).
 * </p>
 */
public class TerminalDisplay implements IFramebufferDisplay {

    /** Header size in bytes. */
    public static final int HEADER_SIZE = 64;
    /** Bytes per cell. */
    public static final int CELL_SIZE = 4;
    /** Magic number for a valid framebuffer header. */
    public static final int FB_MAGIC = 0xFB01;
    /** Default attribute: bright green (10) on black (0). */
    public static final byte DEFAULT_ATTR = 0x0A;

    // Header field offsets
    private static final int OFF_MAGIC = 0x00;
    private static final int OFF_WIDTH = 0x02;
    private static final int OFF_HEIGHT = 0x04;
    private static final int OFF_CURSOR_X = 0x06;
    private static final int OFF_CURSOR_Y = 0x08;
    private static final int OFF_CURSOR_VISIBLE = 0x0A;
    private static final int OFF_DIRTY_COUNTER = 0x0C;

    private int width;
    private int height;
    private int cursorX;
    private int cursorY;
    private boolean cursorVisible;
    private int dirtyCounter;
    private byte[] cellData; // width * height * CELL_SIZE bytes

    public TerminalDisplay() {
        this(160, 50);
    }

    public TerminalDisplay(int width, int height) {
        this.width = width;
        this.height = height;
        this.cursorX = 0;
        this.cursorY = 0;
        this.cursorVisible = true;
        this.dirtyCounter = 0;
        this.cellData = new byte[width * height * CELL_SIZE];
        // Initialize all cells to space + default attr
        for (int i = 0; i < width * height; i++) {
            int off = i * CELL_SIZE;
            cellData[off] = (byte) ' ';
            cellData[off + 1] = DEFAULT_ATTR;
            cellData[off + 2] = 0;
            cellData[off + 3] = 0;
        }
    }

    @Override
    public int getWidth() {
        return width;
    }

    @Override
    public int getHeight() {
        return height;
    }

    @Override
    public int getCursorX() {
        return cursorX;
    }

    @Override
    public int getCursorY() {
        return cursorY;
    }

    @Override
    public boolean isCursorVisible() {
        return cursorVisible;
    }

    public int getDirtyCounter() {
        return dirtyCounter;
    }

    @Override
    public byte[] getCellData() {
        return cellData;
    }

    @Override
    public void setFromBytes(byte[] data) {
        if (data == null || data.length < HEADER_SIZE) return;

        ByteBuffer buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);

        int magic = buf.getShort(OFF_MAGIC) & 0xFFFF;
        if (magic != FB_MAGIC) return;

        this.width = buf.getShort(OFF_WIDTH) & 0xFFFF;
        this.height = buf.getShort(OFF_HEIGHT) & 0xFFFF;
        this.cursorX = buf.getShort(OFF_CURSOR_X) & 0xFFFF;
        this.cursorY = buf.getShort(OFF_CURSOR_Y) & 0xFFFF;
        this.cursorVisible = data[OFF_CURSOR_VISIBLE] != 0;
        this.dirtyCounter = buf.getInt(OFF_DIRTY_COUNTER);

        int cellBytes = width * height * CELL_SIZE;
        int expectedLen = HEADER_SIZE + cellBytes;
        if (data.length >= expectedLen) {
            this.cellData = new byte[cellBytes];
            System.arraycopy(data, HEADER_SIZE, this.cellData, 0, cellBytes);
        }
    }

    /**
     * Serialize the display to a byte array (header + cells) for network/NBT.
     */
    public byte[] toBytes() {
        int cellBytes = width * height * CELL_SIZE;
        byte[] result = new byte[HEADER_SIZE + cellBytes];
        ByteBuffer buf = ByteBuffer.wrap(result).order(ByteOrder.LITTLE_ENDIAN);

        buf.putShort(OFF_MAGIC, (short) FB_MAGIC);
        buf.putShort(OFF_WIDTH, (short) width);
        buf.putShort(OFF_HEIGHT, (short) height);
        buf.putShort(OFF_CURSOR_X, (short) cursorX);
        buf.putShort(OFF_CURSOR_Y, (short) cursorY);
        result[OFF_CURSOR_VISIBLE] = (byte) (cursorVisible ? 1 : 0);
        buf.putInt(OFF_DIRTY_COUNTER, dirtyCounter);

        System.arraycopy(cellData, 0, result, HEADER_SIZE, cellBytes);
        return result;
    }

    @Override
    public byte getCharAt(int x, int y) {
        if (x < 0 || x >= width || y < 0 || y >= height) return (byte) ' ';
        return cellData[(y * width + x) * CELL_SIZE];
    }

    @Override
    public byte getAttrAt(int x, int y) {
        if (x < 0 || x >= width || y < 0 || y >= height) return DEFAULT_ATTR;
        return cellData[(y * width + x) * CELL_SIZE + 1];
    }

    @Override
    public byte getFlagsAt(int x, int y) {
        if (x < 0 || x >= width || y < 0 || y >= height) return 0;
        return cellData[(y * width + x) * CELL_SIZE + 2];
    }
}
