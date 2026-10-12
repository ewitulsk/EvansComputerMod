package com.example.evanscomputermod.client;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.Identifier;

/**
 * Wraps a DynamicTexture for rendering the graphics framebuffer.
 * Three pixel formats are supported:
 * <ul>
 *   <li>{@link TerminalDisplay#PIXEL_FORMAT_INDEXED8}: 1 byte per pixel,
 *       expanded through a 256-entry ARGB palette.</li>
 *   <li>{@link TerminalDisplay#PIXEL_FORMAT_RGB565}: 2 bytes per pixel.</li>
 *   <li>{@link TerminalDisplay#PIXEL_FORMAT_RGBA8888}: 4 bytes per pixel,
 *       written directly to the GPU texture (with RGBA → ABGR byte swap).</li>
 * </ul>
 */
public class TerminalGraphicsTexture implements AutoCloseable {

    private DynamicTexture texture;
    private Identifier textureId;
    private int width;
    private int height;

    /** Palette converted to ABGR once per frame, so the pixel loop is a lookup and a store. */
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

    /** Uploads to the GPU so far, and how many of them were partial (diagnostics). */
    private long uploads, partialUploads;

    /**
     * Bring the texture up to date with {@code display}, re-expanding and
     * uploading only the 16x16 tiles the display marked as changed (see
     * {@link TerminalDisplay#takeGfxDamage}): a delta that touches a few
     * tiles costs a few small uploads, not a whole frame. {@code full}
     * forces everything (a texture that was just created or resized).
     * Render thread.
     */
    public void update(TerminalDisplay display, boolean full) {
        java.util.BitSet damage = display.takeGfxDamage();
        int pixelFormat = display.getPixelFormat();
        byte[] pixels = display.getPixelData();
        int[] palette = display.getPalette();
        int tilesX = (width + TerminalDisplay.GFX_TILE - 1) / TerminalDisplay.GFX_TILE;
        int tilesY = (height + TerminalDisplay.GFX_TILE - 1) / TerminalDisplay.GFX_TILE;
        // Past half the tiles one big upload is cheaper than many small ones.
        if (full || damage == null || damage.cardinality() * 2 > tilesX * tilesY) {
            updateFull(pixelFormat, pixels, palette);
            return;
        }
        if (damage.isEmpty() || pixels == null) return;
        NativeImage image = texture.getPixels();
        if (image == null) return;
        if (pixels.length < width * height * TerminalDisplay.bytesPerPixel(pixelFormat)) return;
        if (pixelFormat == TerminalDisplay.PIXEL_FORMAT_INDEXED8) {
            if (palette == null) return;
            convertPalette(palette);
        }
        // One upload per run of adjacent changed tiles in a tile row.
        for (int ty = 0; ty < tilesY; ty++) {
            int tx = 0;
            while (tx < tilesX) {
                if (!damage.get(ty * tilesX + tx)) {
                    tx++;
                    continue;
                }
                int run = tx;
                while (run < tilesX && damage.get(ty * tilesX + run)) run++;
                int x0 = tx * TerminalDisplay.GFX_TILE, y0 = ty * TerminalDisplay.GFX_TILE;
                int x1 = Math.min(width, run * TerminalDisplay.GFX_TILE);
                int y1 = Math.min(height, y0 + TerminalDisplay.GFX_TILE);
                expand(image, pixelFormat, pixels, x0, y0, x1, y1);
                uploadRegion(image, x0, y0, x1 - x0, y1 - y0);
                partialUploads++;
                tx = run;
            }
        }
    }

