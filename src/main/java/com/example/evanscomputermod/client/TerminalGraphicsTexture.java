package com.example.evanscomputermod.client;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.Identifier;

/**
 * Wraps a DynamicTexture for rendering the graphics framebuffer.
 * Two pixel formats are supported:
 * <ul>
 *   <li>{@link TerminalDisplay#PIXEL_FORMAT_INDEXED8}: 1 byte per pixel,
 *       expanded through a 256-entry ARGB palette.</li>
 *   <li>{@link TerminalDisplay#PIXEL_FORMAT_RGBA8888}: 4 bytes per pixel,
 *       written directly to the GPU texture (with RGBA → ABGR byte swap).</li>
 * </ul>
 */
public class TerminalGraphicsTexture implements AutoCloseable {

    private DynamicTexture texture;
    private Identifier textureId;
    private int width;
    private int height;

    /**
     * Pre-converted ABGR palette used by the update hot path. Computing this
     * once per {@link #updateFull} call lets the inner pixel loop call
     * {@link NativeImage#setPixelABGR} directly, skipping the per-pixel
     * {@code ARGB.toABGR} swap that {@link NativeImage#setPixel} does. Also
     * lets the JIT see a tight lookup-and-store kernel.
     */
    private final int[] abgrPalette = new int[256];

    public TerminalGraphicsTexture(int width, int height) {
        this.width = width;
        this.height = height;
        //? if >=26.1 {
        this.texture = new DynamicTexture("ecm_gfx", width, height, true);
        //?} else
        /*this.texture = new DynamicTexture(width, height, true);*/
        this.textureId = Identifier.fromNamespaceAndPath("evanscomputermod",
                "dynamic/gfx_" + System.identityHashCode(this));
        Minecraft.getInstance().getTextureManager().register(this.textureId, this.texture);
    }

    /**
     * Full update: re-expand all pixels and upload to the GPU. Routes by
     * {@code pixelFormat} — see {@link TerminalDisplay#PIXEL_FORMAT_INDEXED8}
     * and {@link TerminalDisplay#PIXEL_FORMAT_RGBA8888}.
     */
    public void updateFull(int pixelFormat, byte[] pixelData, int[] paletteARGB) {
        if (pixelFormat == TerminalDisplay.PIXEL_FORMAT_RGBA8888) {
            updateFullRgba(pixelData);
        } else {
            updateFull(pixelData, paletteARGB);
        }
    }

    /**
     * Indexed-color full update — re-expand all pixels from indexed data
     * using the palette. Runs on the render thread every time the anchor's
     * dirty counters bump (as often as 20 Hz during {@code gfxtest screen}'s
     * palette-animation and bouncing-ball phases). Pre-converts ARGB → ABGR
     * once per call so the inner pixel loop is a tight {@code lookup +
     * setPixelABGR} kernel with no per-pixel channel swap.
     */
    public void updateFull(byte[] pixelData, int[] paletteARGB) {
        if (pixelData == null || paletteARGB == null) return;
        NativeImage image = texture.getPixels();
        if (image == null) return;

        // Pre-convert ARGB (server/packet format) → ABGR (NativeImage native
        // format). 256 entries, negligible cost.
        int paletteLen = Math.min(256, paletteARGB.length);
        for (int i = 0; i < paletteLen; i++) {
            int a = paletteARGB[i];
            abgrPalette[i] = (a & 0xFF00FF00) | ((a >>> 16) & 0xFF) | ((a & 0xFF) << 16);
        }

        int w = this.width;
        int h = this.height;
        int total = w * h;
        if (pixelData.length < total) return;

        for (int y = 0; y < h; y++) {
            int rowBase = y * w;
            for (int x = 0; x < w; x++) {
                //? if >=26.1 {
                image.setPixelABGR(x, y, abgrPalette[pixelData[rowBase + x] & 0xFF]);
                //?} else
                /*image.setPixelRGBA(x, y, abgrPalette[pixelData[rowBase + x] & 0xFF]);*/
            }
        }
        texture.upload();
    }

    /**
     * RGBA8888 full update — copy pixels directly to the GPU texture with
     * an RGBA → ABGR byte swap. {@code pixelData} is laid out as 4 bytes
     * per pixel: R, G, B, A.
     */
    public void updateFullRgba(byte[] pixelData) {
        if (pixelData == null) return;
        NativeImage image = texture.getPixels();
        if (image == null) return;

        int w = this.width;
        int h = this.height;
        int needed = w * h * 4;
        if (pixelData.length < needed) return;

        for (int y = 0; y < h; y++) {
            int rowBase = y * w * 4;
            for (int x = 0; x < w; x++) {
                int off = rowBase + x * 4;
                int r = pixelData[off]     & 0xFF;
                int g = pixelData[off + 1] & 0xFF;
                int b = pixelData[off + 2] & 0xFF;
                int a = pixelData[off + 3] & 0xFF;
                int abgr = (a << 24) | (b << 16) | (g << 8) | r;
                //? if >=26.1 {
                image.setPixelABGR(x, y, abgr);
                //?} else
                /*image.setPixelRGBA(x, y, abgr);*/
            }
        }
        texture.upload();
    }

