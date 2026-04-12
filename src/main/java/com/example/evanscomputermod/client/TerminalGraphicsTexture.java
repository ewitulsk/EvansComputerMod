package com.example.evanscomputermod.client;

import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.Identifier;

/**
 * Wraps a DynamicTexture for rendering the graphics framebuffer.
 * Indexed pixel data + 256-color ARGB palette → GPU texture.
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
        this.texture = new DynamicTexture("ecm_gfx", width, height, true);
        this.textureId = Identifier.fromNamespaceAndPath("evanscomputermod",
                "dynamic/gfx_" + System.identityHashCode(this));
        Minecraft.getInstance().getTextureManager().register(this.textureId, this.texture);
    }

    /**
     * Full update: re-expand all pixels from indexed data using the palette.
     * Runs on the render thread every time the anchor's dirty counters bump
     * (as often as 20 Hz during {@code gfxtest screen}'s palette-animation
     * and bouncing-ball phases). Pre-converts ARGB → ABGR once per call so
     * the inner pixel loop is a tight {@code lookup + setPixelABGR} kernel
     * with no per-pixel channel swap.
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
                image.setPixelABGR(x, y, abgrPalette[pixelData[rowBase + x] & 0xFF]);
            }
        }
        texture.upload();
    }

    /**
     * Partial update: only update specific 16x16 tiles.
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
                    image.setPixel(tileX + px, tileY + py, argb);
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
        this.texture = new DynamicTexture("ecm_gfx", newWidth, newHeight, true);
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
