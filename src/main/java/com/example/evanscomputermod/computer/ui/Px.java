package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.video.Rgb332Palette;

/**
 * Pixel math shared by the rasterizer and the pixel paths. Everything the
 * compositor draws is <b>premultiplied ARGB</b> in an {@code int}
 * ({@code 0xAARRGGBB}, colour channels already multiplied by alpha). All
 * arithmetic is integer and exact, so every renderer that uses this class
 * (the server today, the client in phase 2) produces the same pixels.
 */
public final class Px {
    private Px() {}

    /** The fixed palette of protocol pixel surfaces and canvases in indexed8 (RGB332). */
    private static final int[] RGB332 = Rgb332Palette.argb();

    /** {@code round(x / 255)} for {@code 0 <= x <= 255 * 255}. */
    public static int div255(int x) {
        x += 128;
        return (x + (x >> 8)) >> 8;
    }

    /** Protocol colour {@code 0xRRGGBBAA} (straight alpha) to premultiplied ARGB. */
    public static int premul(int rgba) {
        int a = rgba & 0xFF;
        if (a == 0) return 0;
        int r = rgba >>> 24, g = (rgba >>> 16) & 0xFF, b = (rgba >>> 8) & 0xFF;
        if (a == 255) return 0xFF000000 | r << 16 | g << 8 | b;
        return a << 24 | div255(r * a) << 16 | div255(g * a) << 8 | div255(b * a);
    }

    /** Straight RGBA bytes to premultiplied ARGB. */
    public static int premul(int r, int g, int b, int a) {
        if (a == 0) return 0;
        if (a == 255) return 0xFF000000 | r << 16 | g << 8 | b;
        return a << 24 | div255(r * a) << 16 | div255(g * a) << 8 | div255(b * a);
    }

    /** A premultiplied colour scaled by {@code o} (0..255). */
    public static int fade(int c, int o) {
        if (o >= 255) return c;
        if (o <= 0 || c == 0) return 0;
        return div255((c >>> 24) * o) << 24 | div255(((c >>> 16) & 0xFF) * o) << 16
                | div255(((c >>> 8) & 0xFF) * o) << 8 | div255((c & 0xFF) * o);
    }

    /** Source-over: {@code src} (premultiplied) on top of {@code dst} (premultiplied). */
    public static int over(int src, int dst) {
        int sa = src >>> 24;
        if (sa == 255) return src;
        if (sa == 0) return dst;
        int inv = 255 - sa;
        int a = sa + div255((dst >>> 24) * inv);
        int r = ((src >>> 16) & 0xFF) + div255(((dst >>> 16) & 0xFF) * inv);
        int g = ((src >>> 8) & 0xFF) + div255(((dst >>> 8) & 0xFF) * inv);
        int b = (src & 0xFF) + div255((dst & 0xFF) * inv);
        return a << 24 | r << 16 | g << 8 | b;
    }

    /** Multiply a premultiplied colour by a straight-alpha tint {@code 0xRRGGBBAA}. */
    public static int tint(int c, int tintRgba) {
        if (tintRgba == 0xFFFFFFFF) return c;
        int ta = tintRgba & 0xFF;
        int tr = tintRgba >>> 24, tg = (tintRgba >>> 16) & 0xFF, tb = (tintRgba >>> 8) & 0xFF;
        int a = div255((c >>> 24) * ta);
        int r = div255(div255(((c >>> 16) & 0xFF) * tr) * ta);
        int g = div255(div255(((c >>> 8) & 0xFF) * tg) * ta);
        int b = div255(div255((c & 0xFF) * tb) * ta);
        return a << 24 | r << 16 | g << 8 | b;
    }

    /** RGB565 (little-endian u16 value) to opaque ARGB, expanding like the client texture does. */
    public static int rgb565(int p) {
        int r = (p >> 11) & 0x1F, g = (p >> 5) & 0x3F, b = p & 0x1F;
        r = (r << 3) | (r >> 2);
        g = (g << 2) | (g >> 4);
        b = (b << 3) | (b >> 2);
        return 0xFF000000 | r << 16 | g << 8 | b;
    }

    /**
     * Convert rectangle {@code (x, y, w, h)} of a {@code bw x bh} buffer in a
     * {@code TerminalDisplay.PIXEL_FORMAT_*} to premultiplied ARGB in {@code dst}
     * (same {@code bw x bh} layout). {@code palette} (opaque ARGB, 256 entries) is
     * used for indexed8; null means the fixed RGB332 palette.
     */
    public static void convert(byte[] src, int format, int[] palette, int bw, int bh,
                               int x, int y, int w, int h, int[] dst) {
        int x0 = Math.max(0, x), y0 = Math.max(0, y);
        int x1 = Math.min(bw, x + w), y1 = Math.min(bh, y + h);
        int[] pal = palette != null ? palette : RGB332;
        for (int row = y0; row < y1; row++) {
            int o = row * bw;
            switch (format) {
                case TerminalDisplay.PIXEL_FORMAT_RGBA8888 -> {
                    for (int col = x0; col < x1; col++) {
                        int i = (o + col) * 4;
                        dst[o + col] = premul(src[i] & 0xFF, src[i + 1] & 0xFF, src[i + 2] & 0xFF, src[i + 3] & 0xFF);
                    }
                }
                case TerminalDisplay.PIXEL_FORMAT_RGB565 -> {
                    for (int col = x0; col < x1; col++) {
                        int i = (o + col) * 2;
                        dst[o + col] = rgb565((src[i] & 0xFF) | (src[i + 1] & 0xFF) << 8);
                    }
                }
                default -> {
                    for (int col = x0; col < x1; col++) {
                        dst[o + col] = 0xFF000000 | pal[src[o + col] & 0xFF];
                    }
                }
            }
        }
    }

    /** Premultiplied ARGB to straight RGBA bytes ({@code TerminalDisplay} RGBA8888). Opaque in practice. */
    public static void toRgbaBytes(int c, byte[] out, int off) {
        int a = c >>> 24;
        int r = (c >>> 16) & 0xFF, g = (c >>> 8) & 0xFF, b = c & 0xFF;
        if (a != 255 && a != 0) {
            r = Math.min(255, (r * 255 + a / 2) / a);
            g = Math.min(255, (g * 255 + a / 2) / a);
            b = Math.min(255, (b * 255 + a / 2) / a);
        }
        out[off] = (byte) r;
        out[off + 1] = (byte) g;
        out[off + 2] = (byte) b;
        out[off + 3] = (byte) (a == 0 ? 0 : a);
    }
}
