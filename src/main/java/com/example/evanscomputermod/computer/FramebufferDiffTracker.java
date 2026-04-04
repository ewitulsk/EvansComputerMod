package com.example.evanscomputermod.computer;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.zip.CRC32;

/**
 * Shadow buffer diff engine — compares current display state against a shadow copy
 * to produce minimal deltas for network transmission.
 *
 * Supports:
 * - Row-level text diffing with scroll detection
 * - Tile-based (16×16) graphics pixel diffing
 * - Palette change detection
 */
public class FramebufferDiffTracker {

    private static final int TILE_SIZE = 16;

    // Shadow state (last-sent to this client)
    private byte[] shadowTextCells;
    private byte[] shadowPixelData;
    private int[] shadowPalette;
    private int shadowDisplayMode;
    private int shadowCursorX, shadowCursorY;
    private boolean shadowCursorVisible;
    private int shadowTextWidth, shadowTextHeight;
    private int shadowGfxWidth, shadowGfxHeight;

    // Generation counter
    private long generation = 0;

    // Row hashes for scroll detection
    private long[] shadowRowHashes;

    public FramebufferDiffTracker() {
    }

    /** Initialize shadow from a display snapshot. */
    public void initShadow(TerminalDisplay display) {
        shadowTextWidth = display.getWidth();
        shadowTextHeight = display.getHeight();
        shadowTextCells = display.getCellData().clone();
        shadowCursorX = display.getCursorX();
        shadowCursorY = display.getCursorY();
        shadowCursorVisible = display.isCursorVisible();
        shadowDisplayMode = display.getDisplayMode();
        shadowGfxWidth = display.getGfxWidth();
        shadowGfxHeight = display.getGfxHeight();

        int[] pal = display.getPalette();
        shadowPalette = pal != null ? pal.clone() : new int[256];

        byte[] pix = display.getPixelData();
        shadowPixelData = pix != null ? pix.clone() : null;

        shadowRowHashes = computeRowHashes(shadowTextCells, shadowTextWidth, shadowTextHeight);
    }

    public long getGeneration() { return generation; }

    /**
     * Compute text delta between current display and shadow.
     * Detects scroll offset and returns only changed rows.
     */
    public TextDelta computeTextDelta(TerminalDisplay display) {
        generation++;

        int width = display.getWidth();
        int height = display.getHeight();
        byte[] cells = display.getCellData();

        // If dimensions changed, everything is dirty
        if (width != shadowTextWidth || height != shadowTextHeight) {
            List<Integer> allRows = new ArrayList<>(height);
            List<byte[]> allData = new ArrayList<>(height);
            int rowBytes = width * TerminalDisplay.CELL_SIZE;
            for (int r = 0; r < height; r++) {
                allRows.add(r);
                byte[] row = new byte[rowBytes];
                System.arraycopy(cells, r * rowBytes, row, 0, rowBytes);
                allData.add(row);
            }
            return new TextDelta(generation, 0, allRows, allData,
                    display.getCursorX(), display.getCursorY(), display.isCursorVisible());
        }

        int rowBytes = width * TerminalDisplay.CELL_SIZE;

        // Scroll detection: compute row hashes and check for shift
        long[] currentHashes = computeRowHashes(cells, width, height);
        int scrollOffset = detectScroll(shadowRowHashes, currentHashes, height);

        // Find changed rows (accounting for scroll)
        List<Integer> changedRows = new ArrayList<>();
        List<byte[]> changedData = new ArrayList<>();

        for (int r = 0; r < height; r++) {
            int shadowRow = r + scrollOffset; // which shadow row maps to current row r
            boolean rowChanged;

            if (scrollOffset != 0 && shadowRow >= 0 && shadowRow < height) {
                // After scroll, compare current row r with shadow row (r + scrollOffset)
                int curOff = r * rowBytes;
                int shadOff = shadowRow * rowBytes;
                rowChanged = Arrays.mismatch(cells, curOff, curOff + rowBytes,
                        shadowTextCells, shadOff, shadOff + rowBytes) >= 0;
            } else if (scrollOffset != 0) {
                // New row that scrolled in
                rowChanged = true;
            } else {
                // No scroll: compare same row
                int off = r * rowBytes;
                rowChanged = Arrays.mismatch(cells, off, off + rowBytes,
                        shadowTextCells, off, off + rowBytes) >= 0;
            }

            if (rowChanged) {
                changedRows.add(r);
                byte[] row = new byte[rowBytes];
                System.arraycopy(cells, r * rowBytes, row, 0, rowBytes);
                changedData.add(row);
            }
        }

        return new TextDelta(generation, scrollOffset, changedRows, changedData,
                display.getCursorX(), display.getCursorY(), display.isCursorVisible());
    }

