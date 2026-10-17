package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.ui.proto.UiProtoException;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.LongFunction;

/**
 * A client's copy of one computer display's scene (§14 phase 2a): the
 * surfaces, windows, trees, pixel buffers and resources the server's
 * {@link SceneExporter} streams, kept up to date by applying its steps,
 * and drawn with the same {@link SceneRenderer} and {@link Rasterizer} as
 * the server, so the pixels match. Window transitions are animated here
 * from their start time, length and easing; the server sends nothing while
 * they run.
 *
 * <p>Plain Java with no client classes (tests drive it directly). Not
 * thread-safe: the client applies messages and renders on its main thread.
 */
public final class UiSceneMirror {
    /** Result of applying a message. */
    public enum Result { APPLIED, IGNORED, NEED_KEYFRAME }

    private final LongFunction<int[]> images;
    private UiDisplay display;
    private int epoch = -1;
    private long gen = -1;
    private boolean ready;
    /** Server clock minus client clock, from the last message (the server's times are used as they are). */
    private long clockOffset;
    private boolean clockSet;

    private final Map<Integer, Surface> surfaces = new LinkedHashMap<>();
    private final Map<Surface, Integer> scopes = new HashMap<>();
    private final Map<Integer, UiWindow> windows = new HashMap<>();
    private final List<UiWindow> stack = new ArrayList<>();
    private final List<Surface> layers = new ArrayList<>();
    private final List<Surface> popups = new ArrayList<>();
    private UiWindow focused;
    private boolean safeMode;
    private String safeText = "";
    private final Map<Long, MirrorResource> resources = new HashMap<>();
    private final Set<Long> missing = new HashSet<>();

    /** Timed frames (protocol v2) waiting for their time, per canvas or pixel surface ({@link SceneExporter#timedKey}). */
    private record TimedEntry(long dueUs, UiSceneCodec.Timed timed, UiSceneCodec.PixelBlock untimed) {}

    private final Map<Long, java.util.ArrayDeque<TimedEntry>> timedQ = new HashMap<>();
    private final Map<Long, Long> timedLast = new HashMap<>();
    private long timedApplied, timedDropped;

    /** What this client shows before the server answers (hover, scrolls, drags, typing; Part F). */
    private final ClientPrediction prediction = new ClientPrediction(this);

    private int[] frame;
    private byte[] rgba;
    private final Box.Region damage = new Box.Region();
    private boolean wasAnimating;
    private long renders;

    private static final class MirrorResource {
        final UiProto.ResourceKind kind;
        final int w, h;
        long key;
        int[] argb;

        MirrorResource(UiProto.ResourceKind kind, int w, int h) {
            this.kind = kind;
            this.w = w;
            this.h = h;
        }
    }

    /** {@code images}: the client's image cache (premultiplied pixels by key, or null if it lacks one). */
    public UiSceneMirror(LongFunction<int[]> images) {
        this.images = images;
    }

    public boolean ready() {
        return ready;
    }

    public UiDisplay display() {
        return display;
    }

    public long generation() {
        return gen;
    }

    public int epoch() {
        return epoch;
    }

    /** Image keys a header referenced that the cache didn't have (cleared by the caller). */
    public Set<Long> missing() {
        return missing;
    }

    public long renders() {
        return renders;
    }

    /** The server-clock time the mirror draws at for client time {@code clientNowMs}. */
    public long sceneTime(long clientNowMs) {
        return clientNowMs + clockOffset;
    }

    // =========================================================== messages

    /** Apply one STEP or KEYFRAME message at client time {@code clientNowMs}. */
    public Result apply(UiSceneCodec.Message m, long clientNowMs) {
        // The server's values first; predictions are made again on what it says.
        prediction.undo();
        try {
            return applyMessage(m, clientNowMs);
        } finally {
            if (ready) prediction.redo(clientNowMs);
        }
    }

