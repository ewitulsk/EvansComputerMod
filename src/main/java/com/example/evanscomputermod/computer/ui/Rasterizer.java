package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.font.UiFonts;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.util.Arrays;
import java.util.List;

/**
 * The reference rasterizer for display-list trees and pixel buffers (§6,
 * §14 phase 1). It draws into premultiplied ARGB {@link Target}s in device
 * pixels, with integer arithmetic only, so its output is the same on every
 * JVM: golden images (src/test/resources/ui-golden) hold it to that, and the
 * client will run this same class in phase 2.
 *
 * <p>Rules the protocol leaves to the renderer, fixed here:
 * <ul>
 *   <li>No anti-aliasing. A pixel belongs to a shape when its centre is
 *       inside it (rounded corners, thick lines), so edges are pixel-crisp at
 *       every whole-number scale.</li>
 *   <li>A group with opacity below 255 is drawn into an offscreen layer that
 *       is then blended once, so overlapping children don't double-blend.</li>
 *   <li>Lines are rasterized into a coverage mask first, for the same reason.</li>
 *   <li>Fully opaque fills and pixel rows are copied, not blended.</li>
 * </ul>
 */
public final class Rasterizer {
    private Rasterizer() {}

    /** A premultiplied ARGB buffer covering device pixels {@code [ox, ox + w) x [oy, oy + h)}. */
    public static final class Target {
        public final int[] px;
        public final int w, h, ox, oy;

        public Target(int[] px, int w, int h, int ox, int oy) {
            this.px = px;
            this.w = w;
            this.h = h;
            this.ox = ox;
            this.oy = oy;
        }

        public static Target of(Box b) {
            return new Target(new int[Math.max(1, b.w() * b.h())], b.w(), b.h(), b.x0(), b.y0());
        }

        public Box box() {
            return new Box(ox, oy, ox + w, oy + h);
        }

        void blend(int x, int y, int c) {
            int i = (y - oy) * w + (x - ox);
            px[i] = Px.over(c, px[i]);
        }

        /** Fill {@code b} (already clipped to this target) with {@code c}. */
        public void fill(Box b, int c) {
            if (b.isEmpty() || c == 0) return;
            boolean opaque = (c >>> 24) == 255;
            for (int y = b.y0(); y < b.y1(); y++) {
                int row = (y - oy) * w - ox;
                if (opaque) {
                    Arrays.fill(px, row + b.x0(), row + b.x1(), c);
                } else {
                    for (int x = b.x0(); x < b.x1(); x++) px[row + x] = Px.over(c, px[row + x]);
                }
            }
        }

        /** Overwrite {@code b} with {@code c} (no blending). */
        public void clear(Box b, int c) {
            for (int y = b.y0(); y < b.y1(); y++) {
                int row = (y - oy) * w - ox;
                Arrays.fill(px, row + b.x0(), row + b.x1(), c);
            }
        }

        /** Blend a layer (same coordinate system) onto this target with an extra opacity. */
        public void composite(Target layer, int opacity, Box clip) {
            Box b = clip.intersect(layer.box()).intersect(box());
            for (int y = b.y0(); y < b.y1(); y++) {
                int lrow = (y - layer.oy) * layer.w - layer.ox;
                int row = (y - oy) * w - ox;
                for (int x = b.x0(); x < b.x1(); x++) {
                    int c = layer.px[lrow + x];
                    if (c == 0) continue;
                    px[row + x] = Px.over(Px.fade(c, opacity), px[row + x]);
                }
            }
        }
    }

    /** Where resources come from (one connection's images and canvases). */
    public interface Resources {
        /** Premultiplied pixels of resource {@code id}, or null if unknown or not loaded yet. */
        int[] pixels(int id);

        int width(int id);

        int height(int id);
    }

    // ------------------------------------------------------------------ trees

    /**
     * Draw the children of {@code group} (normally a surface's root) with
     * their parent's content origin at device {@code (tx, ty)} and {@code s}
     * device pixels per lu, clipped to {@code clip}.
     */
    public static void drawChildren(NodeTree.Node group, int tx, int ty, int s, Box clip, Target t, Resources res) {
        for (NodeTree.Node c : group.children()) draw(c, tx, ty, s, clip, t, res);
    }

