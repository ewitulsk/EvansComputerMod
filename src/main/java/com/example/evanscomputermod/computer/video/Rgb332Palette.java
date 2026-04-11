package com.example.evanscomputermod.computer.video;

/**
 * Canonical 256-entry RGB palette matching FFmpeg's {@code AV_PIX_FMT_RGB8}
 * pixel format (packed 3:3:2 with the layout {@code (msb) 2R 3G 3B (lsb)}).
 *
 * <p>FFmpeg's libswscale can output directly in this format, so we pay zero
 * runtime cost for RGB→indexed quantization. We just install this fixed
 * palette in the graphics framebuffer once and blit the swscale bytes as-is.
 */
public final class Rgb332Palette {

    /** Number of palette entries. */
    public static final int ENTRY_COUNT = 256;

    /** Raw bytes of the palette: 256 * 3 = 768 bytes, R,G,B,R,G,B,... */
    private static final byte[] DATA = buildPalette();

    private Rgb332Palette() {}

    /**
     * Returns a fresh copy of the 768-byte palette buffer. Indices 0-255 map
     * to consecutive RGB triples. Callers are free to mutate the returned
     * array without affecting the canonical palette.
     */
    public static byte[] bytes() {
        return DATA.clone();
    }

    /**
     * Copies the palette into {@code dst} starting at {@code offset}. Writes
     * exactly 768 bytes. Useful when building a larger gfx-framebuffer blob.
     */
    public static void copyInto(byte[] dst, int offset) {
        System.arraycopy(DATA, 0, dst, offset, DATA.length);
    }

    private static byte[] buildPalette() {
        byte[] out = new byte[ENTRY_COUNT * 3];
        for (int i = 0; i < ENTRY_COUNT; i++) {
            int r2 = (i >> 6) & 0x3;
            int g3 = (i >> 3) & 0x7;
            int b3 = i & 0x7;
            int r = (r2 * 255) / 3;
            int g = (g3 * 255) / 7;
            int b = (b3 * 255) / 7;
            out[i * 3]     = (byte) r;
            out[i * 3 + 1] = (byte) g;
            out[i * 3 + 2] = (byte) b;
        }
        return out;
    }
}
