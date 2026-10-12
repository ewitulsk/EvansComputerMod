package com.example.evanscomputermod.phone.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.ResourceLocation;
import org.jetbrains.annotations.Nullable;

import java.io.InputStream;
import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;
import java.util.UUID;

/**
 * Client: an ePhone's screen as one texture at the panel's portrait
 * resolution (120x200, 240x400 or 480x800), composed on the CPU from the
 * display this client was sent ({@link ClientDisplayCache}, by phone UUID and
 * display id): the graphics plane (nearest-neighbour, letterboxed), then the
 * text grid in the terminal font (8x16 cells times the panel scale), its
 * colours, inverse/underline/bold and the cursor. A Tier 1 panel is drawn in
 * two tones of its phosphor colour. Recomposed only when the picture changed.
 * The in-hand renderer, the dock and the open phone screen all use it.
 */
public final class PhoneScreenTexture implements AutoCloseable {

    private static final int[] PALETTE = {
            0xFF000000, 0xFFAA0000, 0xFF00AA00, 0xFFAA5500, 0xFF0000AA, 0xFFAA00AA, 0xFF00AAAA, 0xFFAAAAAA,
            0xFF555555, 0xFFFF5555, 0xFF55FF55, 0xFFFFFF55, 0xFF5555FF, 0xFFFF55FF, 0xFF55FFFF, 0xFFFFFFFF};
    private static final int BACKGROUND = 0xFF0A0F14;
    private static final int GLYPH_W = 8, GLYPH_H = 16;

    private record Key(UUID id, int displayId) {}

    private static final Map<Key, PhoneScreenTexture> CACHE = new HashMap<>();
    private static boolean[][] glyphs;
    private static long frame;

    private final ResourceLocation location;
    private DynamicTexture texture;
    private int width, height;
    private long signature = Long.MIN_VALUE;
    private long lastUsed;
    private int[] canvas = new int[0];

