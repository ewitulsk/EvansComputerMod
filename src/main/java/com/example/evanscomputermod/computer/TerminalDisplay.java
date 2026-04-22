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
    /** Magic number for a valid graphics framebuffer header. */
    public static final int GFX_MAGIC = 0xFB02;
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

    // GFX header field offsets (from start of gfx data)
    public static final int GFX_OFF_MAGIC = 0x00;
    public static final int GFX_OFF_MODE = 0x02;
    public static final int GFX_OFF_WIDTH = 0x04;
    public static final int GFX_OFF_HEIGHT = 0x06;
    public static final int GFX_OFF_PALETTE_DIRTY = 0x08;
    public static final int GFX_OFF_PIXEL_DIRTY = 0x0C;
    public static final int GFX_OFF_PIXEL_FORMAT = 0x10;
    public static final int GFX_PALETTE_OFF = 0x40;
    public static final int GFX_PIXEL_OFF = 0x400;

    /** Pixel format: 1 byte per pixel, indexed into the 256-entry RGB palette. */
    public static final int PIXEL_FORMAT_INDEXED8 = 0;
    /** Pixel format: 4 bytes per pixel, packed RGBA8888 (no palette). */
    public static final int PIXEL_FORMAT_RGBA8888 = 1;

    private int width;
    private int height;
    private int cursorX;
    private int cursorY;
    private boolean cursorVisible;
    private int dirtyCounter;
    private byte[] cellData; // width * height * CELL_SIZE bytes

    // Graphics framebuffer state. All gfx mutators are `synchronized` on
    // `this`, so external sync-path callers can hold `synchronized(display)`
    // across a snapshot + diff + commit and see a consistent frame even
    // when a WASI child bridge or kernel worker is mid-write.
    private int displayMode = 0;   // 0=text, 1=gfx, 2=overlay
    private int gfxWidth = 0;
    private int gfxHeight = 0;
    private int pixelFormat = PIXEL_FORMAT_INDEXED8;
    private int[] palette = new int[256];   // ARGB format (INDEXED8 only)
    /**
     * Pixel storage. Length depends on {@link #pixelFormat}:
     * <ul>
     *   <li>INDEXED8: gfxWidth × gfxHeight bytes of palette indices</li>
     *   <li>RGBA8888: gfxWidth × gfxHeight × 4 bytes of packed RGBA</li>
     * </ul>
     */
    private byte[] pixelData = null;
    private int paletteDirtyCounter = 0;
    private int pixelDirtyCounter = 0;

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

    public void setCursorX(int x) { this.cursorX = x; }
    public void setCursorY(int y) { this.cursorY = y; }
    public void setCursorVisible(boolean visible) { this.cursorVisible = visible; }

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
    public synchronized void setFromBytes(byte[] data) {
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

    // --- Graphics framebuffer methods ---

    @Override
    public synchronized int getDisplayMode() { return displayMode; }

    public synchronized void setDisplayMode(int mode) { this.displayMode = mode; }

    /** Set dirty counters to specific values (for client-side monotonic tracking). */
    public synchronized void setGfxDirtyCounters(int pixDirty, int palDirty) {
        this.pixelDirtyCounter = pixDirty;
        this.paletteDirtyCounter = palDirty;
    }

    @Override
    public synchronized int getGfxWidth() { return gfxWidth; }

    @Override
    public synchronized int getGfxHeight() { return gfxHeight; }

    @Override
    public synchronized int[] getPalette() { return palette; }

    @Override
    public synchronized byte[] getPixelData() { return pixelData; }

    public synchronized int getPaletteDirtyCounter() { return paletteDirtyCounter; }

    public synchronized int getPixelDirtyCounter() { return pixelDirtyCounter; }

    /** Current pixel format ({@link #PIXEL_FORMAT_INDEXED8} or {@link #PIXEL_FORMAT_RGBA8888}). */
    public synchronized int getPixelFormat() { return pixelFormat; }

    public synchronized void setPixelFormat(int format) { this.pixelFormat = format; }

    /** Bytes per pixel for the current format: 1 (indexed8) or 4 (rgba8888). */
    public synchronized int getBytesPerPixel() {
        return pixelFormat == PIXEL_FORMAT_RGBA8888 ? 4 : 1;
    }

    /**
     * Parse graphics framebuffer data read from WASM memory at GFX_BASE.
     * The data starts at offset 0 = the GFX header.
     *
     * <p>Synchronised on {@code this} so that the server-tick sync path can
     * snapshot the palette/pixel arrays atomically under {@code synchronized(display)}
     * while the WASM worker thread is mid-write. Without this guard the tile
     * diff produced spurious "tile changed" hits from torn reads.
     */
    public synchronized void setGfxFromBytes(byte[] data) {
        if (data == null || data.length < GFX_PALETTE_OFF) return;

        ByteBuffer buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);

        int magic = buf.getShort(GFX_OFF_MAGIC) & 0xFFFF;
        if (magic != GFX_MAGIC) {
            this.displayMode = 0;
            return;
        }

        this.displayMode = data[GFX_OFF_MODE] & 0xFF;
        this.gfxWidth = buf.getShort(GFX_OFF_WIDTH) & 0xFFFF;
        this.gfxHeight = buf.getShort(GFX_OFF_HEIGHT) & 0xFFFF;
        this.paletteDirtyCounter = buf.getInt(GFX_OFF_PALETTE_DIRTY);
        this.pixelDirtyCounter = buf.getInt(GFX_OFF_PIXEL_DIRTY);
        this.pixelFormat = data[GFX_OFF_PIXEL_FORMAT] & 0xFF;

        // Read palette (256 RGB entries -> ARGB). Always read regardless of
        // format so a switch back to INDEXED8 has the right palette ready.
        int paletteEnd = GFX_PALETTE_OFF + 768;
        if (data.length >= paletteEnd) {
            for (int i = 0; i < 256; i++) {
                int off = GFX_PALETTE_OFF + i * 3;
                int r = data[off] & 0xFF;
                int g = data[off + 1] & 0xFF;
                int b = data[off + 2] & 0xFF;
                palette[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
            }
        }

        // Read pixel data: 1 byte/pixel for indexed, 4 bytes/pixel for rgba.
        int bpp = pixelFormat == PIXEL_FORMAT_RGBA8888 ? 4 : 1;
        int pixelCount = gfxWidth * gfxHeight;
        int pixelBytes = pixelCount * bpp;
        int pixelEnd = GFX_PIXEL_OFF + pixelBytes;
        if (pixelBytes > 0 && data.length >= pixelEnd) {
            if (pixelData == null || pixelData.length != pixelBytes) {
                pixelData = new byte[pixelBytes];
            }
            System.arraycopy(data, GFX_PIXEL_OFF, pixelData, 0, pixelBytes);
        }
    }

    /**
     * Snapshot this display's graphics state into caller-provided scratch
     * buffers under the same monitor as {@link #setGfxFromBytes}. Used by the
     * server-tick screen sync path so the diff/commit runs against a private
     * copy and can't race the WASM worker thread.
     *
     * @param outPalette destination for the 256-entry ARGB palette (must be ≥256 long)
     * @param outPixels  destination for pixel data (sized ≥ gfxWidth*gfxHeight); may
     *                   be {@code null} or under-sized — caller should pre-size using
     *                   the width/height returned via {@code outDims}
     * @param outDims    two-entry array, receives {@code [gfxWidth, gfxHeight]}
     * @return the current display mode
     */
    public synchronized int snapshotGfx(int[] outDims, byte[] outPixels, int[] outPalette) {
        outDims[0] = gfxWidth;
        outDims[1] = gfxHeight;
        if (outPalette != null && outPalette.length >= palette.length) {
            System.arraycopy(palette, 0, outPalette, 0, palette.length);
        }
        int bpp = pixelFormat == PIXEL_FORMAT_RGBA8888 ? 4 : 1;
        int pixBytes = gfxWidth * gfxHeight * bpp;
        if (outPixels != null && outPixels.length >= pixBytes && pixelData != null
                && pixelData.length >= pixBytes) {
            System.arraycopy(pixelData, 0, outPixels, 0, pixBytes);
        }
        return displayMode;
    }

    /**
     * Serialize the graphics framebuffer state for network transmission.
     * Returns null if no graphics are active.
     */
    public synchronized byte[] gfxToBytes() {
        if (displayMode == 0 || gfxWidth == 0 || gfxHeight == 0) return null;

        int bpp = pixelFormat == PIXEL_FORMAT_RGBA8888 ? 4 : 1;
        int pixelBytes = gfxWidth * gfxHeight * bpp;
        int totalSize = GFX_PIXEL_OFF + pixelBytes;
        byte[] result = new byte[totalSize];
        ByteBuffer buf = ByteBuffer.wrap(result).order(ByteOrder.LITTLE_ENDIAN);

        buf.putShort(GFX_OFF_MAGIC, (short) GFX_MAGIC);
        result[GFX_OFF_MODE] = (byte) displayMode;
        buf.putShort(GFX_OFF_WIDTH, (short) gfxWidth);
        buf.putShort(GFX_OFF_HEIGHT, (short) gfxHeight);
        buf.putInt(GFX_OFF_PALETTE_DIRTY, paletteDirtyCounter);
        buf.putInt(GFX_OFF_PIXEL_DIRTY, pixelDirtyCounter);
        result[GFX_OFF_PIXEL_FORMAT] = (byte) pixelFormat;

        // Write palette (ARGB -> RGB). Always written so the destination
        // can switch between formats without losing the palette.
        for (int i = 0; i < 256; i++) {
            int off = GFX_PALETTE_OFF + i * 3;
            int argb = palette[i];
            result[off] = (byte) ((argb >> 16) & 0xFF);     // R
            result[off + 1] = (byte) ((argb >> 8) & 0xFF);  // G
            result[off + 2] = (byte) (argb & 0xFF);          // B
        }

        // Write pixel data
        if (pixelData != null && pixelData.length == pixelBytes) {
            System.arraycopy(pixelData, 0, result, GFX_PIXEL_OFF, pixelBytes);
        }

        return result;
    }
}