    private Result applyMessage(UiSceneCodec.Message m, long clientNowMs) {
        UiSceneCodec.Patch p = m.patch();
        if (p == null) return Result.IGNORED;
        if (p.keyframe()) {
            reset();
            epoch = m.epoch();
        } else {
            if (!ready) return Result.NEED_KEYFRAME;
            if (p.from() != gen) return p.from() < gen ? Result.IGNORED : Result.NEED_KEYFRAME;
        }
        long offset = p.serverNowMs() - clientNowMs;
        if (!clockSet || p.keyframe() || Math.abs(offset - clockOffset) > 100) {
            clockOffset = offset;
            clockSet = true;
        }
        long now = sceneTime(clientNowMs);
        try {
            if (p.header() != null) applyHeader(p.header(), now);
            if (display == null) return Result.NEED_KEYFRAME;
            for (UiSceneCodec.TreeBlock t : p.trees()) {
                Surface s = surfaces.get(t.surface());
                if (s == null || s.tree == null) throw UiError.unknown("no tree surface " + t.surface());
                Box.Region d = new Box.Region();
                for (UiProto.TreeOp op : t.ops()) s.tree.apply(op, Integer.MAX_VALUE, Integer.MAX_VALUE, d);
                for (Box b : d.boxes()) damage.add(SceneRenderer.surfaceDamage(display, s, b, now));
            }
            for (UiSceneCodec.PixelBlock b : p.pixels()) {
                // Exact pixels never overtake timed frames still waiting for that buffer (a refine lands after them).
                java.util.ArrayDeque<TimedEntry> q = timedQ.get(SceneExporter.timedKey(
                        b.target() == UiSceneCodec.TARGET_CANVAS ? b.scope() : 0, b.id()));
                if (q != null && !q.isEmpty()) q.addLast(new TimedEntry(q.peekLast().dueUs, null, b));
                else applyPixels(b, now);
            }
        } catch (UiError | UiProtoException | RuntimeException e) {
            ready = false;
            return Result.NEED_KEYFRAME;
        }
        gen = p.to();
        ready = true;
        return Result.APPLIED;
    }

    private void reset() {
        timedQ.clear();
        timedLast.clear();
        surfaces.clear();
        scopes.clear();
        windows.clear();
        stack.clear();
        layers.clear();
        popups.clear();
        resources.clear();
        focused = null;
        safeMode = false;
        safeText = "";
        ready = false;
        gen = -1;
        if (display != null) damage.add(SceneRenderer.displayBox(display));
    }

    private void applyHeader(UiSceneCodec.Header h, long now) throws UiProtoException {
        UiDisplay d = h.display();
        if (!d.equals(display) || frame == null) {
            display = d;
            frame = new int[d.deviceW() * d.deviceH()];
            java.util.Arrays.fill(frame, SceneRenderer.BG);
            rgba = new byte[frame.length * 4];
            for (int i = 3; i < rgba.length; i += 4) rgba[i] = (byte) 255;
        }
        // Surfaces: keep the ones that are the same thing, make the rest.
        Map<Integer, Surface> next = new LinkedHashMap<>();
        Map<Integer, UiSceneCodec.SurfaceInfo> infos = new HashMap<>();
        for (UiSceneCodec.SurfaceInfo si : h.surfaces()) infos.put(si.id(), si);
        for (UiSceneCodec.SurfaceInfo si : h.surfaces()) surfaceFor(si, infos, next);
        surfaces.clear();
        surfaces.putAll(next);
        scopes.keySet().retainAll(next.values());
        // Windows, bottom to top.
        Map<Integer, UiWindow> nw = new HashMap<>();
        stack.clear();
        for (Surface s : surfaces.values()) s.window = null;
        for (UiSceneCodec.WindowInfo wi : h.windows()) {
            Surface s = surfaces.get(wi.surface());
            if (s == null) throw UiProtoException.malformed("window " + wi.id() + " without its surface");
            UiWindow w = windows.get(wi.id());
            if (w == null || w.surface != s) w = new UiWindow(wi.id(), s);
            w.placed = wi.placed();
            w.visible = wi.visible();
            w.states = wi.states();
            w.restoreAnimation(wi.anim(), wi.placeStart(), wi.alphaStart());
            s.window = w;
            nw.put(w.id, w);
            stack.add(w);
        }
        windows.clear();
        windows.putAll(nw);
        layers.clear();
        for (int id : h.layers()) if (surfaces.get(id) != null) layers.add(surfaces.get(id));
        popups.clear();
        for (int id : h.popups()) if (surfaces.get(id) != null) popups.add(surfaces.get(id));
        focused = windows.get(h.focusedWindow());
        safeMode = h.safeMode();
        safeText = h.safeText();
        // Resources: images by key from the cache; canvases keep their pixels while their size holds.
        Map<Long, MirrorResource> nr = new HashMap<>();
        for (UiSceneCodec.ResourceInfo ri : h.resources()) {
            long rk = resKey(ri.scope(), ri.id());
            MirrorResource r = resources.get(rk);
            if (r == null || r.kind != ri.kind() || r.w != ri.w() || r.h != ri.h()) {
                timedQ.remove(SceneExporter.timedKey(ri.scope(), ri.id())); // a resized canvas drops its timed frames
                r = new MirrorResource(ri.kind(), ri.w(), ri.h());
                if (ri.kind() == UiProto.ResourceKind.CANVAS) r.argb = new int[ri.w() * ri.h()];
            }
            if (ri.kind() == UiProto.ResourceKind.IMAGE) {
                r.key = ri.key();
                r.argb = ri.key() == 0 ? null : images.apply(ri.key());
                if (ri.key() != 0 && r.argb == null) missing.add(ri.key());
            }
            nr.put(rk, r);
        }
        resources.clear();
        resources.putAll(nr);
        // The header moved things around: redraw everything (rare next to tree and pixel changes).
        damage.add(SceneRenderer.displayBox(display));
    }