    /** Draw one node (and its subtree) in its parent's content space. */
    public static void draw(NodeTree.Node n, int tx, int ty, int s, Box clip, Target t, Resources res) {
        if (clip.isEmpty()) return;
        switch (n.kind) {
            case GROUP -> drawGroup(n, tx, ty, s, clip, t, res);
            case RECT -> drawRect(Box.xywh(tx + n.x * s, ty + n.y * s, n.w * s, n.h * s), n.radius * s, n.borderW * s,
                    Px.premul(n.fill), Px.premul(n.borderColor), clip, t);
            case TEXT -> {
                UiFonts.Face f = n.face();
                if (f != null) drawText(n.text, f, tx + n.x * s, ty + n.y * s, s, Px.premul(n.color), clip, t);
            }
            case IMAGE -> drawImage(n, tx, ty, s, clip, t, res);
            case CANVAS -> drawCanvas(n, tx, ty, s, clip, t, res);
            case LINE -> drawLine(n.points, tx, ty, s, Math.max(1, n.width * s), Px.premul(n.color), clip, t);
        }
    }

    private static void drawGroup(NodeTree.Node n, int tx, int ty, int s, Box clip, Target t, Resources res) {
        if (n.opacity == 0 || n.children().isEmpty()) return;
        int gx = tx + n.x * s, gy = ty + n.y * s;
        Box c = n.clip ? clip.intersect(Box.xywh(gx, gy, n.w * s, n.h * s)) : clip;
        if (c.isEmpty()) return;
        int cs = s * n.scale;
        if (n.opacity >= 255) {
            for (NodeTree.Node ch : n.children()) draw(ch, gx, gy, cs, c, t, res);
            return;
        }
        Box area = c.intersect(NodeTree.localBox(n).scale(s).translate(tx, ty)).intersect(t.box());
        if (area.isEmpty()) return;
        Target layer = Target.of(area);
        for (NodeTree.Node ch : n.children()) draw(ch, gx, gy, cs, area, layer, res);
        t.composite(layer, n.opacity, area);
    }

    /**
     * A rectangle with optional rounded corners (radius in device pixels) and
     * an inner border {@code bw} pixels wide. A pixel is inside a corner when
     * its centre is within the corner circle.
     */
    public static void drawRect(Box r, int radius, int bw, int fill, int border, Box clip, Target t) {
        Box vis = r.intersect(clip).intersect(t.box());
        if (vis.isEmpty() || (fill == 0 && (border == 0 || bw == 0))) return;
        int rad = Math.min(radius, Math.min(r.w(), r.h()) / 2);
        if (rad <= 0 && bw <= 0) {
            t.fill(vis, fill);
            return;
        }
        Box inner = new Box(r.x0() + bw, r.y0() + bw, r.x1() - bw, r.y1() - bw);
        int irad = Math.max(0, rad - bw);
        if (rad <= 0) {
            // Square corners: the border is four bands, the fill the inner box.
            if (bw > 0 && border != 0) {
                t.fill(vis.intersect(new Box(r.x0(), r.y0(), r.x1(), inner.y0())), border);
                t.fill(vis.intersect(new Box(r.x0(), Math.max(inner.y0(), inner.y1()), r.x1(), r.y1())), border);
                t.fill(vis.intersect(new Box(r.x0(), inner.y0(), inner.x0(), inner.y1())), border);
                t.fill(vis.intersect(new Box(Math.max(inner.x0(), inner.x1()), inner.y0(), r.x1(), inner.y1())), border);
            }
            t.fill(vis.intersect(inner), fill);
            return;
        }
        for (int y = vis.y0(); y < vis.y1(); y++) {
            for (int x = vis.x0(); x < vis.x1(); x++) {
                if (!inRounded(r, rad, x, y)) continue;
                boolean in = bw <= 0 || (!inner.isEmpty() && inRounded(inner, irad, x, y));
                int c = in ? fill : border;
                if (c != 0) t.blend(x, y, c);
            }
        }
    }

