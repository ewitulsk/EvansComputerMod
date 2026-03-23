package com.example.evanscomputermod.computer;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Server-side RGBA pixel buffer with dirty-tile tracking.
 * Used by DisplayBlockEntity to store pixel data that gets synced to clients.
 */
public class Framebuffer {

    /** Global memory tracker for all framebuffers (bytes). */
    private static final AtomicLong GLOBAL_MEMORY_USAGE = new AtomicLong(0);

    private final int width;
    private final int height;
    private final int tileSize;
    private final int tilesX;
    private final int tilesY;

    /** Current pixel data: RGBA, length = width * height * 4. */
    private final byte[] pixels;

    /** Snapshot from last flush (for diffing). */
    private final byte[] previousPixels;

    /** One flag per tile: true if tile has been modified since last flush. */
    private final boolean[] dirtyTiles;

    /** Whether any tile is dirty (fast check). */
    private volatile boolean anyDirty = false;

    /**
     * A single dirty tile's data.
     */
    public record DirtyTile(int tileX, int tileY, int pixelX, int pixelY,
                            int tileWidth, int tileHeight, byte[] pixelData) {
    }

    public Framebuffer(int width, int height, int tileSize) {
        this.width = width;
        this.height = height;
        this.tileSize = tileSize;
        this.tilesX = (width + tileSize - 1) / tileSize;
        this.tilesY = (height + tileSize - 1) / tileSize;

        int pixelBytes = width * height * 4;
        this.pixels = new byte[pixelBytes];
        this.previousPixels = new byte[pixelBytes];
        this.dirtyTiles = new boolean[tilesX * tilesY];

        GLOBAL_MEMORY_USAGE.addAndGet(pixelBytes * 2L);
    }

    /** Returns global memory usage across all framebuffers in bytes. */
    public static long getGlobalMemoryUsage() {
        return GLOBAL_MEMORY_USAGE.get();
    }

    /** Call when this framebuffer is no longer needed. */
    public void release() {
        GLOBAL_MEMORY_USAGE.addAndGet(-(long) pixels.length * 2);
    }

    public int getWidth() { return width; }
    public int getHeight() { return height; }
    public int getTileSize() { return tileSize; }
    public int getTilesX() { return tilesX; }
    public int getTilesY() { return tilesY; }
    public boolean isAnyDirty() { return anyDirty; }

    /** Returns raw pixel data (RGBA). For full-frame sync only. */
    public byte[] getPixels() { return pixels; }

    /**
     * Set a single pixel.
     */
    public void setPixel(int x, int y, int r, int g, int b, int a) {
        if (x < 0 || x >= width || y < 0 || y >= height) return;
        int idx = (y * width + x) * 4;
        pixels[idx] = (byte) r;
        pixels[idx + 1] = (byte) g;
        pixels[idx + 2] = (byte) b;
        pixels[idx + 3] = (byte) a;
        markTileDirty(x, y);
    }

    /**
     * Fill a rectangle with a solid color.
     */
    public void fillRect(int x, int y, int w, int h, int r, int g, int b, int a) {
        int x0 = Math.max(0, x);
        int y0 = Math.max(0, y);
        int x1 = Math.min(width, x + w);
        int y1 = Math.min(height, y + h);

        byte br = (byte) r, bg = (byte) g, bb = (byte) b, ba = (byte) a;

        for (int py = y0; py < y1; py++) {
            int rowBase = py * width * 4;
            for (int px = x0; px < x1; px++) {
                int idx = rowBase + px * 4;
                pixels[idx] = br;
                pixels[idx + 1] = bg;
                pixels[idx + 2] = bb;
                pixels[idx + 3] = ba;
            }
        }

        // Mark all affected tiles dirty
        int tileX0 = x0 / tileSize;
        int tileY0 = y0 / tileSize;
        int tileX1 = Math.min(tilesX - 1, (x1 - 1) / tileSize);
        int tileY1 = Math.min(tilesY - 1, (y1 - 1) / tileSize);
        for (int ty = tileY0; ty <= tileY1; ty++) {
            for (int tx = tileX0; tx <= tileX1; tx++) {
                dirtyTiles[ty * tilesX + tx] = true;
            }
        }
        anyDirty = true;
    }