    private PhoneScreenTexture(Key key) {
        this.location = ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID,
                "dynamic/ephone_" + key.id().toString().replace("-", "") + "_" + key.displayId());
    }

    /**
     * The texture showing phone {@code id}'s display {@code displayId} on a
     * {@code tier} panel, or null while this client has no picture for it yet.
     * Render thread.
     */
    @Nullable
    public static ResourceLocation get(UUID id, int displayId, PanelTier tier, PanelTier.Tone tone) {
        ClientDisplayCache.Entry entry = ClientDisplayCache.entry(id, displayId);
        if (!entry.hasKeyframe) return null;
        Key key = new Key(id, displayId);
        PhoneScreenTexture t = CACHE.computeIfAbsent(key, PhoneScreenTexture::new);
        t.lastUsed = frame;
        t.update(entry.display, tier, tier == PanelTier.PHOSPHOR ? tone : null);
        return t.location;
    }

    /** Once per frame: forget textures nobody drew for a while. */
    public static void endFrame() {
        frame++;
        if (frame % 600 != 0) return;
        for (Iterator<PhoneScreenTexture> it = CACHE.values().iterator(); it.hasNext(); ) {
            PhoneScreenTexture t = it.next();
            if (frame - t.lastUsed > 1200) {
                t.close();
                it.remove();
            }
        }
    }

    public static void clearAll() {
        for (PhoneScreenTexture t : CACHE.values()) t.close();
        CACHE.clear();
    }

    /** The canvas size for a tier: the panel's portrait resolution. */
    public static int[] size(PanelTier tier) {
        var mode = tier.modes().phone();
        return new int[]{mode.gfxWidth(), mode.gfxHeight(), mode.scale()};
    }

    private void update(TerminalDisplay d, PanelTier tier, @Nullable PanelTier.Tone tone) {
        int[] sz = size(tier);
        int w = sz[0], h = sz[1], scale = sz[2];
        boolean blink = (System.currentTimeMillis() / 500) % 2 == 0;
        long sig;
        synchronized (d) {
            sig = signature(d, blink) * 31 + w * 7919L + h + (tone == null ? 0 : tone.ordinal() + 1);
        }
        if (texture != null && w == width && h == height && sig == signature) return;
        if (texture == null || w != width || h != height) {
            if (texture != null) Minecraft.getInstance().getTextureManager().release(location);
            texture = new DynamicTexture(w, h, false);
            Minecraft.getInstance().getTextureManager().register(location, texture);
            width = w;
            height = h;
            canvas = new int[w * h];
        }
        signature = sig;
        synchronized (d) {
            compose(d, scale, blink);
        }
        NativeImage img = texture.getPixels();
        if (img == null) return;
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                int argb = canvas[y * w + x];
                if (tone != null) argb = tone.map(argb);
                img.setPixelRGBA(x, y, (argb & 0xFF00FF00) | ((argb >>> 16) & 0xFF) | ((argb & 0xFF) << 16));
            }
        }
        texture.upload();
    }

    private static long signature(TerminalDisplay d, boolean blink) {
        long s = java.util.Arrays.hashCode(d.getCellData());
        s = s * 31 + d.getWidth() * 1000L + d.getHeight();
        s = s * 31 + d.getDisplayMode();
        s = s * 31 + d.getPixelDirtyCounter() * 17L + d.getPaletteDirtyCounter();
        s = s * 31 + d.getGfxWidth() * 1000L + d.getGfxHeight() + d.getPixelFormat();
        if (d.isCursorVisible()) s = s * 31 + d.getCursorX() * 1000L + d.getCursorY() + (blink ? 1 : 0);
        return s;
    }

    /** Draw the display into {@link #canvas}. Caller holds the display's monitor. */
    private void compose(TerminalDisplay d, int scale, boolean blink) {
        java.util.Arrays.fill(canvas, BACKGROUND);
        int mode = d.getDisplayMode();
        if (mode >= 1) drawGraphics(d);
        if (mode == 0 || mode == 2) drawText(d, scale, mode == 2, blink);
    }

    private void drawGraphics(TerminalDisplay d) {
        int gw = d.getGfxWidth(), gh = d.getGfxHeight();
        byte[] px = d.getPixelData();
        int fmt = d.getPixelFormat();
        int[] pal = d.getPalette();
        if (gw <= 0 || gh <= 0 || px == null || px.length < gw * gh * TerminalDisplay.bytesPerPixel(fmt)) return;
        if (fmt == TerminalDisplay.PIXEL_FORMAT_INDEXED8 && pal == null) return;
        // Letterbox: the largest rectangle with the picture's aspect.
        double s = Math.min(width / (double) gw, height / (double) gh);
        int dw = Math.max(1, (int) (gw * s)), dh = Math.max(1, (int) (gh * s));
        int ox = (width - dw) / 2, oy = (height - dh) / 2;
        for (int y = 0; y < dh; y++) {
            int sy = Math.min(gh - 1, (int) (y / s));
            for (int x = 0; x < dw; x++) {
                int sx = Math.min(gw - 1, (int) (x / s));
                int i = sy * gw + sx;
                int argb = switch (fmt) {
                    case TerminalDisplay.PIXEL_FORMAT_RGBA8888 -> {
                        int o = i * 4;
                        yield ((px[o + 3] & 0xFF) << 24) | ((px[o] & 0xFF) << 16) | ((px[o + 1] & 0xFF) << 8) | (px[o + 2] & 0xFF);
                    }
                    case TerminalDisplay.PIXEL_FORMAT_RGB565 -> {
                        int p = (px[i * 2] & 0xFF) | ((px[i * 2 + 1] & 0xFF) << 8);
                        int r = (p >> 11) & 0x1F, g = (p >> 5) & 0x3F, b = p & 0x1F;
                        yield 0xFF000000 | (((r << 3) | (r >> 2)) << 16) | (((g << 2) | (g >> 4)) << 8) | ((b << 3) | (b >> 2));
                    }
                    default -> pal[px[i] & 0xFF] | 0xFF000000;
                };
                if ((argb >>> 24) == 0) continue;
                canvas[(oy + y) * width + ox + x] = argb | 0xFF000000;
            }
        }
    }

    private void drawText(TerminalDisplay d, int scale, boolean overlay, boolean blink) {
        boolean[][] font = glyphs();
        int cols = d.getWidth(), rows = d.getHeight();
        if (cols <= 0 || rows <= 0) return;
        // Cells are 8x16 times the panel scale; a grid bigger than the canvas (before the
        // console is resized) is shrunk to fit.
        double s = Math.min(scale, Math.min(width / (double) (cols * GLYPH_W), height / (double) (rows * GLYPH_H)));
        int cx = d.getCursorX(), cy = d.getCursorY();
        boolean cursor = d.isCursorVisible() && blink;
        int cellW = Math.max(1, (int) Math.round(GLYPH_W * s)), cellH = Math.max(1, (int) Math.round(GLYPH_H * s));
        for (int row = 0; row < rows; row++) {
            for (int col = 0; col < cols; col++) {
                int ch = d.getCharAt(col, row) & 0xFF;
                int attr = d.getAttrAt(col, row) & 0xFF;
                int flags = d.getFlagsAt(col, row);
                int fgIdx = attr & 0x0F, bgIdx = (attr >> 4) & 0x0F;
                if ((flags & 0x01) != 0 && fgIdx < 8) fgIdx += 8;
                int fg = PALETTE[fgIdx];
                int bg = bgIdx != 0 ? PALETTE[bgIdx] : 0;
                if ((flags & 0x08) != 0) {
                    int t = fg;
                    fg = bgIdx != 0 ? PALETTE[bgIdx] : BACKGROUND;
                    bg = t;
                }
                if (cursor && col == cx && row == cy) {
                    bg = 0xFFC0C0C0;
                    fg = PALETTE[0];
                }
                boolean hidden = (flags & 0x04) != 0 && !blink;
                boolean underline = (flags & 0x02) != 0;
                int x0 = (int) Math.round(col * GLYPH_W * s), y0 = (int) Math.round(row * GLYPH_H * s);
                boolean[] g = ch > 0x20 && !hidden ? font[ch] : null;
                for (int py = 0; py < cellH; py++) {
                    int y = y0 + py;
                    if (y >= height) break;
                    int gy = Math.min(GLYPH_H - 1, (int) (py / s));
                    for (int px = 0; px < cellW; px++) {
                        int x = x0 + px;
                        if (x >= width) break;
                        int gx = Math.min(GLYPH_W - 1, (int) (px / s));
                        boolean on = g != null && g[gy * GLYPH_W + gx] || underline && !hidden && gy == 14;
                        if (on) canvas[y * width + x] = fg;
                        else if (bg != 0) canvas[y * width + x] = bg;
                        else if (!overlay) canvas[y * width + x] = BACKGROUND;
                    }
                }
            }
        }
    }

    /** The terminal font's 256 glyphs (8x16, 16 per row in terminal_font.png), lit where opaque. */
    private static boolean[][] glyphs() {
        if (glyphs != null) return glyphs;
        boolean[][] g = new boolean[256][GLYPH_W * GLYPH_H];
        try (InputStream in = Minecraft.getInstance().getResourceManager()
                .open(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "textures/terminal_font.png"));
             NativeImage img = NativeImage.read(in)) {
            for (int c = 0; c < 256; c++) {
                int gx = (c % 16) * GLYPH_W, gy = (c / 16) * GLYPH_H;
                for (int y = 0; y < GLYPH_H; y++)
                    for (int x = 0; x < GLYPH_W; x++) {
                        int abgr = img.getPixelRGBA(gx + x, gy + y);
                        int a = (abgr >>> 24) & 0xFF, lum = Math.max(abgr & 0xFF, (abgr >> 8) & 0xFF);
                        g[c][y * GLYPH_W + x] = a > 127 && lum > 64;
                    }
            }
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("ePhone: can't read the terminal font", e);
        }
        glyphs = g;
        return g;
    }

    @Override
    public void close() {
        if (texture != null) Minecraft.getInstance().getTextureManager().release(location);
        texture = null;
    }
}
//?}
