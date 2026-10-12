package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.font.UiFonts;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Function;

/**
 * How a scene becomes pixels: surface placement in device pixels, the
 * drawing order of layers, windows, popups and safe mode's bar, and the
 * compositing of damaged rectangles with the {@link Rasterizer}. The
 * server's {@link Compositor} and the client's {@link UiSceneMirror} both
 * draw through this one class (UI_PROTOCOL.md §14 phase 2a), so the same
 * scene at the same time gives the same pixels on both sides.
 *
 * <p>Plain Java with no server state: everything it needs is passed in.
 */
public final class SceneRenderer {
    private SceneRenderer() {}

    /** The colour behind everything (and a scaled pixel window's letterbox). */
    public static final int BG = 0xFF000000;

    /** Device-pixel placement of a surface: its frame (clip) and content origin, k device px per content unit. */
    static final class Geo {
        final Box frame;
        final int ox, oy, k;

        Geo(Box frame, int ox, int oy, int k) {
            this.frame = frame;
            this.ox = ox;
            this.oy = oy;
            this.k = k;
        }
    }

    /** One thing to draw, bottom to top: a surface (with its window alpha), or safe mode's bar ({@code surface == null}). */
    static final class Item {
        final Surface surface;
        final Geo geo;
        final int alpha;

        Item(Surface surface, Geo geo, int alpha) {
            this.surface = surface;
            this.geo = geo;
            this.alpha = alpha;
        }
    }

    /** What a scene is made of, as the drawing order needs it. */
    record Scene(UiDisplay display, List<Surface> layers, List<UiWindow> stack, List<Surface> popups,
                 boolean safeMode, UiWindow focused) {}

    static Box displayBox(UiDisplay d) {
        return new Box(0, 0, d.deviceW(), d.deviceH());
    }

    /** Where surface {@code s} is at time {@code now}, or null if it has no place. */
    static Geo geo(UiDisplay display, Surface s, long now) {
        int sc = display.scale();
        if (s.window != null) {
            UiWindow w = s.window;
            int[] r = w.rect(now);
            if (s.isPixels()) {
                Box frame = Box.xywh(r[0] * sc, r[1] * sc, r[2] * sc, r[3] * sc);
                int k = Math.max(1, Math.min(frame.w() / Math.max(1, s.bw), frame.h() / Math.max(1, s.bh)));
                int ox = frame.x0() + (frame.w() - s.bw * k) / 2, oy = frame.y0() + (frame.h() - s.bh * k) / 2;
                if (frame.w() < s.bw * k) ox = frame.x0();
                if (frame.h() < s.bh * k) oy = frame.y0();
                return new Geo(frame, ox, oy, k);
            }
            Box frame = Box.xywh(r[0] * sc, r[1] * sc, Math.min(r[2], s.contentW) * sc, Math.min(r[3], s.contentH) * sc);
            return new Geo(frame, frame.x0(), frame.y0(), sc);
        }
        int ox, oy;
        if (s.parent != null) {
            Geo pg = geo(display, s.parent, now);
            if (pg == null) return null;
            int pk = s.parent.isPixels() ? sc : pg.k;
            ox = pg.ox + s.px * pk;
            oy = pg.oy + s.py * pk;
        } else {
            ox = s.px * sc;
            oy = s.py * sc;
        }
        if (s.isPixels()) {
            int k = s.pscale * sc;
            return new Geo(Box.xywh(ox, oy, s.bw * k, s.bh * k), ox, oy, k);
        }
        return new Geo(Box.xywh(ox, oy, s.contentW * sc, s.contentH * sc), ox, oy, sc);
    }

    static boolean shown(Scene sc, UiWindow w) {
        if (!w.placed || !w.visible || !w.surface.mapped) return false;
        return !sc.safeMode() || w == sc.focused();
    }

    /** Everything to draw at time {@code now}, bottom to top. */
    static List<Item> drawOrder(Scene sc, long now) {
        UiDisplay display = sc.display();
        List<Item> out = new ArrayList<>();
        addLayer(sc, out, UiProto.Layer.BACKGROUND, now);
        addLayer(sc, out, UiProto.Layer.BOTTOM, now);
        UiWindow full = null;
        List<UiWindow> stack = sc.stack();
        for (int i = stack.size() - 1; i >= 0; i--) {
            UiWindow w = stack.get(i);
            if (shown(sc, w)) {
                if ((w.states & UiProto.WindowStates.FULLSCREEN) != 0) full = w;
                break;
            }
        }
        for (UiWindow w : stack) {
            if (w == full || !shown(sc, w)) continue;
            Geo g = geo(display, w.surface, now);
            if (g != null) out.add(new Item(w.surface, g, w.alpha(now)));
        }
        addLayer(sc, out, UiProto.Layer.TOP, now);
        if (full != null) {
            Geo g = geo(display, full.surface, now);
            int a = full.alpha(now);
            if (g != null) {
                // Exclusive fullscreen: an opaque pixel window covering the display is the
                // only thing below popups and overlays, so nothing underneath is drawn.
                if (full.surface.isPixels() && a == 255 && g.frame.contains(displayBox(display))) out.clear();
                out.add(new Item(full.surface, g, a));
            }
        }
        for (Surface p : sc.popups()) {
            if (!p.mapped || p.popupDone) continue;
            if (p.parent != null && p.parent.window != null && !shown(sc, p.parent.window)) continue;
            Geo g = geo(display, p, now);
            if (g != null) out.add(new Item(p, g, 255));
        }
        addLayer(sc, out, UiProto.Layer.OVERLAY, now);
        if (sc.safeMode()) {
            out.add(new Item(null, new Geo(new Box(0, 0, display.deviceW(), Compositor.SAFE_BAR_H * display.scale()), 0, 0,
                    display.scale()), 255));
        }
        return out;
    }