    /**
     * Compute graphics delta using 16×16 tile-based comparison.
     */
    public GfxDelta computeGfxDelta(TerminalDisplay display) {
        int gfxW = display.getGfxWidth();
        int gfxH = display.getGfxHeight();
        byte[] pixels = display.getPixelData();
        int[] palette = display.getPalette();

        // Check palette changes
        boolean paletteChanged = false;
        if (palette != null && shadowPalette != null) {
            paletteChanged = !Arrays.equals(palette, shadowPalette);
        } else if (palette != null || shadowPalette != null) {
            paletteChanged = true;
        }

        // If dimensions changed, send everything
        if (gfxW != shadowGfxWidth || gfxH != shadowGfxHeight || shadowPixelData == null || pixels == null) {
            // Full gfx update
            int tilesX = (gfxW + TILE_SIZE - 1) / TILE_SIZE;
            int tilesY = (gfxH + TILE_SIZE - 1) / TILE_SIZE;
            List<Integer> allTiles = new ArrayList<>();
            List<byte[]> allData = new ArrayList<>();
            if (pixels != null) {
                for (int t = 0; t < tilesX * tilesY; t++) {
                    allTiles.add(t);
                    allData.add(extractTile(pixels, gfxW, gfxH, t, tilesX));
                }
            }
            return new GfxDelta(generation, paletteChanged, paletteChanged ? palette : null,
                    allTiles, allData);
        }

        // Tile-based diffing
        int tilesX = (gfxW + TILE_SIZE - 1) / TILE_SIZE;
        int tilesY = (gfxH + TILE_SIZE - 1) / TILE_SIZE;
        int totalTiles = tilesX * tilesY;

        List<Integer> changedTiles = new ArrayList<>();
        List<byte[]> changedData = new ArrayList<>();

        for (int t = 0; t < totalTiles; t++) {
            byte[] currentTile = extractTile(pixels, gfxW, gfxH, t, tilesX);
            byte[] shadowTile = extractTile(shadowPixelData, gfxW, gfxH, t, tilesX);

            if (!Arrays.equals(currentTile, shadowTile)) {
                changedTiles.add(t);
                changedData.add(currentTile);
            }
        }

        return new GfxDelta(generation, paletteChanged, paletteChanged ? palette : null,
                changedTiles, changedData);
    }

    /** Commit current display state to shadow (call after sending delta). */
    public void commitShadow(TerminalDisplay display) {
        shadowTextWidth = display.getWidth();
        shadowTextHeight = display.getHeight();
        shadowTextCells = display.getCellData().clone();
        shadowCursorX = display.getCursorX();
        shadowCursorY = display.getCursorY();
        shadowCursorVisible = display.isCursorVisible();
        shadowDisplayMode = display.getDisplayMode();
        shadowGfxWidth = display.getGfxWidth();
        shadowGfxHeight = display.getGfxHeight();

        int[] pal = display.getPalette();
        shadowPalette = pal != null ? pal.clone() : shadowPalette;

        byte[] pix = display.getPixelData();
        shadowPixelData = pix != null ? pix.clone() : null;

        shadowRowHashes = computeRowHashes(shadowTextCells, shadowTextWidth, shadowTextHeight);
    }

    // --- Scroll detection ---

    private static long[] computeRowHashes(byte[] cells, int width, int height) {
        int rowBytes = width * TerminalDisplay.CELL_SIZE;
        long[] hashes = new long[height];
        CRC32 crc = new CRC32();
        for (int r = 0; r < height; r++) {
            crc.reset();
            crc.update(cells, r * rowBytes, rowBytes);
            hashes[r] = crc.getValue();
        }
        return hashes;
    }

    /**
     * Detect scroll offset by comparing row hash sequences.
     * Returns positive for scroll-up (content moved up), negative for scroll-down.
     * Returns 0 if no scroll detected.
     */
    private static int detectScroll(long[] oldHashes, long[] newHashes, int height) {
        if (oldHashes == null || oldHashes.length != height) return 0;

        // Try small scroll offsets (1-5 rows) — most common case
        for (int offset = 1; offset <= Math.min(5, height / 2); offset++) {
            // Check scroll up by 'offset'
            int matchUp = 0;
            for (int r = 0; r < height - offset; r++) {
                if (newHashes[r] == oldHashes[r + offset]) matchUp++;
            }
            if (matchUp >= (height - offset) * 80 / 100) {
                return offset; // scroll up
            }

            // Check scroll down by 'offset'
            int matchDown = 0;
            for (int r = offset; r < height; r++) {
                if (newHashes[r] == oldHashes[r - offset]) matchDown++;
            }
            if (matchDown >= (height - offset) * 80 / 100) {
                return -offset; // scroll down
            }
        }

        return 0;
    }

    // --- Tile extraction ---

    private static byte[] extractTile(byte[] pixels, int width, int height, int tileIdx, int tilesPerRow) {
        int tileX = (tileIdx % tilesPerRow) * TILE_SIZE;
        int tileY = (tileIdx / tilesPerRow) * TILE_SIZE;
        byte[] tile = new byte[TILE_SIZE * TILE_SIZE];

        for (int py = 0; py < TILE_SIZE; py++) {
            int srcY = tileY + py;
            if (srcY >= height) break;
            for (int px = 0; px < TILE_SIZE; px++) {
                int srcX = tileX + px;
                if (srcX >= width) break;
                tile[py * TILE_SIZE + px] = pixels[srcY * width + srcX];
            }
        }
        return tile;
    }

    // --- Delta records ---

    public record TextDelta(
            long generation,
            int scrollOffset,
            List<Integer> changedRowIndices,
            List<byte[]> changedRowData,
            int cursorX, int cursorY, boolean cursorVisible
    ) {}

    public record GfxDelta(
            long generation,
            boolean paletteChanged,
            int[] palette,
            List<Integer> changedTileIndices,
            List<byte[]> changedTileData
    ) {}
}
