package com.example.evanscomputermod.computer.ui.font;

import com.example.evanscomputermod.computer.ui.proto.UiProto;

/**
 * The ECM fonts (docs/gui/UI_PROTOCOL.md §6 "Fonts are part of the protocol"). Guests lay text out with the same
 * tables (rust/crates/ecm-ui-proto/src/fonts.rs), so the rasterizer must measure and draw from these only; resource
 * packs can't change them.
 *
 * <p>A size is the line height in lu. A glyph is a bitmap {@code height} rows tall drawn at the pen position; its
 * advance is {@code width + spacing} (times the scale). A string's width is the sum of its code points' advances;
 * code points without a glyph draw as the design's fixed-width box.
 */
public final class UiFonts {
    private UiFonts() {}

    /** Sent in {@code Welcome.font_metrics}. */
    public static final int METRICS_VERSION = UiFontData.METRICS_VERSION;
    public static final int METRICS_HASH = UiFontData.METRICS_HASH;

    private static final Design[] DESIGNS = UiFontData.designs();

    /** One designed size of a font. Glyph rows are u16 bitmasks, bit 0 = leftmost column. */
    public static final class Design {
        public final String name;
        public final int font, baseSize, height, ascent, spacing;
        private final char[] glyphs; // (codepoint, width, row offset) triples, sorted
        private final char[] rows;
        private final int missingWidth, missingOffset;

        Design(String name, int font, int baseSize, int height, int ascent, int spacing, String glyphs, String rows,
               int missingWidth, int missingOffset) {
            this.name = name;
            this.font = font;
            this.baseSize = baseSize;
            this.height = height;
            this.ascent = ascent;
            this.spacing = spacing;
            this.glyphs = glyphs.toCharArray();
            this.rows = rows.toCharArray();
            this.missingWidth = missingWidth;
            this.missingOffset = missingOffset;
        }

        /** Index into the glyph table, or -1. */
        private int find(int cp) {
            int lo = 0, hi = glyphs.length / 3 - 1;
            while (lo <= hi) {
                int mid = (lo + hi) >>> 1;
                int c = glyphs[mid * 3];
                if (c < cp) lo = mid + 1;
                else if (c > cp) hi = mid - 1;
                else return mid;
            }
            return -1;
        }

        public boolean has(int cp) {
            return find(cp) >= 0;
        }

        public int width(int cp) {
            int i = find(cp);
            return i < 0 ? missingWidth : glyphs[i * 3 + 1];
        }

        public int advance(int cp) {
            return width(cp) + spacing;
        }

        /** Row {@code y} (0 = top) of the glyph for {@code cp}; bit 0 = leftmost column. */
        public int row(int cp, int y) {
            int i = find(cp);
            int off = i < 0 ? missingOffset : glyphs[i * 3 + 2];
            return rows[off + y];
        }

        int glyphCount() {
            return glyphs.length / 3;
        }
    }

    /** A font at one supported size. */
    public record Face(Design design, int scale, int size) {
        public int lineHeight() {
            return design.height * scale;
        }

        public int ascent() {
            return design.ascent * scale;
        }

        public int advance(int cp) {
            return design.advance(cp) * scale;
        }

        public int textWidth(String s) {
            int w = 0;
            for (int i = 0; i < s.length(); ) {
                int cp = s.codePointAt(i);
                w += design.advance(cp);
                i += Character.charCount(cp);
            }
            return w * scale;
        }
    }

    /** The face for {@code font} at {@code size}, or null if that size isn't supported. */
    public static Face face(UiProto.Font font, int size) {
        for (int[] s : UiFontData.SIZES) {
            if (s[0] == font.code && s[1] == size) return new Face(DESIGNS[s[2]], s[3], size);
        }
        return null;
    }

    public static boolean supported(UiProto.Font font, int size) {
        return face(font, size) != null;
    }

    /** FNV-1a 32 over the tables, as scripts/gen-ui-fonts.py computes it (equals {@link #METRICS_HASH}). */
    public static int computeMetricsHash() {
        int[] h = {0x811C9DC5};
        java.util.function.IntConsumer feed = v -> {
            for (int k = 0; k < 4; k++) {
                h[0] = (h[0] ^ ((v >>> (8 * k)) & 0xFF)) * 0x01000193;
            }
        };
        for (Design d : DESIGNS) {
            for (int v : new int[] {d.font, d.baseSize, d.height, d.ascent, d.spacing, d.missingWidth}) feed.accept(v);
            for (int y = 0; y < d.height; y++) feed.accept(d.rows[d.missingOffset + y]);
            for (int g = 0; g < d.glyphCount(); g++) {
                feed.accept(d.glyphs[g * 3]);
                feed.accept(d.glyphs[g * 3 + 1]);
                for (int y = 0; y < d.height; y++) feed.accept(d.rows[d.glyphs[g * 3 + 2] + y]);
            }
        }
        for (int[] s : UiFontData.SIZES) for (int v : s) feed.accept(v);
        return h[0];
    }
}