    private Surface surfaceFor(UiSceneCodec.SurfaceInfo si, Map<Integer, UiSceneCodec.SurfaceInfo> infos, Map<Integer, Surface> next)
            throws UiProtoException {
        Surface done = next.get(si.id());
        if (done != null) return done;
        Surface parent = null;
        if (si.parent() != 0) {
            UiSceneCodec.SurfaceInfo pi = infos.get(si.parent());
            if (pi == null || pi.id() == si.id()) throw UiProtoException.malformed("surface " + si.id() + " has no parent " + si.parent());
            if (next.size() > infos.size()) throw UiProtoException.malformed("surface parents loop");
            parent = surfaceFor(pi, infos, next);
        }
        Surface s = surfaces.get(si.id());
        if (s == null || s.kind != si.kind() || s.role != si.role() || s.layer != si.layer() || s.parent != parent
                || s.px != si.x() || s.py != si.y()) {
            s = new Surface(si.id(), si.kind(), si.role(), si.layer(), parent, si.x(), si.y());
        }
        s.contentW = si.contentW();
        s.contentH = si.contentH();
        s.mapped = si.mapped();
        s.popupDone = si.popupDone();
        if (s.isPixels() && (s.argb == null || s.bw != si.bw() || s.bh != si.bh() || s.pscale != si.pscale())) {
            if (s.argb != null && s.bw == si.bw() && s.bh == si.bh()) {
                s.pscale = si.pscale();
            } else {
                s.setMirrorBuffer(si.bw(), si.bh(), si.pscale());
            }
        }
        scopes.put(s, si.scope());
        next.put(si.id(), s);
        return s;
    }

    private void applyPixels(UiSceneCodec.PixelBlock b, long now) throws UiError {
        int[] dst;
        int w = b.w(), h = b.h();
        Surface s = null;
        MirrorResource canvas = null;
        if (b.target() == UiSceneCodec.TARGET_SURFACE) {
            s = surfaces.get(b.id());
            if (s == null || !s.isPixels() || s.bw != w || s.bh != h) throw UiError.unknown("no pixel surface " + b.id());
            dst = s.argb;
        } else {
            canvas = resources.get(resKey(b.scope(), b.id()));
            if (canvas == null || canvas.kind != UiProto.ResourceKind.CANVAS || canvas.w != w || canvas.h != h) {
                throw UiError.unknown("no canvas " + b.id());
            }
            dst = canvas.argb;
        }
        int T = UiSceneCodec.TILE;
        int tx = (w + T - 1) / T;
        Box.Region changed = new Box.Region();
        if (b.tiles() == null) {
            System.arraycopy(b.pixels(), 0, dst, 0, w * h);
            changed.add(new Box(0, 0, w, h));
        } else {
            int k = 0;
            for (int t : b.tiles()) {
                int x0 = (t % tx) * T, y0 = (t / tx) * T;
                int x1 = Math.min(w, x0 + T), y1 = Math.min(h, y0 + T);
                for (int y = y0; y < y1; y++) {
                    System.arraycopy(b.pixels(), k, dst, y * w + x0, x1 - x0);
                    k += x1 - x0;
                }
                changed.add(new Box(x0, y0, x1, y1));
            }
        }
        if (s != null) {
            s.pixelVersion++;
            for (Box c : changed.boxes()) damage.add(SceneRenderer.surfaceDamage(display, s, c.scale(s.pscale), now));
        } else {
            damageUsers(b.scope(), b.id(), now);
        }
    }

