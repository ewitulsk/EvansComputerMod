package com.example.evanscomputermod.computer.video;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Builder for the byte-layout that {@code TerminalDisplay.setGfxFromBytes}
 * consumes. The layout mirrors the WASM-side graphics framebuffer region
 * starting at {@code 0x30000}:
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
 * 0x40    768     palette (256 RGB triplets)
 * 0x400   w*h     indexed pixel data
 * </pre>
 *
 * This class is kept separate from {@code TerminalDisplay.gfxToBytes()}: that
 * method serializes state already held by the display; we go the other way,
 * synthesizing a blob from raw indexed pixel bytes + a palette so the display
 * can be updated in one atomic {@code setGfxFromBytes} call.
 */
public final class GfxFrameBlob {

    public static final int GFX_MAGIC = 0xFB02;
    public static final int HEADER_SIZE = 0x40;      // 64 bytes
    public static final int PALETTE_OFFSET = 0x40;   // 64
    public static final int PIXEL_OFFSET = 0x400;    // 1024
    public static final int PALETTE_SIZE = 768;      // 256 * 3

    private GfxFrameBlob() {}

    /**
     * Build a blob carrying one full frame.
     *
     * @param width         frame width in pixels
     * @param height        frame height in pixels
     * @param mode          display mode (0 text, 1 gfx, 2 overlay)
     * @param palette       768-byte RGB palette; if {@code null}, a zeroed
     *                      palette is written
     * @param indexedPixels {@code width * height} bytes of indexed pixels
     * @param paletteDirty  monotonic palette dirty counter
     * @param pixelDirty    monotonic pixel dirty counter
     */
    public static byte[] build(int width, int height, int mode,
                                byte[] palette, byte[] indexedPixels,
                                int paletteDirty, int pixelDirty) {
        if (width <= 0 || height <= 0) {
            throw new IllegalArgumentException("width/height must be positive");
        }
        int pixelCount = width * height;
        if (indexedPixels == null || indexedPixels.length != pixelCount) {
            throw new IllegalArgumentException(
                    "indexedPixels size mismatch: expected " + pixelCount
                    + " got " + (indexedPixels == null ? 0 : indexedPixels.length));
        }

        int totalSize = PIXEL_OFFSET + pixelCount;
        byte[] out = new byte[totalSize];
        ByteBuffer buf = ByteBuffer.wrap(out).order(ByteOrder.LITTLE_ENDIAN);

        buf.putShort(0x00, (short) GFX_MAGIC);
        out[0x02] = (byte) mode;
        out[0x03] = 0;
        buf.putShort(0x04, (short) width);
        buf.putShort(0x06, (short) height);
        buf.putInt(0x08, paletteDirty);
        buf.putInt(0x0C, pixelDirty);

        if (palette != null) {
            int copy = Math.min(palette.length, PALETTE_SIZE);
            System.arraycopy(palette, 0, out, PALETTE_OFFSET, copy);
        }

        System.arraycopy(indexedPixels, 0, out, PIXEL_OFFSET, pixelCount);
        return out;
    }
}
