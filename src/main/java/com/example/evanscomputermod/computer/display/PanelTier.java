package com.example.evanscomputermod.computer.display;

import com.example.evanscomputermod.EcmConfig;
import org.jetbrains.annotations.Nullable;

/**
 * Display panel tiers: what a display can do, like a monitor's EDID. A panel
 * item sits in a device's screen slot (a Terminal today, the ePhone later)
 * and its tier sets the modes the display advertises. Screen blocks carry a
 * tier of their own; a Screen wall runs at its lowest block's tier.
 *
 * <p>This is the one place the tier table lives (docs/gui/DISPLAY_PIPELINE.md
 * §6). The numbers below are defaults; a server can lower each of them in
 * {@code config/evanscomputermod-common.toml} ({@code display.tier1..3}), see
 * {@link EcmConfig}. Tier 2 matches the limits from before tiers existed, so
 * Terminals and Screens from older worlds load as Tier 2 and lose nothing.
 */
public enum PanelTier {
    //            text      gfx (landscape)  phone (portrait)  screen tile  wall cap      Hz  touch
    PHOSPHOR(1,   80, 25,   320, 200,        120, 200, 1,      64, 36,      320, 180,     20, false),
    COLOR_LCD(2, 160, 50,   640, 400,        240, 400, 1,     128, 72,      640, 360,     60, false),
    HD(3,        160, 50,  1280, 800,        480, 800, 2,     256, 144,    1920, 1080,    60, true);

    /** {@link Modes#colorFlags} bits (also in {@code gfx_modes}, abi/child-abi.toml). */
    public static final int COLOR_TWO_TONE = 1;
    public static final int COLOR_INDEXED8 = 2;
    public static final int COLOR_RGB565 = 4;
    public static final int COLOR_RGBA = 8;
    public static final int COLOR_ALPHA = 16;

    private final int tier;
    private final int textCols, textRows, gfxW, gfxH, phoneW, phoneH, scale;
    private final int tileW, tileH, wallW, wallH, refreshHz;
    private final boolean touch;

    PanelTier(int tier, int textCols, int textRows, int gfxW, int gfxH, int phoneW, int phoneH, int scale,
              int tileW, int tileH, int wallW, int wallH, int refreshHz, boolean touch) {
        this.tier = tier;
        this.textCols = textCols;
        this.textRows = textRows;
        this.gfxW = gfxW;
        this.gfxH = gfxH;
        this.phoneW = phoneW;
        this.phoneH = phoneH;
        this.scale = scale;
        this.tileW = tileW;
        this.tileH = tileH;
        this.wallW = wallW;
        this.wallH = wallH;
        this.refreshHz = refreshHz;
        this.touch = touch;
    }

    /** 1, 2 or 3. */
    public int tier() {
        return tier;
    }

    /** By number (1..3), or null. */
    @Nullable
    public static PanelTier of(int tier) {
        for (PanelTier t : values()) if (t.tier == tier) return t;
        return null;
    }

    /** The lower of two tiers (a mixed Screen wall runs at its lowest). */
    public static PanelTier min(PanelTier a, PanelTier b) {
        return a.tier <= b.tier ? a : b;
    }

    /** Lower-case name used in lang keys, configs and the block-state ({@code phosphor}, {@code color_lcd}, {@code hd}). */
    public String id() {
        return name().toLowerCase(java.util.Locale.ROOT);
    }

    /**
     * One display mode: a text grid and a graphics size in device pixels,
     * with the scale (device pixels per logical unit, UI_PROTOCOL §4).
     */
    public record DisplayMode(int textCols, int textRows, int gfxWidth, int gfxHeight, int scale) {
        public int logicalWidth() {
            return gfxWidth / Math.max(1, scale);
        }

        public int logicalHeight() {
            return gfxHeight / Math.max(1, scale);
        }

        /** compact (&lt; 400 lu wide), medium (400-799) or wide (&ge; 800), UI_PROTOCOL §4. */
        public String sizeClass() {
            int w = logicalWidth();
            return w < 400 ? "compact" : w < 800 ? "medium" : "wide";
        }
    }