    private void damageUsers(int scope, int id, long now) {
        for (Surface s : surfaces.values()) {
            if (s.tree == null || !s.mapped || scopes.getOrDefault(s, -1) != scope) continue;
            for (NodeTree.Node n : s.tree.usersOf(id)) damage.add(SceneRenderer.surfaceDamage(display, s, NodeTree.absBox(n), now));
        }
    }

    // =========================================================== client prediction (Part F)

    /**
     * A pointer event the player gave this display (device pixels; kinds as
     * MouseInputPacket: 1 move, 2 down, 3 up, 4 scroll). Returns the scroll
     * to report to the app when the wheel scrolled a scroll container here
     * (the wheel then doesn't go to the server), else null.
     */
    public ClientPrediction.ScrollReport pointer(int kind, int x, int y, int button, int scrollDir, long clientNowMs) {
        if (!ready || display == null) return null;
        return prediction.pointer(kind, x, y, button, scrollDir, clientNowMs);
    }

    /** Typed text: shown in the focused text field at once. */
    public boolean typed(String text, long clientNowMs) {
        return ready && prediction.text(text, clientNowMs);
    }

    /** A key (HID usage) going down or up: Backspace edits the predicted text. */
    public boolean key(int hid, boolean down, long clientNowMs) {
        return ready && prediction.key(hid, down, clientNowMs);
    }

    /** Predictions made so far (tests, stats). */
    public long predictions() {
        return prediction.predictions();
    }

    /** Let predictions that waited for the server expire (once per rendered frame). */
    void refreshPrediction(long clientNowMs) {
        if (!ready || !prediction.active()) return;
        prediction.undo();
        prediction.redo(clientNowMs);
    }

    List<SceneRenderer.Item> drawItems(long now) {
        return SceneRenderer.drawOrder(scene(), now);
    }

    int sceneIdOf(Surface s) {
        for (Map.Entry<Integer, Surface> e : surfaces.entrySet()) if (e.getValue() == s) return e.getKey();
        return 0;
    }

    UiWindow focusedWindow() {
        return focused;
    }

    UiWindow window(int id) {
        return windows.get(id);
    }

    int displayScale() {
        return display == null ? 1 : display.scale();
    }

    void damageNode(Surface s, NodeTree.Node n) {
        if (display == null || s == null) return;
        long now = System.nanoTime() / 1_000_000L + clockOffset;
        damage.add(SceneRenderer.surfaceDamage(display, s, NodeTree.absBox(n), now));
    }

    void damageAll() {
        if (display != null) damage.add(SceneRenderer.displayBox(display));
    }

    // =========================================================== timed frames (protocol v2)

    /**
     * A timed frame (or a flush) of a canvas or pixel surface, to be shown at
     * client time {@code dueUs} ({@code 0}: after the frames already waiting,
     * a refine). Returns false if the frame builds on one this client doesn't
     * have (it asks for a keyframe then).
     */
    public boolean timed(UiSceneCodec.Message m, long dueUs) {
        UiSceneCodec.Timed t = m.timed();
        if (t == null || !ready) return true;
        long key = SceneExporter.timedKey(t.target() == UiSceneCodec.TARGET_CANVAS ? t.scope() : 0, t.id());
        java.util.ArrayDeque<TimedEntry> q = timedQ.computeIfAbsent(key, k -> new java.util.ArrayDeque<>());
        if (m.kind() == UiSceneCodec.TFLUSH) {
            q.removeIf(e -> e.timed != null && e.timed.seq() > t.seq());
            timedLast.put(key, t.seq());
            return true;
        }
        Long last = timedLast.get(key);
        if (!t.whole() && last != null && last != t.base()) return false;
        timedLast.put(key, t.seq());
        long due = dueUs != 0 ? dueUs : (q.isEmpty() ? 0 : q.peekLast().dueUs);
        q.addLast(new TimedEntry(due, t, null));
        return true;
    }