    /**
     * Partial update by tile. Routes by {@code pixelFormat}; the indexed
     * variant uses the legacy 1-bpp tile path, the rgba variant copies
     * 16×16×4 bytes per tile direct to the texture.
     */
    public void updateTiles(int pixelFormat, java.util.List<Integer> tileIndices,
                            java.util.List<byte[]> tileData, int tilesPerRow, int[] paletteARGB) {
        if (pixelFormat == TerminalDisplay.PIXEL_FORMAT_RGBA8888) {
            updateTilesRgba(tileIndices, tileData, tilesPerRow);
        } else {
            updateTiles(tileIndices, tileData, tilesPerRow, paletteARGB);
        }
    }

    /**
     * Indexed-color partial update: only update specific 16x16 tiles.
     */
    public void updateTiles(java.util.List<Integer> tileIndices, java.util.List<byte[]> tileData,
                            int tilesPerRow, int[] paletteARGB) {
        NativeImage image = texture.getPixels();
        if (image == null || paletteARGB == null) return;

        for (int i = 0; i < tileIndices.size(); i++) {
            int tileIdx = tileIndices.get(i);
            int tileX = (tileIdx % tilesPerRow) * 16;
            int tileY = (tileIdx / tilesPerRow) * 16;
            byte[] data = tileData.get(i);

            for (int py = 0; py < 16 && tileY + py < height; py++) {
                for (int px = 0; px < 16 && tileX + px < width; px++) {
                    int idx = data[py * 16 + px] & 0xFF;
                    int argb = paletteARGB[idx];
                    //? if >=26.1 {
                    image.setPixel(tileX + px, tileY + py, argb);
                    //?} else {
                    /*int abgrT = (argb & 0xFF00FF00) | ((argb >>> 16) & 0xFF) | ((argb & 0xFF) << 16);
                    image.setPixelRGBA(tileX + px, tileY + py, abgrT);*/
                    //?}
                }
            }
        }
        texture.upload();
    }

    /**
     * RGBA partial update: tiles are 16×16×4 bytes (RGBA) — copy with
     * RGBA → ABGR byte swap into the GPU texture.
     */
    public void updateTilesRgba(java.util.List<Integer> tileIndices,
                                 java.util.List<byte[]> tileData, int tilesPerRow) {
        NativeImage image = texture.getPixels();
        if (image == null) return;

        for (int i = 0; i < tileIndices.size(); i++) {
            int tileIdx = tileIndices.get(i);
            int tileX = (tileIdx % tilesPerRow) * 16;
            int tileY = (tileIdx / tilesPerRow) * 16;
            byte[] data = tileData.get(i);

            for (int py = 0; py < 16 && tileY + py < height; py++) {
                int rowBase = py * 16 * 4;
                for (int px = 0; px < 16 && tileX + px < width; px++) {
                    int off = rowBase + px * 4;
                    int r = data[off]     & 0xFF;
                    int g = data[off + 1] & 0xFF;
                    int b = data[off + 2] & 0xFF;
                    int a = data[off + 3] & 0xFF;
                    int abgr = (a << 24) | (b << 16) | (g << 8) | r;
                    //? if >=26.1 {
                    image.setPixelABGR(tileX + px, tileY + py, abgr);
                    //?} else
                    /*image.setPixelRGBA(tileX + px, tileY + py, abgr);*/
                }
            }
        }
        texture.upload();
    }

    /**
     * Re-expand all pixels with a new palette (pixel data unchanged, colors changed).
     */
    public void updatePalette(byte[] pixelData, int[] paletteARGB) {
        updateFull(pixelData, paletteARGB);
    }

    /**
     * Resize the texture if the graphics resolution changed.
     */
    public void resize(int newWidth, int newHeight) {
        if (newWidth == this.width && newHeight == this.height) return;

        close();
        this.width = newWidth;
        this.height = newHeight;
        //? if >=26.1 {
        this.texture = new DynamicTexture("ecm_gfx", newWidth, newHeight, true);
        //?} else
        /*this.texture = new DynamicTexture(newWidth, newHeight, true);*/
        this.textureId = Identifier.fromNamespaceAndPath("evanscomputermod",
                "dynamic/gfx_" + System.identityHashCode(this));
        Minecraft.getInstance().getTextureManager().register(this.textureId, this.texture);
    }

    public Identifier getTextureId() {
        return textureId;
    }

    public int getWidth() {
        return width;
    }

    public int getHeight() {
        return height;
    }

    @Override
    public void close() {
        if (textureId != null) {
            Minecraft.getInstance().getTextureManager().release(textureId);
            textureId = null;
        }
        // texture is closed by TextureManager.release()
        texture = null;
    }
}
