package com.example.evanscomputermod.computer.video;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Builder for the byte-layout that {@code TerminalDisplay.setGfxFromBytes}
 * consumes. The layout mirrors the WASM-side graphics framebuffer region:
 *
 * <pre>
 * Offset  Size    Field
 * 0x00    u16     magic (0xFB02)
 * 0x02    u8      mode (0=text, 1=gfx, 2=overlay)
 * 0x03    u8      reserved
 * 0x04    u16     width
 * 0x06    u16     height
 * 0x08    u32     palette_dirty counter
 * 0x0C    u32     pixel_dirty counter
 * 0x10    u8      pixel_format (0=indexed8, 1=rgba8888)
 * 0x40    768     palette (256 RGB triplets; only used in indexed8 mode)
 * 0x400   ...     pixel data (w*h for indexed8, w*h*4 for rgba8888)
 * </pre>
 *
 * This class is kept separate from {@code TerminalDisplay.gfxToBytes()}: that
 * method serializes state already held by the display; we go the other way,
 * synthesizing a blob from raw pixel bytes + a palette so the display can
 * be updated in one atomic {@code setGfxFromBytes} call.
 */
public final class GfxFrameBlob {

    public static final int GFX_MAGIC = 0xFB02;
    public static final int HEADER_SIZE = 0x40;      // 64 bytes
    public static final int PALETTE_OFFSET = 0x40;   // 64
    public static final int PIXEL_OFFSET = 0x400;    // 1024
    public static final int PALETTE_SIZE = 768;      // 256 * 3
    public static final int OFF_PIXEL_FORMAT = 0x10;

    public static final int PIXEL_FORMAT_INDEXED8 = 0;
    public static final int PIXEL_FORMAT_RGBA8888 = 1;

    private GfxFrameBlob() {}

    /**
     * Build a blob carrying one full indexed-color frame. Equivalent to
     * calling {@link #build(int, int, int, int, byte[], byte[], int, int)}
     * with {@code pixelFormat = PIXEL_FORMAT_INDEXED8}.
     */
    public static byte[] build(int width, int height, int mode,
                                byte[] palette, byte[] indexedPixels,
                                int paletteDirty, int pixelDirty) {
        return build(width, height, mode, PIXEL_FORMAT_INDEXED8,
                palette, indexedPixels, paletteDirty, pixelDirty);
    }

    /**
     * Build a blob carrying one full frame in the requested pixel format.
     *
     * @param width         frame width in pixels
     * @param height        frame height in pixels
     * @param mode          display mode (0 text, 1 gfx, 2 overlay)
     * @param pixelFormat   {@link #PIXEL_FORMAT_INDEXED8} or {@link #PIXEL_FORMAT_RGBA8888}
     * @param palette       768-byte RGB palette; if {@code null}, a zeroed
     *                      palette is written. Ignored for rgba but still
     *                      stored in the reserved palette region.
     * @param pixels        pixel bytes — {@code w*h} for indexed8 or
     *                      {@code w*h*4} for rgba8888 (RGBA byte order)
     * @param paletteDirty  monotonic palette dirty counter
     * @param pixelDirty    monotonic pixel dirty counter
     */
    public static byte[] build(int width, int height, int mode, int pixelFormat,
                                byte[] palette, byte[] pixels,
                                int paletteDirty, int pixelDirty) {
        if (width <= 0 || height <= 0) {
            throw new IllegalArgumentException("width/height must be positive");
        }
        int bpp = (pixelFormat == PIXEL_FORMAT_RGBA8888) ? 4 : 1;
        int pixelByteCount = width * height * bpp;
        if (pixels == null || pixels.length != pixelByteCount) {
            throw new IllegalArgumentException(
                    "pixel byte count mismatch (format=" + pixelFormat + "): expected " + pixelByteCount
                    + " got " + (pixels == null ? 0 : pixels.length));
        }

        int totalSize = PIXEL_OFFSET + pixelByteCount;
        byte[] out = new byte[totalSize];
        ByteBuffer buf = ByteBuffer.wrap(out).order(ByteOrder.LITTLE_ENDIAN);

        buf.putShort(0x00, (short) GFX_MAGIC);
        out[0x02] = (byte) mode;
        out[0x03] = 0;
        buf.putShort(0x04, (short) width);
        buf.putShort(0x06, (short) height);
        buf.putInt(0x08, paletteDirty);
        buf.putInt(0x0C, pixelDirty);
        out[OFF_PIXEL_FORMAT] = (byte) pixelFormat;

        if (palette != null) {
            int copy = Math.min(palette.length, PALETTE_SIZE);
            System.arraycopy(palette, 0, out, PALETTE_OFFSET, copy);
        }

        System.arraycopy(pixels, 0, out, PIXEL_OFFSET, pixelByteCount);
        return out;
    }
}
