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

    /**
     * Re-expand every pixel of a frame in {@code pixelFormat} (one of the
     * {@code TerminalDisplay.PIXEL_FORMAT_*} constants) and upload it to the
     * GPU. Runs on the render thread whenever the display's dirty counters
     * change, so up to the display's refresh rate.
     */
    public void updateFull(int pixelFormat, byte[] pixelData, int[] paletteARGB) {
        if (pixelData == null) return;
        NativeImage image = texture.getPixels();
        if (image == null) return;
        int w = this.width;
        int h = this.height;
        if (pixelData.length < w * h * TerminalDisplay.bytesPerPixel(pixelFormat)) return;

        switch (pixelFormat) {
            case TerminalDisplay.PIXEL_FORMAT_RGBA8888 -> {
                for (int y = 0; y < h; y++) {
                    int off = y * w * 4;
                    for (int x = 0; x < w; x++, off += 4) {
                        int abgr = ((pixelData[off + 3] & 0xFF) << 24) | ((pixelData[off + 2] & 0xFF) << 16)
                                | ((pixelData[off + 1] & 0xFF) << 8) | (pixelData[off] & 0xFF);
                        put(image, x, y, abgr);
                    }
                }
            }
            case TerminalDisplay.PIXEL_FORMAT_RGB565 -> {
                for (int y = 0; y < h; y++) {
                    int off = y * w * 2;
                    for (int x = 0; x < w; x++, off += 2) {
                        int p = (pixelData[off] & 0xFF) | ((pixelData[off + 1] & 0xFF) << 8);
                        put(image, x, y, rgb565ToAbgr(p));
                    }
                }
            }
            default -> {
                if (paletteARGB == null) return;
                // ARGB (server/packet format) -> ABGR (NativeImage's native order), once per frame.
                int paletteLen = Math.min(256, paletteARGB.length);
                for (int i = 0; i < paletteLen; i++) {
                    int a = paletteARGB[i];
                    abgrPalette[i] = (a & 0xFF00FF00) | ((a >>> 16) & 0xFF) | ((a & 0xFF) << 16);
                }
                for (int y = 0; y < h; y++) {
                    int rowBase = y * w;
                    for (int x = 0; x < w; x++) {
                        put(image, x, y, abgrPalette[pixelData[rowBase + x] & 0xFF]);
                    }
                }
            }
        }
        texture.upload();
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