    /** Is pixel {@code (x, y)}'s centre inside box {@code b} with corner radius {@code r}? Exact, in half pixels. */
    static boolean inRounded(Box b, int r, int x, int y) {
        if (!b.contains(x, y)) return false;
        if (r <= 0) return true;
        int cx, cy;
        if (x < b.x0() + r) cx = b.x0() + r;
        else if (x >= b.x1() - r) cx = b.x1() - r;
        else return true;
        if (y < b.y0() + r) cy = b.y0() + r;
        else if (y >= b.y1() - r) cy = b.y1() - r;
        else return true;
        long dx = 2L * cx - (2L * x + 1), dy = 2L * cy - (2L * y + 1);
        return dx * dx + dy * dy <= 4L * r * r;
    }

    /**
     * One line of text, its line box's top-left at device {@code (x, y)}, with
     * {@code s} device pixels per lu. Each glyph pixel is a square of
     * {@code face.scale() * s} device pixels.
     */
    public static void drawText(String text, UiFonts.Face f, int x, int y, int s, int color, Box clip, Target t) {
        if (text.isEmpty() || color == 0) return;
        UiFonts.Design d = f.design();
        int gs = f.scale() * s;
        Box lineBox = Box.xywh(x, y, f.textWidth(text) * s, d.height * gs);
        Box vis = lineBox.intersect(clip).intersect(t.box());
        if (vis.isEmpty()) return;
        int pen = x;
        for (int i = 0; i < text.length(); ) {
            int cp = text.codePointAt(i);
            i += Character.charCount(cp);
            int gw = d.width(cp);
            int adv = d.advance(cp) * gs;
            if (pen + gw * gs > vis.x0() && pen < vis.x1()) {
                for (int gy = 0; gy < d.height; gy++) {
                    int bits = d.row(cp, gy);
                    if (bits == 0) continue;
                    int py = y + gy * gs;
                    if (py + gs <= vis.y0() || py >= vis.y1()) continue;
                    for (int gx = 0; gx < gw; gx++) {
                        if ((bits & (1 << gx)) == 0) continue;
                        t.fill(Box.xywh(pen + gx * gs, py, gs, gs).intersect(vis), color);
                    }
                }
            }
            pen += adv;
            if (pen >= vis.x1()) break;
        }
    }

    private static void drawImage(NodeTree.Node n, int tx, int ty, int s, Box clip, Target t, Resources res) {
        int[] img = res.pixels(n.resource);
        if (img == null) return;
        int iw = res.width(n.resource), ih = res.height(n.resource);
        // An empty src rect (the prop's default) means the whole resource.
        UiProto.Rect src = n.src != null && n.src.w() > 0 && n.src.h() > 0 ? n.src : new UiProto.Rect(0, 0, iw, ih);
        int sx = Math.max(0, src.x()), sy = Math.max(0, src.y());
        int sw = Math.min(iw - sx, src.w()), sh = Math.min(ih - sy, src.h());
        if (sw <= 0 || sh <= 0) return;
        Box dst = Box.xywh(tx + n.x * s, ty + n.y * s, n.w * s, n.h * s);
        Box vis = dst.intersect(clip).intersect(t.box());
        if (vis.isEmpty()) return;
        int dw = dst.w(), dh = dst.h();
        boolean linear = n.sampling == UiProto.Sampling.LINEAR;
        for (int y = vis.y0(); y < vis.y1(); y++) {
            int dy = y - dst.y0();
            for (int x = vis.x0(); x < vis.x1(); x++) {
                int dx = x - dst.x0();
                int c;
                if (!linear) {
                    int u = sx + (int) (((2L * dx + 1) * sw) / (2L * dw));
                    int v = sy + (int) (((2L * dy + 1) * sh) / (2L * dh));
                    c = img[v * iw + u];
                } else {
                    c = bilinear(img, iw, sx, sy, sw, sh, dx, dy, dw, dh);
                }
                c = Px.tint(c, n.tint);
                if (c != 0) t.blend(x, y, c);
            }
        }
    }