    private static void addLayer(Scene sc, List<Item> out, UiProto.Layer layer, long now) {
        if (sc.safeMode()) return; // the shell's layers are gone with it; nothing else makes them
        for (Surface s : sc.layers()) {
            if (s.layer != layer || !s.mapped) continue;
            Geo g = geo(sc.display(), s, now);
            if (g != null) out.add(new Item(s, g, 255));
        }
    }

    /**
     * Redraw {@code boxes} of the frame ({@code frame} premultiplied ARGB,
     * {@code rgba} its straight RGBA bytes, both the display's size) from
     * {@code order}. Each box is cleared and drawn from scratch, so how the
     * damage is cut into boxes never changes the result.
     */
    static void composite(UiDisplay display, int[] frame, byte[] rgba, List<Box> boxes, List<Item> order,
                          String safeText, Function<Surface, Rasterizer.Resources> res) {
        int W = display.deviceW(), H = display.deviceH();
        Rasterizer.Target t = new Rasterizer.Target(frame, W, H, 0, 0);
        for (Box b : boxes) {
            t.clear(b, BG);
            for (Item it : order) {
                Box clip = b.intersect(it.geo.frame);
                if (clip.isEmpty()) continue;
                if (it.surface == null) {
                    drawSafeBar(display, t, clip, safeText);
                    continue;
                }
                if (it.alpha <= 0) continue;
                if (it.alpha >= 255) {
                    drawSurface(it.surface, it.geo, clip, t, res);
                } else {
                    Rasterizer.Target layer = Rasterizer.Target.of(clip);
                    drawSurface(it.surface, it.geo, clip, layer, res);
                    t.composite(layer, it.alpha, clip);
                }
            }
            for (int y = b.y0(); y < b.y1(); y++) {
                for (int x = b.x0(); x < b.x1(); x++) Px.toRgbaBytes(frame[y * W + x], rgba, (y * W + x) * 4);
            }
        }
    }

    private static void drawSurface(Surface s, Geo g, Box clip, Rasterizer.Target t,
                                    Function<Surface, Rasterizer.Resources> res) {
        if (s.isPixels()) {
            if (s.window != null) t.fill(clip, BG); // letterbox of a scaled pixel window
            Rasterizer.drawPixels(s.argb, s.bw, s.bh, g.ox, g.oy, g.k, clip, t);
        } else {
            Rasterizer.drawChildren(s.tree.root(), g.ox, g.oy, g.k, clip, t, res.apply(s));
        }
    }

    static void drawSafeBar(UiDisplay display, Rasterizer.Target t, Box clip, String text) {
        int sc = display.scale();
        Rasterizer.drawRect(new Box(0, 0, display.deviceW(), Compositor.SAFE_BAR_H * sc), 0, 0, Px.premul(0x3A1D1DFF), 0, clip, t);
        UiFonts.Face f = UiFonts.face(UiProto.Font.UI, 12);
        Rasterizer.drawText(text == null ? "" : text, f, 4 * sc, sc, sc, Px.premul(0xF2E6E6FF), clip, t);
    }

    /**
     * Damage in a surface's own units (lu for trees, buffer pixels times
     * scale for pixel surfaces) as device pixels, clipped to its frame.
     */
    static Box surfaceDamage(UiDisplay display, Surface s, Box b, long now) {
        Geo g = geo(display, s, now);
        if (g == null) return Box.EMPTY;
        Box d = s.isPixels()
                ? new Box(g.ox + Math.floorDiv(b.x0(), s.pscale) * g.k, g.oy + Math.floorDiv(b.y0(), s.pscale) * g.k,
                g.ox + ceilDiv(b.x1(), s.pscale) * g.k, g.oy + ceilDiv(b.y1(), s.pscale) * g.k)
                : b.scale(g.k).translate(g.ox, g.oy);
        return d.intersect(g.frame).intersect(displayBox(display));
    }

    static int ceilDiv(int a, int b) {
        return -Math.floorDiv(-a, b);
    }
}