    /**
     * Bulk write RGBA data from a byte array into a rectangular region.
     *
     * @param x      Left edge of the region
     * @param y      Top edge of the region
     * @param w      Width of the region
     * @param h      Height of the region
     * @param data   RGBA byte array, row-major, length must be >= w * h * 4
     */
    public void writeRegion(int x, int y, int w, int h, byte[] data) {
        if (data.length < w * h * 4) return;

        for (int row = 0; row < h; row++) {
            int srcY = y + row;
            if (srcY < 0 || srcY >= height) continue;
            int srcOffset = row * w * 4;
            int dstBase = srcY * width * 4;

            int startX = Math.max(0, x);
            int endX = Math.min(width, x + w);

            for (int px = startX; px < endX; px++) {
                int srcIdx = srcOffset + (px - x) * 4;
                int dstIdx = dstBase + px * 4;
                pixels[dstIdx] = data[srcIdx];
                pixels[dstIdx + 1] = data[srcIdx + 1];
                pixels[dstIdx + 2] = data[srcIdx + 2];
                pixels[dstIdx + 3] = data[srcIdx + 3];
            }
        }

        // Mark affected tiles dirty
        int tileX0 = Math.max(0, x) / tileSize;
        int tileY0 = Math.max(0, y) / tileSize;
        int tileX1 = Math.min(tilesX - 1, (Math.min(width, x + w) - 1) / tileSize);
        int tileY1 = Math.min(tilesY - 1, (Math.min(height, y + h) - 1) / tileSize);
        for (int ty = tileY0; ty <= tileY1; ty++) {
            for (int tx = tileX0; tx <= tileX1; tx++) {
                dirtyTiles[ty * tilesX + tx] = true;
            }
        }
        anyDirty = true;
    }

    /**
     * Clear the entire buffer with a solid color.
     */
    public void clear(int r, int g, int b, int a) {
        byte br = (byte) r, bg = (byte) g, bb = (byte) b, ba = (byte) a;
        for (int i = 0; i < pixels.length; i += 4) {
            pixels[i] = br;
            pixels[i + 1] = bg;
            pixels[i + 2] = bb;
            pixels[i + 3] = ba;
        }
        // Mark all tiles dirty
        for (int i = 0; i < dirtyTiles.length; i++) {
            dirtyTiles[i] = true;
        }
        anyDirty = true;
    }

    /**
     * Flush: diff against previous snapshot, return changed tiles, update snapshot.
     * Thread-safe: should be called from the server tick thread.
     */
    public synchronized List<DirtyTile> flush() {
        List<DirtyTile> result = new ArrayList<>();

        if (!anyDirty) return result;

        for (int ty = 0; ty < tilesY; ty++) {
            for (int tx = 0; tx < tilesX; tx++) {
                int tileIdx = ty * tilesX + tx;
                if (!dirtyTiles[tileIdx]) continue;

                // Calculate actual tile dimensions (edge tiles may be smaller)
                int tilePixelX = tx * tileSize;
                int tilePixelY = ty * tileSize;
                int tw = Math.min(tileSize, width - tilePixelX);
                int th = Math.min(tileSize, height - tilePixelY);

                // Check if this tile actually changed
                boolean changed = false;
                for (int row = 0; row < th && !changed; row++) {
                    int base = ((tilePixelY + row) * width + tilePixelX) * 4;
                    for (int col = 0; col < tw * 4; col++) {
                        if (pixels[base + col] != previousPixels[base + col]) {
                            changed = true;
                            break;
                        }
                    }
                }

                if (changed) {
                    // Extract tile pixel data
                    byte[] tileData = new byte[tw * th * 4];
                    for (int row = 0; row < th; row++) {
                        int srcBase = ((tilePixelY + row) * width + tilePixelX) * 4;
                        System.arraycopy(pixels, srcBase, tileData, row * tw * 4, tw * 4);
                    }

                    result.add(new DirtyTile(tx, ty, tilePixelX, tilePixelY, tw, th, tileData));

                    // Update snapshot for this tile
                    for (int row = 0; row < th; row++) {
                        int base = ((tilePixelY + row) * width + tilePixelX) * 4;
                        System.arraycopy(pixels, base, previousPixels, base, tw * 4);
                    }
                }

                dirtyTiles[tileIdx] = false;
            }
        }

        anyDirty = false;
        return result;
    }

    private void markTileDirty(int pixelX, int pixelY) {
        int tx = pixelX / tileSize;
        int ty = pixelY / tileSize;
        if (tx < tilesX && ty < tilesY) {
            dirtyTiles[ty * tilesX + tx] = true;
            anyDirty = true;
        }
    }
}
