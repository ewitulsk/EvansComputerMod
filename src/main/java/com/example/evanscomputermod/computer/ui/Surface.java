package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * One surface (§5): a tree or a pixel buffer, with a role. Everything an app
 * sends for a surface is pending until its {@code Commit}, which applies it
 * atomically: TreeOps, pixel rows, and the size of the configure it acked.
 */
final class Surface {
    final UiConnection conn;
    final int id;
    final UiProto.SurfaceKind kind;
    final UiProto.SurfaceRole role;
    final UiProto.Layer layer;
    /** Popups: the parent surface and the offset in it. Layers: the position on the display. */
    final Surface parent;
    final int px, py;
    String title;
    int minW, minH, maxW, maxH;

    // --- tree surfaces
    final NodeTree tree;
    final List<UiProto.TreeOp> pendingOps = new ArrayList<>();
    long pendingOpBytes;

    // --- pixel surfaces: buffer size, format (TerminalDisplay codes), whole-number scale
    int bw, bh, format, pscale = 1;
    byte[] pendingRaw;
    final Box.Region pendingPixels = new Box.Region();
    /** Committed pixels (premultiplied ARGB, bw x bh). */
    int[] argb;
    /** Palette for indexed8 (legacy windows carry the program's own); null = RGB332. */
    int[] palette;
    boolean resizePending;
    int newBw, newBh, newFormat, newScale;

    // --- configure / commit
    int nextSerial = 1;
    /** Configures sent and not yet acked: serial -> {w, h, states}. */
    final Map<Integer, int[]> configures = new LinkedHashMap<>();
    int[] acked;
    /** Size (lu) and states of the last applied configure. */
    int contentW, contentH, states;
    /** Has a commit after an acked configure been applied (the surface may show). */
    boolean mapped;
    /** Committed content changed since the last composite: surface-lu damage. */
    final Box.Region damage = new Box.Region();
    boolean damageAll;
    boolean frameRequested;
    /** Popups: dismissed by a click outside. */
    boolean popupDone;
    /** The window of a window-role surface. */
    UiWindow window;
    /** Commits deferred by the tree-ops rate limit (merged into the next one). */
    boolean commitDeferred;
    /** Bumped whenever {@link #argb} changes (the scene exporter diffs pixel tiles only then). */
    int pixelVersion;

    Surface(UiConnection conn, UiProto.SurfaceCreate c, Surface parent) {
        this.conn = conn;
        this.id = c.surface();
        this.kind = c.kind();
        this.role = c.role();
        this.layer = c.layer();
        this.parent = parent;
        this.px = c.x();
        this.py = c.y();
        this.title = c.title();
        this.tree = kind == UiProto.SurfaceKind.TREE ? new NodeTree() : null;
        if (kind == UiProto.SurfaceKind.PIXELS) {
            setBuffer(c.w(), c.h(), c.format().code, c.scale());
        } else {
            contentW = c.w();
            contentH = c.h();
        }
    }

    /**
     * A surface of a client's scene mirror ({@link UiSceneMirror}): no
     * connection, no pending state; pixel surfaces get their buffer from
     * {@link #setMirrorBuffer}.
     */
    Surface(int id, UiProto.SurfaceKind kind, UiProto.SurfaceRole role, UiProto.Layer layer, Surface parent, int px, int py) {
        this.conn = null;
        this.id = id;
        this.kind = kind;
        this.role = role;
        this.layer = layer;
        this.parent = parent;
        this.px = px;
        this.py = py;
        this.title = "";
        this.tree = kind == UiProto.SurfaceKind.TREE ? new NodeTree() : null;
    }

    /** Mirror side: a {@code w x h} premultiplied buffer at {@code scale}, black until tiles arrive. */
    void setMirrorBuffer(int w, int h, int scale) {
        bw = w;
        bh = h;
        pscale = Math.max(1, scale);
        argb = new int[Math.max(1, w * h)];
        java.util.Arrays.fill(argb, 0xFF000000);
        pixelVersion++;
    }

    void setBuffer(int w, int h, int fmt, int scale) {
        bw = w;
        bh = h;
        format = fmt;
        pscale = Math.max(1, scale);
        pendingRaw = new byte[w * h * TerminalDisplay.bytesPerPixel(fmt)];
        argb = new int[Math.max(1, w * h)];
        java.util.Arrays.fill(argb, 0xFF000000);
        pendingPixels.clear();
        damageAll = true;
        pixelVersion++;
    }

    boolean isPixels() {
        return kind == UiProto.SurfaceKind.PIXELS;
    }

    /** Natural size in lu: a tree's configured size, a pixel buffer's size times its scale. */
    int naturalW() {
        return isPixels() ? bw * pscale : contentW;
    }

    int naturalH() {
        return isPixels() ? bh * pscale : contentH;
    }

    long bytes() {
        return isPixels() ? (long) bw * bh * (4 + TerminalDisplay.bytesPerPixel(format)) : 0;
    }

    int sendConfigure(int w, int h, int st) {
        int serial = nextSerial++;
        configures.put(serial, new int[]{w, h, st});
        while (configures.size() > 64) configures.remove(configures.keySet().iterator().next());
        conn.queue(new UiProto.Configure(id, serial, w, h, st));
        return serial;
    }
}