    /**
     * Show the timed frames that are due at client time {@code nowUs}: for
     * each buffer the frames before the newest due one are applied too (they
     * build on each other) but never shown on their own. Returns how many
     * buffers changed.
     */
    public int tickTimed(long nowUs, long clientNowMs) {
        if (!ready || display == null || timedQ.isEmpty()) return 0;
        long now = sceneTime(clientNowMs);
        int changed = 0;
        for (java.util.ArrayDeque<TimedEntry> q : timedQ.values()) {
            int n = 0;
            while (!q.isEmpty() && q.peekFirst().dueUs <= nowUs) {
                TimedEntry e = q.removeFirst();
                try {
                    if (e.untimed != null) applyPixels(e.untimed, now);
                    else applyTimed(e.timed, now);
                } catch (UiError err) {
                    // its buffer is gone (resized, destroyed): drop it
                }
                n++;
            }
            if (n > 0) {
                changed++;
                timedApplied++;
                timedDropped += n - 1;
            }
        }
        return changed;
    }

    private void applyTimed(UiSceneCodec.Timed t, long now) throws UiError {
        int[] src;
        int[] tiles = null;
        if (t.encoding() == UiSceneCodec.ENC_JPEG) {
            if (!t.decode()) return;
            src = t.decoded()[0];
        } else {
            src = t.pixels();
            tiles = t.tiles();
        }
        applyPixels(new UiSceneCodec.PixelBlock(t.target(), t.scope(), t.id(), t.w(), t.h(), tiles, src), now);
    }

    /** Timed frames waiting (diagnostics, tests). */
    public int timedQueued() {
        int n = 0;
        for (java.util.ArrayDeque<TimedEntry> q : timedQ.values()) n += q.size();
        return n;
    }

    /** {applied (shown), dropped (applied but replaced in the same frame)}. */
    public long[] timedCounters() {
        return new long[]{timedApplied, timedDropped};
    }

    /** An image arrived in the cache: resources waiting for it get it. */
    public void imageArrived(long key, int[] argb, long clientNowMs) {
        missing.remove(key);
        if (display == null) return;
        long now = sceneTime(clientNowMs);
        for (Map.Entry<Long, MirrorResource> e : resources.entrySet()) {
            MirrorResource r = e.getValue();
            if (r.kind == UiProto.ResourceKind.IMAGE && r.key == key && r.argb == null) {
                r.argb = argb;
                damageUsers((int) (e.getKey() >>> 32), (int) (long) e.getKey(), now);
            }
        }
    }

    private static long resKey(int scope, int id) {
        return ((long) scope << 32) | (id & 0xFFFFFFFFL);
    }

    private Rasterizer.Resources resourcesOf(Surface s) {
        int scope = scopes.getOrDefault(s, -1);
        return new Rasterizer.Resources() {
            @Override public int[] pixels(int id) {
                MirrorResource r = resources.get(resKey(scope, id));
                return r == null ? null : r.argb;
            }

            @Override public int width(int id) {
                MirrorResource r = resources.get(resKey(scope, id));
                return r == null ? 0 : r.w;
            }

            @Override public int height(int id) {
                MirrorResource r = resources.get(resKey(scope, id));
                return r == null ? 0 : r.h;
            }
        };
    }

    // =========================================================== drawing

    private SceneRenderer.Scene scene() {
        return new SceneRenderer.Scene(display, layers, stack, popups, safeMode, focused);
    }

    /** Is a window transition running at client time {@code clientNowMs}? */
    public boolean animating(long clientNowMs) {
        long now = sceneTime(clientNowMs);
        for (UiWindow w : stack) if (w.animating(now)) return true;
        return false;
    }

