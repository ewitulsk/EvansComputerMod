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

    public int getShadowDisplayMode() { return shadowDisplayMode; }

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
     * Compute graphics delta using 16×16 tile-based comparison. Delegates to
     * the raw-array variant after reading fields from {@code display}. Callers
     * that can snapshot the pixel/palette buffers under a lock should call the
     * raw variant directly to avoid cross-thread races.
     */
    public GfxDelta computeGfxDelta(TerminalDisplay display) {
        return computeGfxDelta(display.getGfxWidth(), display.getGfxHeight(),
                display.getPixelData(), display.getPalette(), display.getDisplayMode());
    }

    /**
     * Compute graphics delta from raw snapshotted arrays. Used by the screen
     * cluster sync path, which snapshots the live pixel/palette arrays under
     * {@code synchronized(screenDisplay)} so the worker thread can't write
     * them mid-diff.
     *
     * <p>Hot-path notes:
     * <ul>
     *   <li>If {@code pixels} is exactly equal to the shadow (common during
     *       the gfxtest palette-animation phase: palette cycles but pixels are
     *       static), a single {@link Arrays#mismatch} call short-circuits the
     *       tile loop. This is a vectorised JDK intrinsic — ~100× faster than
     *       the per-tile compare that preceded this fix.</li>
     *   <li>Otherwise the tile loop walks both buffers in place and only
     *       allocates a fresh {@code byte[256]} for tiles that actually
     *       differ.</li>
     * </ul>
     */
    public GfxDelta computeGfxDelta(int gfxW, int gfxH, byte[] pixels, int[] palette, int displayMode) {
        // Check palette changes. Clone the palette into the delta when it
        // changed so callers can reuse the palette scratch buffer in place
        // next tick without mutating an in-flight packet.
        boolean paletteChanged = false;
        if (palette != null && shadowPalette != null) {
            paletteChanged = !Arrays.equals(palette, shadowPalette);
        } else if (palette != null || shadowPalette != null) {
            paletteChanged = true;
        }
        int[] deltaPalette = (paletteChanged && palette != null) ? palette.clone() : null;

        // If dimensions or display mode changed, send everything.
        boolean modeChanged = displayMode != shadowDisplayMode;
        if (gfxW != shadowGfxWidth || gfxH != shadowGfxHeight
                || shadowPixelData == null || pixels == null || modeChanged) {
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
            return new GfxDelta(generation, paletteChanged, deltaPalette, allTiles, allData);
        }

        // Fast path: pixels are byte-identical to the shadow. Used every
        // frame during the gfxtest palette-animation phase.
        if (pixels.length == shadowPixelData.length
                && Arrays.mismatch(pixels, shadowPixelData) < 0) {
            return new GfxDelta(generation, paletteChanged, deltaPalette,
                    java.util.Collections.emptyList(), java.util.Collections.emptyList());
        }

        // Tile-based diffing — walk both buffers in place; only allocate
        // tile copies for tiles that differ.
        int tilesX = (gfxW + TILE_SIZE - 1) / TILE_SIZE;
        int tilesY = (gfxH + TILE_SIZE - 1) / TILE_SIZE;
        int totalTiles = tilesX * tilesY;

        List<Integer> changedTiles = new ArrayList<>();
        List<byte[]> changedData = new ArrayList<>();

        for (int t = 0; t < totalTiles; t++) {
            int tileX = (t % tilesX) * TILE_SIZE;
            int tileY = (t / tilesX) * TILE_SIZE;
            if (tileDiffersInPlace(pixels, shadowPixelData, gfxW, gfxH, tileX, tileY)) {
                changedTiles.add(t);
                changedData.add(extractTile(pixels, gfxW, gfxH, t, tilesX));
            }
        }

        return new GfxDelta(generation, paletteChanged, deltaPalette, changedTiles, changedData);
    }

    /**
     * Row-by-row {@link Arrays#mismatch} comparison of a tile against the
     * shadow. Skips the per-row copy that {@link #extractTile} would allocate.
     */
    private static boolean tileDiffersInPlace(byte[] a, byte[] b,
                                               int width, int height,
                                               int tileX, int tileY) {
        int maxY = Math.min(tileY + TILE_SIZE, height);
        int maxX = Math.min(tileX + TILE_SIZE, width);
        for (int py = tileY; py < maxY; py++) {
            int rowStart = py * width + tileX;
            int rowEnd = py * width + maxX;
            if (Arrays.mismatch(a, rowStart, rowEnd, b, rowStart, rowEnd) >= 0) {
                return true;
            }
        }
        return false;
    }

    /**
     * Commit current display state to shadow (call after sending delta). Reuses
     * existing shadow buffers when sizes match — a steady-state commit performs
     * only {@code System.arraycopy} calls, no {@code .clone()} allocations.
     * Previously this allocated ~70 KB per commit (pixels + cells + palette)
     * which at 20 Hz added ~1.4 MB/s of GC churn per synced player.
     */
    public void commitShadow(TerminalDisplay display) {
        commitShadowText(display);
        commitShadowGfx(display.getGfxWidth(), display.getGfxHeight(),
                display.getPixelData(), display.getPalette(), display.getDisplayMode());
    }

    /**
     * Commit text-related shadow state from {@code display}. Called on its own
     * by the screen cluster sync path, whose text cells are static after init
     * so this path is race-free.
     */
    public void commitShadowText(TerminalDisplay display) {
        int textW = display.getWidth();
        int textH = display.getHeight();
        byte[] cells = display.getCellData();

        shadowTextWidth = textW;
        shadowTextHeight = textH;
        if (cells != null) {
            if (shadowTextCells == null || shadowTextCells.length != cells.length) {
                shadowTextCells = cells.clone();
            } else {
                System.arraycopy(cells, 0, shadowTextCells, 0, cells.length);
            }
        }
        shadowCursorX = display.getCursorX();
        shadowCursorY = display.getCursorY();
        shadowCursorVisible = display.isCursorVisible();

        if (shadowTextCells != null) {
            if (shadowRowHashes == null || shadowRowHashes.length != textH) {
                shadowRowHashes = new long[textH];
            }
            computeRowHashesInto(shadowTextCells, textW, textH, shadowRowHashes);
        }
    }

    /**
     * Commit graphics shadow state from raw snapshotted arrays. Used by the
     * screen cluster sync path so the commit doesn't re-read the racy
     * {@link TerminalDisplay} pixel/palette arrays after the worker thread
     * may have written them.
     */
    public void commitShadowGfx(int gfxW, int gfxH, byte[] pixels, int[] palette, int displayMode) {
        shadowDisplayMode = displayMode;
        shadowGfxWidth = gfxW;
        shadowGfxHeight = gfxH;

        if (palette != null) {
            if (shadowPalette == null || shadowPalette.length != palette.length) {
                shadowPalette = palette.clone();
            } else {
                System.arraycopy(palette, 0, shadowPalette, 0, palette.length);
            }
        }

        if (pixels != null) {
            if (shadowPixelData == null || shadowPixelData.length != pixels.length) {
                shadowPixelData = pixels.clone();
            } else {
                System.arraycopy(pixels, 0, shadowPixelData, 0, pixels.length);
            }
        } else {
            shadowPixelData = null;
        }
    }

    // --- Scroll detection ---

    private static long[] computeRowHashes(byte[] cells, int width, int height) {
        long[] hashes = new long[height];
        computeRowHashesInto(cells, width, height, hashes);
        return hashes;
    }

    private static void computeRowHashesInto(byte[] cells, int width, int height, long[] out) {
        int rowBytes = width * TerminalDisplay.CELL_SIZE;
        CRC32 crc = new CRC32();
        for (int r = 0; r < height; r++) {
            crc.reset();
            crc.update(cells, r * rowBytes, rowBytes);
            out[r] = crc.getValue();
        }
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