    /**
     * Everything the tier allows, after the server's config caps: what
     * {@code gfx_modes} reports and what every limit is computed from.
     *
     * @param terminal    landscape mode of a Terminal (text grid, max graphics)
     * @param phone       portrait mode of an ePhone (text grid derived from 8x16 cells)
     * @param screenTileW per-block Screen resolution
     * @param screenMaxW  Screen wall resolution cap
     * @param refreshHz   highest refresh rate (still clamped by display.maxRefreshHz)
     * @param colorFlags  {@code COLOR_*} bits: what the panel can show
     * @param touch       a Screen wall of this tier takes clicks as pointer events
     */
    public record Modes(int tier, DisplayMode terminal, DisplayMode phone,
                        int screenTileW, int screenTileH, int screenMaxW, int screenMaxH,
                        int refreshHz, int colorFlags, boolean touch) {
        /** Largest program framebuffer on a Terminal of this tier (RGBA at the max graphics size). */
        public int terminalMaxBytes() {
            return terminal.gfxWidth() * terminal.gfxHeight() * 4;
        }

        /** Largest program framebuffer on a Screen wall of this tier. */
        public int screenMaxBytes() {
            return screenMaxW * screenMaxH * 4;
        }
    }

    /** This tier's modes, with the server's caps applied. */
    public Modes modes() {
        EcmConfig.TierCaps c = EcmConfig.tierCaps(this);
        int cols = Math.min(textCols, c.textCols());
        int rows = Math.min(textRows, c.textRows());
        int gw = Math.min(gfxW, c.gfxWidth());
        int gh = Math.min(gfxH, c.gfxHeight());
        int pw = Math.min(phoneW, c.phoneWidth());
        int ph = Math.min(phoneH, c.phoneHeight());
        DisplayMode terminal = new DisplayMode(cols, rows, gw, gh, scale);
        DisplayMode phone = new DisplayMode(Math.max(1, pw / (8 * scale)), Math.max(1, ph / (16 * scale)), pw, ph, scale);
        int colors = switch (this) {
            case PHOSPHOR -> COLOR_TWO_TONE;
            case COLOR_LCD -> COLOR_INDEXED8 | COLOR_RGB565 | COLOR_RGBA;
            case HD -> COLOR_INDEXED8 | COLOR_RGB565 | COLOR_RGBA | COLOR_ALPHA;
        };
        return new Modes(tier, terminal, phone,
                Math.min(tileW, c.screenTileWidth()), Math.min(tileH, c.screenTileHeight()),
                Math.min(wallW, c.screenMaxWidth()), Math.min(wallH, c.screenMaxHeight()),
                Math.max(1, Math.min(refreshHz, c.refreshHz())), colors, touch);
    }

    /** Defaults before any config cap (for the config's own defaults). */
    public EcmConfig.TierCaps defaults() {
        return new EcmConfig.TierCaps(textCols, textRows, gfxW, gfxH, phoneW, phoneH,
                tileW, tileH, wallW, wallH, refreshHz);
    }

    /** Phosphor colours (Tier 1 shows everything in two tones of one of these). */
    public enum Tone {
        GREEN(0xFF33FF66),
        AMBER(0xFFFFB000);

        private final int argb;

        Tone(int argb) {
            this.argb = argb;
        }

        /** The lit colour, ARGB. */
        public int argb() {
            return argb;
        }

        /** The unlit colour: the lit one at a tenth, ARGB. */
        public int darkArgb() {
            int r = ((argb >> 16) & 0xFF) / 10, g = ((argb >> 8) & 0xFF) / 10, b = (argb & 0xFF) / 10;
            return 0xFF000000 | (r << 16) | (g << 8) | b;
        }

        /** Map any ARGB colour to the two tones by luminance. */
        public int map(int argb) {
            int r = (argb >> 16) & 0xFF, g = (argb >> 8) & 0xFF, b = argb & 0xFF;
            int lum = (r * 299 + g * 587 + b * 114) / 1000;
            return lum >= 64 ? this.argb : darkArgb();
        }
    }
}