    /**
     * Redraw what changed (and everything while a transition runs, as the
     * server does) at client time {@code clientNowMs}. Returns the redrawn
     * rectangles in device pixels (empty when nothing changed).
     */
    public List<Box> render(long clientNowMs) {
        if (!ready || display == null) return List.of();
        long now = sceneTime(clientNowMs);
        boolean anim = false;
        for (UiWindow w : stack) anim |= w.animating(now);
        if (anim || wasAnimating) damage.add(SceneRenderer.displayBox(display));
        wasAnimating = anim;
        List<Box> boxes = damage.take(SceneRenderer.displayBox(display));
        if (boxes.isEmpty()) return boxes;
        SceneRenderer.composite(display, frame, rgba, boxes, SceneRenderer.drawOrder(scene(), now), safeText, this::resourcesOf);
        renders++;
        return boxes;
    }

    /** Redraw everything at client time {@code clientNowMs} (tests, a texture that was lost). */
    public void renderAll(long clientNowMs) {
        if (display == null) return;
        damage.add(SceneRenderer.displayBox(display));
        render(clientNowMs);
    }

    /** The frame as straight RGBA bytes (the live array; copy it to keep it). */
    public byte[] rgba() {
        return rgba;
    }

    /**
     * Copy the redrawn rectangles into a client display (RGBA8888, graphics
     * mode), marking only the 16x16 tiles whose pixels really changed, so
     * the texture uploads just those. A display of another shape gets the
     * whole frame.
     */
    public void scanOut(TerminalDisplay td, List<Box> boxes) {
        if (display == null || rgba == null) return;
        int W = display.deviceW(), H = display.deviceH();
        synchronized (td) {
            byte[] px = td.getPixelData();
            if (td.getDisplayMode() != 1 || td.getGfxWidth() != W || td.getGfxHeight() != H
                    || td.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGBA8888 || px == null || px.length != rgba.length) {
                td.setGfxFrame(1, W, H, TerminalDisplay.PIXEL_FORMAT_RGBA8888, null, rgba);
                return;
            }
            int T = TerminalDisplay.GFX_TILE;
            int cols = (W + T - 1) / T;
            java.util.BitSet seen = new java.util.BitSet();
            boolean changed = false;
            for (Box b : boxes) {
                for (int ty = b.y0() / T; ty * T < b.y1(); ty++) {
                    for (int tx = b.x0() / T; tx * T < b.x1(); tx++) {
                        int idx = ty * cols + tx;
                        if (seen.get(idx)) continue;
                        seen.set(idx);
                        int x0 = tx * T, y0 = ty * T, x1 = Math.min(W, x0 + T), y1 = Math.min(H, y0 + T);
                        boolean diff = false;
                        for (int y = y0; y < y1; y++) {
                            int off = (y * W + x0) * 4, end = (y * W + x1) * 4;
                            if (!java.util.Arrays.equals(rgba, off, end, px, off, end)) {
                                System.arraycopy(rgba, off, px, off, end - off);
                                diff = true;
                            }
                        }
                        if (diff) {
                            td.addGfxDamageTile(idx);
                            changed = true;
                        }
                    }
                }
            }
            // Renderers re-upload only when the pixel counter moves (TerminalScreen, the Screen wall BER).
            if (changed) td.setGfxDirtyCounters(td.getPixelDirtyCounter() + 1, td.getPaletteDirtyCounter());
        }
    }

    // =========================================================== inspection

    /** Windows in the mirror, bottom to top: {id, x, y, w, h} at client time {@code clientNowMs}. */
    public List<int[]> windows(long clientNowMs) {
        long now = sceneTime(clientNowMs);
        List<int[]> out = new ArrayList<>();
        for (UiWindow w : stack) {
            int[] r = w.rect(now);
            out.add(new int[]{w.id, r[0], r[1], r[2], r[3]});
        }
        return out;
    }

    /** Image keys this scene uses. */
    Set<Long> imageKeys() {
        Set<Long> out = new HashSet<>();
        for (MirrorResource r : resources.values()) if (r.kind == UiProto.ResourceKind.IMAGE && r.key != 0) out.add(r.key);
        return out;
    }

    public int surfaceCount() {
        return surfaces.size();
    }
}