    /**
     * Re-expand every pixel of a frame in {@code pixelFormat} (one of the
     * {@code TerminalDisplay.PIXEL_FORMAT_*} constants) and upload it to the
     * GPU.
     */
    public void updateFull(int pixelFormat, byte[] pixelData, int[] paletteARGB) {
        if (pixelData == null) return;
        NativeImage image = texture.getPixels();
        if (image == null) return;
        int w = this.width;
        int h = this.height;
        if (pixelData.length < w * h * TerminalDisplay.bytesPerPixel(pixelFormat)) return;
        if (pixelFormat != TerminalDisplay.PIXEL_FORMAT_RGBA8888 && pixelFormat != TerminalDisplay.PIXEL_FORMAT_RGB565) {
            if (paletteARGB == null) return;
            convertPalette(paletteARGB);
        }
        expand(image, pixelFormat, pixelData, 0, 0, w, h);
        texture.upload();
        uploads++;
    }

    /** ARGB (server/packet format) -> ABGR (NativeImage's native order), once per update. */
    private void convertPalette(int[] paletteARGB) {
        int paletteLen = Math.min(256, paletteARGB.length);
        for (int i = 0; i < paletteLen; i++) {
            int a = paletteARGB[i];
            abgrPalette[i] = (a & 0xFF00FF00) | ((a >>> 16) & 0xFF) | ((a & 0xFF) << 16);
        }
    }

    /** Expand the pixels of {@code [x0, x1) x [y0, y1)} into the image. Indexed needs {@link #convertPalette} first. */
    private void expand(NativeImage image, int pixelFormat, byte[] pixelData, int x0, int y0, int x1, int y1) {
        int w = this.width;
        switch (pixelFormat) {
            case TerminalDisplay.PIXEL_FORMAT_RGBA8888 -> {
                for (int y = y0; y < y1; y++) {
                    int off = (y * w + x0) * 4;
                    for (int x = x0; x < x1; x++, off += 4) {
                        int abgr = ((pixelData[off + 3] & 0xFF) << 24) | ((pixelData[off + 2] & 0xFF) << 16)
                                | ((pixelData[off + 1] & 0xFF) << 8) | (pixelData[off] & 0xFF);
                        put(image, x, y, abgr);
                    }
                }
            }
            case TerminalDisplay.PIXEL_FORMAT_RGB565 -> {
                for (int y = y0; y < y1; y++) {
                    int off = (y * w + x0) * 2;
                    for (int x = x0; x < x1; x++, off += 2) {
                        int p = (pixelData[off] & 0xFF) | ((pixelData[off + 1] & 0xFF) << 8);
                        put(image, x, y, rgb565ToAbgr(p));
                    }
                }
            }
            default -> {
                for (int y = y0; y < y1; y++) {
                    int rowBase = y * w;
                    for (int x = x0; x < x1; x++) {
                        put(image, x, y, abgrPalette[pixelData[rowBase + x] & 0xFF]);
                    }
                }
            }
        }
    }

    /** Upload one rectangle of the image to the same place in the GPU texture. */
    private void uploadRegion(NativeImage image, int x, int y, int w, int h) {
        //? if >=26.1 {
        com.mojang.blaze3d.systems.RenderSystem.getDevice().createCommandEncoder()
                .writeToTexture(texture.getTexture(), image, 0, 0, x, y, w, h, x, y);
        //?} else {
        /*texture.bind();
        image.upload(0, x, y, x, y, w, h, false, false);*/
        //?}
        uploads++;
    }

    public long uploads() {
        return uploads;
    }

    public long partialUploads() {
        return partialUploads;
    }

    /** RGB565 (5-6-5, little-endian u16) to opaque ABGR, expanding each channel to 8 bits. */
    static int rgb565ToAbgr(int p) {
        int r = (p >> 11) & 0x1F;
        int g = (p >> 5) & 0x3F;
        int b = p & 0x1F;
        r = (r << 3) | (r >> 2);
        g = (g << 2) | (g >> 4);
        b = (b << 3) | (b >> 2);
        return 0xFF000000 | (b << 16) | (g << 8) | r;
    }

    private static void put(NativeImage image, int x, int y, int abgr) {
        //? if >=26.1 {
        image.setPixelABGR(x, y, abgr);
        //?} else
        /*image.setPixelRGBA(x, y, abgr);*/
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