    /** Bilinear sample (16.16 fixed point, edges clamped) of the source rect at destination pixel centre (dx, dy). */
    static int bilinear(int[] img, int iw, int sx, int sy, int sw, int sh, int dx, int dy, int dw, int dh) {
        long fu = ((2L * dx + 1) * sw * 65536L) / (2L * dw) - 32768L;
        long fv = ((2L * dy + 1) * sh * 65536L) / (2L * dh) - 32768L;
        fu = Math.max(0, Math.min(fu, (sw - 1) * 65536L));
        fv = Math.max(0, Math.min(fv, (sh - 1) * 65536L));
        int u0 = (int) (fu >> 16), v0 = (int) (fv >> 16);
        int au = (int) ((fu & 0xFFFF) >> 8), av = (int) ((fv & 0xFFFF) >> 8); // 0..255
        int u1 = Math.min(u0 + 1, sw - 1), v1 = Math.min(v0 + 1, sh - 1);
        int c00 = img[(sy + v0) * iw + sx + u0], c10 = img[(sy + v0) * iw + sx + u1];
        int c01 = img[(sy + v1) * iw + sx + u0], c11 = img[(sy + v1) * iw + sx + u1];
        int out = 0;
        for (int shift = 0; shift <= 24; shift += 8) {
            int a = (c00 >>> shift) & 0xFF, b = (c10 >>> shift) & 0xFF;
            int c = (c01 >>> shift) & 0xFF, d = (c11 >>> shift) & 0xFF;
            int top = a * (256 - au) + b * au;
            int bot = c * (256 - au) + d * au;
            int v = (top * (256 - av) + bot * av + 32768) >> 16;
            out |= Math.min(255, v) << shift;
        }
        return out;
    }

    private static void drawCanvas(NodeTree.Node n, int tx, int ty, int s, Box clip, Target t, Resources res) {
        int[] buf = res.pixels(n.resource);
        if (buf == null) return;
        Box dst = Box.xywh(tx + n.x * s, ty + n.y * s, n.w * s, n.h * s);
        drawPixels(buf, res.width(n.resource), res.height(n.resource), dst.x0(), dst.y0(), n.scale * s,
                dst.intersect(clip), t);
    }

    /**
     * A premultiplied pixel buffer at device {@code (x, y)}, each pixel a
     * {@code ps x ps} square, clipped to {@code clip}. Opaque spans are
     * copied without blending.
     */
    public static void drawPixels(int[] buf, int bw, int bh, int x, int y, int ps, Box clip, Target t) {
        Box vis = Box.xywh(x, y, bw * ps, bh * ps).intersect(clip).intersect(t.box());
        if (vis.isEmpty()) return;
        for (int Y = vis.y0(); Y < vis.y1(); Y++) {
            int srow = ((Y - y) / ps) * bw;
            int row = (Y - t.oy) * t.w - t.ox;
            for (int X = vis.x0(); X < vis.x1(); X++) {
                int c = buf[srow + (X - x) / ps];
                int a = c >>> 24;
                if (a == 255) t.px[row + X] = c;
                else if (a != 0) t.px[row + X] = Px.over(c, t.px[row + X]);
            }
        }
    }

    /** A polyline {@code thick} device pixels wide (square brush), rasterized into a mask and blended once. */
    public static void drawLine(List<UiProto.Point> pts, int tx, int ty, int s, int thick, int color, Box clip, Target t) {
        if (pts.isEmpty() || color == 0) return;
        int x0 = Integer.MAX_VALUE, y0 = Integer.MAX_VALUE, x1 = Integer.MIN_VALUE, y1 = Integer.MIN_VALUE;
        int[] xs = new int[pts.size()], ys = new int[pts.size()];
        for (int i = 0; i < pts.size(); i++) {
            xs[i] = tx + pts.get(i).x() * s;
            ys[i] = ty + pts.get(i).y() * s;
            x0 = Math.min(x0, xs[i]);
            y0 = Math.min(y0, ys[i]);
            x1 = Math.max(x1, xs[i]);
            y1 = Math.max(y1, ys[i]);
        }
        int half = thick / 2;
        Box area = new Box(x0 - half, y0 - half, x1 - half + thick, y1 - half + thick).intersect(clip).intersect(t.box());
        if (area.isEmpty()) return;
        boolean[] mask = new boolean[area.w() * area.h()];
        if (pts.size() == 1) {
            stamp(mask, area, xs[0], ys[0], half, thick);
        }
        for (int i = 0; i + 1 < pts.size(); i++) {
            int ax = xs[i], ay = ys[i], bx = xs[i + 1], by = ys[i + 1];
            int dx = Math.abs(bx - ax), dy = -Math.abs(by - ay);
            int sxs = ax < bx ? 1 : -1, sys = ay < by ? 1 : -1;
            int err = dx + dy;
            while (true) {
                stamp(mask, area, ax, ay, half, thick);
                if (ax == bx && ay == by) break;
                int e2 = 2 * err;
                if (e2 >= dy) {
                    err += dy;
                    ax += sxs;
                }
                if (e2 <= dx) {
                    err += dx;
                    ay += sys;
                }
            }
        }
        boolean opaque = (color >>> 24) == 255;
        for (int y = area.y0(); y < area.y1(); y++) {
            int mrow = (y - area.y0()) * area.w();
            int row = (y - t.oy) * t.w - t.ox;
            for (int x = area.x0(); x < area.x1(); x++) {
                if (!mask[mrow + x - area.x0()]) continue;
                t.px[row + x] = opaque ? color : Px.over(color, t.px[row + x]);
            }
        }
    }

    private static void stamp(boolean[] mask, Box area, int cx, int cy, int half, int thick) {
        int bx0 = Math.max(area.x0(), cx - half), by0 = Math.max(area.y0(), cy - half);
        int bx1 = Math.min(area.x1(), cx - half + thick), by1 = Math.min(area.y1(), cy - half + thick);
        for (int y = by0; y < by1; y++) {
            int row = (y - area.y0()) * area.w() - area.x0();
            for (int x = bx0; x < bx1; x++) mask[row + x] = true;
        }
    }

    // ------------------------------------------------------------------ hit testing

    /**
     * The topmost node with a non-zero {@code hit_id} under surface-lu point
     * {@code (px, py)} among {@code group}'s descendants, or 0. Uses the same
     * geometry as drawing (offsets, scale, clips); text is hit on its line box.
     */
    public static int hitTest(NodeTree.Node group, int px, int py) {
        return hit(group, 0, 0, 1, null, px, py);
    }

    private static int hit(NodeTree.Node g, int tx, int ty, int s, Box clip, int px, int py) {
        List<NodeTree.Node> ch = g.children();
        int gx = tx + g.x * s, gy = ty + g.y * s;
        int cs = s * g.scale;
        if (g.id == 0) {
            gx = tx;
            gy = ty;
            cs = s;
        }
        Box c = clip;
        if (g.clip && g.id != 0) {
            Box gb = Box.xywh(gx, gy, g.w * s, g.h * s);
            c = c == null ? gb : c.intersect(gb);
        }
        if (c != null && !c.contains(px, py)) return 0;
        if (g.opacity == 0) return 0;
        for (int i = ch.size() - 1; i >= 0; i--) {
            NodeTree.Node n = ch.get(i);
            if (n.kind == UiProto.NodeKind.GROUP) {
                int h = hit(n, gx, gy, cs, c, px, py);
                if (h != 0) return h;
                if (n.hitId != 0) {
                    Box b = Box.xywh(gx + n.x * cs, gy + n.y * cs, n.w * cs, n.h * cs);
                    if (b.contains(px, py)) return n.hitId;
                }
                continue;
            }
            if (n.hitId == 0) continue;
            Box b = NodeTree.localBox(n).scale(cs).translate(gx, gy);
            if (b.contains(px, py)) return n.hitId;
        }
        return 0;
    }
}
