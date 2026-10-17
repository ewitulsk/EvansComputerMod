package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.ui.proto.UiWriter;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;
import java.util.zip.Deflater;

/**
 * Turns a {@link Compositor}'s scene into the scene stream (§14 phase 2a,
 * {@link UiSceneCodec}): after every compositor tick it builds at most one
 * <i>step</i>, the patch from the previous generation to the next, and
 * hands it to its sink (a {@link UiSceneHub}) to share with every viewer.
 * Keyframes (the whole scene) are built on request for late joiners.
 *
 * <p>A step holds the scene header when anything in it changed (display,
 * surfaces, window placement and animations, stacking, safe mode,
 * resources), the tree operations each surface applied since the last step
 * (recorded as the compositor applies them, so they mean exactly what the
 * app sent), a new surface's whole tree as {@code Create} operations, and
 * the 16x16 tiles of pixel surfaces and canvases that changed (diffed
 * against a shadow of what was sent). Window transitions travel as their
 * start, length and easing, so the client animates them with no traffic.
 *
 * <p>Every method runs under the compositor's monitor (the compositor calls
 * {@link #applied} and {@link #afterTick}; {@link #keyframe} takes the
 * monitor itself). Image data for the hub is kept in a concurrent map.
 */
public final class SceneExporter {
    /**
     * A built message and what it needs: the image keys its header references
     * (null without a header). A keyframe also says which timed frame each
     * canvas and pixel surface shows in it ({@code timedShown}, by
     * {@link #timedKey}), so the hub sends a late joiner only the frames after.
     */
    public record Step(int epoch, long from, long to, boolean keyframe, byte[] data, long[] images,
                       Map<Long, Long> timedShown) {
        public Step(int epoch, long from, long to, boolean keyframe, byte[] data, long[] images) {
            this(epoch, from, to, keyframe, data, images, Map.of());
        }
    }

    /**
     * Work for the hub's timed stream (protocol v2 CommitAt; built on the
     * pump, not under the compositor's lock): a frame queued ({@link #FRAME},
     * {@code argb} is the queue's own copy, never changed), a frame shown
     * ({@link #SHOWN}), or a flush ({@link #FLUSH}, {@code argb} = what is
     * shown now). {@code tier} is the display panel's (Tier 1 stays exact).
     */
    public record TimedJob(int kind, int target, int scope, int id, int w, int h, long seq, long ptsUs, int[] argb,
                           int hint, int tier) {
        public static final int FRAME = 0, SHOWN = 1, FLUSH = 2;

        public long key() {
            return timedKey(scope, id);
        }
    }

    /** A timed target's key: a canvas by connection and id, a pixel surface by scene id (scope 0). */
    public static long timedKey(int scope, int id) {
        return ((long) scope << 32) | (id & 0xFFFFFFFFL);
    }

    /** Where timed jobs go (the hub), or null. */
    private volatile Consumer<TimedJob> timedSink;

    public void setTimedSink(Consumer<TimedJob> sink) {
        this.timedSink = sink;
    }

    private final Compositor comp;
    private final int epoch;
    private final Consumer<Step> sink;
    private final Deflater deflater = new Deflater(Deflater.BEST_SPEED);

    private final IdentityHashMap<Surface, Integer> ids = new IdentityHashMap<>();
    private int nextId = 1;
    private final IdentityHashMap<Surface, List<UiProto.TreeOp>> logs = new IdentityHashMap<>();
    private final IdentityHashMap<Surface, int[]> shadows = new IdentityHashMap<>();
    private final IdentityHashMap<Surface, Integer> shadowVersions = new IdentityHashMap<>();
    private final Map<Long, int[]> canvasShadows = new HashMap<>();
    private final Map<Long, Integer> canvasVersions = new HashMap<>();
    private byte[] lastHeader;
    private long[] lastImages = new long[0];
    private long gen;
    private long steps, keyframes, stepBytes, keyframeBytes;

    /** Image pixels (premultiplied) by key: those the current header uses, plus recently dropped ones. */
    private final Map<Long, Image> images = new ConcurrentHashMap<>();
    private final Map<Long, Image> recent = java.util.Collections.synchronizedMap(new LinkedHashMap<>(16, 0.75f, false) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<Long, Image> e) {
            return size() > 64;
        }
    });

    public record Image(int w, int h, int[] argb) {}

    public SceneExporter(Compositor comp, int epoch, Consumer<Step> sink) {
        this.comp = comp;
        this.epoch = epoch;
        this.sink = sink;
    }

    public int epoch() {
        return epoch;
    }

    // =========================================================== compositor hooks

    /** The compositor applied {@code op} to {@code s} at a commit. */
    void applied(Surface s, UiProto.TreeOp op) {
        if (ids.containsKey(s)) logs.computeIfAbsent(s, k -> new ArrayList<>()).add(op);
    }

    /** End of a compositor tick: build and hand on the step, if anything changed. */
    void afterTick(long now) {
        Step st = buildStep(now);
        if (st != null) sink.accept(st);
    }

    /**
     * The whole scene at the current generation (after flushing a step for
     * pending changes, which goes to the sink first). Any thread.
     */
    public Step keyframe() {
        synchronized (comp) {
            long now = comp.host.nowMs();
            afterTick(now);
            List<Surface> all = surfaces();
            List<UiSceneCodec.TreeBlock> trees = new ArrayList<>();
            List<UiSceneCodec.PixelBlock> pixels = new ArrayList<>();
            for (Surface s : all) {
                int id = ids.get(s);
                if (s.tree != null) {
                    List<UiProto.TreeOp> ops = createOps(s.tree);
                    if (!ops.isEmpty()) trees.add(new UiSceneCodec.TreeBlock(id, ops));
                } else {
                    pixels.add(new UiSceneCodec.PixelBlock(UiSceneCodec.TARGET_SURFACE, 0, id, s.bw, s.bh, null, s.argb.clone()));
                }
            }
            for (UiConnection c : comp.conns.values()) {
                for (UiResource r : c.resources.values()) {
                    if (r.kind != UiProto.ResourceKind.CANVAS) continue;
                    pixels.add(new UiSceneCodec.PixelBlock(UiSceneCodec.TARGET_CANVAS, c.id, r.id, r.w, r.h, null, r.argb.clone()));
                }
            }
            byte[] header = lastHeader != null ? lastHeader : encodeHeader(header(now));
            byte[] body = UiSceneCodec.encodePatch(true, 0, gen, now, header, trees, pixels);
            byte[] data = UiSceneCodec.pack(UiSceneCodec.KEYFRAME, epoch, body, deflater);
            keyframes++;
            keyframeBytes += data.length;
            Map<Long, Long> shown = new HashMap<>();
            for (Surface s : all) if (s.timed != null) shown.put(timedKey(0, ids.get(s)), s.timed.appliedSeq);
            for (UiConnection c : comp.conns.values()) {
                for (UiResource r : c.resources.values()) if (r.timed != null) shown.put(timedKey(c.id, r.id), r.timed.appliedSeq);
            }
            return new Step(epoch, 0, gen, true, data, lastImages.clone(), shown);
        }
    }

    /** The surface a scene id names, or null (a client's report about a surface that is gone). */
    Surface surfaceOf(int sceneId) {
        for (Map.Entry<Surface, Integer> e : ids.entrySet()) if (e.getValue() == sceneId) return e.getKey();
        return null;
    }

    // =========================================================== timed frames (protocol v2)

    private int sceneId(Surface s) {
        Integer id = ids.get(s);
        if (id == null) {
            id = nextId++;
            ids.put(s, id);
        }
        return id;
    }

    /**
     * Send a canvas ({@code target} canvas) or pixel surface whole and exact
     * in the next step, from what it shows now (a lossy picture that stood
     * still). Any thread.
     */
    public void refine(int target, int scope, int id) {
        synchronized (comp) {
            if (target == UiSceneCodec.TARGET_CANVAS) {
                long key = ((long) scope << 32) | (id & 0xFFFFFFFFL);
                canvasShadows.remove(key);
                canvasVersions.remove(key);
            } else {
                Surface s = surfaceOf(id);
                if (s != null) {
                    shadows.remove(s);
                    shadowVersions.remove(s);
                }
            }
        }
        comp.host.requestTick(0);
    }

    /** A frame was queued for a canvas ({@code c, r}) or a pixel surface ({@code s}). Compositor lock held. */
    void timedFrame(Surface s, UiConnection c, UiResource r, TimedQueue.Frame f, int hint) {
        Consumer<TimedJob> sink = timedSink;
        if (sink == null) return;
        int tier = comp.display.tier();
        if (s != null) {
            sink.accept(new TimedJob(TimedJob.FRAME, UiSceneCodec.TARGET_SURFACE, 0, sceneId(s), s.bw, s.bh, f.seq(), f.ptsUs(),
                    f.argb(), hint, tier));
        } else {
            sink.accept(new TimedJob(TimedJob.FRAME, UiSceneCodec.TARGET_CANVAS, c.id, r.id, r.w, r.h, f.seq(), f.ptsUs(),
                    f.argb(), hint, tier));
        }
    }

    /**
     * The compositor shows timed frame {@code seq} now. Its pixels went to
     * clients ahead, so the step diff takes them as sent (no untimed resend).
     * Compositor lock held.
     */
    void timedShown(Surface s, UiConnection c, UiResource r, long seq) {
        if (s != null) {
            shadows.put(s, copyInto(shadows.get(s), s.argb));
            shadowVersions.put(s, s.pixelVersion);
        } else {
            long key = ((long) c.id << 32) | (r.id & 0xFFFFFFFFL);
            canvasShadows.put(key, copyInto(canvasShadows.get(key), r.argb));
            canvasVersions.put(key, r.version);
        }
        Consumer<TimedJob> sink = timedSink;
        if (sink == null) return;
        if (s != null) sink.accept(new TimedJob(TimedJob.SHOWN, UiSceneCodec.TARGET_SURFACE, 0, sceneId(s), s.bw, s.bh, seq, 0, null, 0, 0));
        else sink.accept(new TimedJob(TimedJob.SHOWN, UiSceneCodec.TARGET_CANVAS, c.id, r.id, r.w, r.h, seq, 0, null, 0, 0));
    }

    /** The queued frames after {@code after} were dropped (QueueFlush). Compositor lock held. */
    void timedFlushed(Surface s, UiConnection c, UiResource r, long after) {
        Consumer<TimedJob> sink = timedSink;
        if (sink == null) return;
        if (s != null) {
            sink.accept(new TimedJob(TimedJob.FLUSH, UiSceneCodec.TARGET_SURFACE, 0, sceneId(s), s.bw, s.bh, after, 0,
                    s.argb.clone(), 0, 0));
        } else {
            sink.accept(new TimedJob(TimedJob.FLUSH, UiSceneCodec.TARGET_CANVAS, c.id, r.id, r.w, r.h, after, 0,
                    r.argb.clone(), 0, 0));
        }
    }

    /** Pixels of an image the stream references, or null if it is long gone. */
    public Image image(long key) {
        Image i = images.get(key);
        return i != null ? i : recent.get(key);
    }

    // =========================================================== steps

    private List<Surface> surfaces() {
        List<Surface> out = new ArrayList<>();
        for (UiConnection c : comp.conns.values()) out.addAll(c.surfaces.values());
        return out;
    }

    private Step buildStep(long now) {
        List<Surface> all = surfaces();
        // Forget surfaces that are gone; new ones get ids (and are sent whole).
        java.util.Set<Surface> live = java.util.Collections.newSetFromMap(new IdentityHashMap<>());
        live.addAll(all);
        ids.keySet().retainAll(live);
        logs.keySet().retainAll(live);
        shadows.keySet().retainAll(live);
        shadowVersions.keySet().retainAll(live);
        java.util.Set<Surface> fresh = java.util.Collections.newSetFromMap(new IdentityHashMap<>());
        for (Surface s : all) {
            if (!ids.containsKey(s)) {
                ids.put(s, nextId++);
                fresh.add(s);
            }
        }
        UiSceneCodec.Header h = header(now);
        byte[] hb = encodeHeader(h);
        boolean headerChanged = !Arrays.equals(hb, lastHeader);

        List<UiSceneCodec.TreeBlock> trees = new ArrayList<>();
        List<UiSceneCodec.PixelBlock> pixels = new ArrayList<>();
        for (Surface s : all) {
            int id = ids.get(s);
            if (s.tree != null) {
                List<UiProto.TreeOp> ops = fresh.contains(s) ? createOps(s.tree) : logs.remove(s);
                logs.remove(s);
                if (ops != null && !ops.isEmpty()) trees.add(new UiSceneCodec.TreeBlock(id, ops));
            } else {
                Integer v = shadowVersions.get(s);
                if (v != null && v == s.pixelVersion) continue;
                int[] shadow = shadows.get(s);
                UiSceneCodec.PixelBlock b = diff(UiSceneCodec.TARGET_SURFACE, 0, id, s.argb, s.bw, s.bh, shadow);
                if (b != null) pixels.add(b);
                shadows.put(s, copyInto(shadow, s.argb));
                shadowVersions.put(s, s.pixelVersion);
            }
        }
        java.util.Set<Long> liveCanvases = new java.util.HashSet<>();
        for (UiConnection c : comp.conns.values()) {
            for (UiResource r : c.resources.values()) {
                if (r.kind != UiProto.ResourceKind.CANVAS) continue;
                long key = ((long) c.id << 32) | (r.id & 0xFFFFFFFFL);
                liveCanvases.add(key);
                Integer v = canvasVersions.get(key);
                int[] shadow = canvasShadows.get(key);
                if (v != null && v == r.version && shadow != null && shadow.length == r.argb.length) continue;
                UiSceneCodec.PixelBlock b = diff(UiSceneCodec.TARGET_CANVAS, c.id, r.id, r.argb, r.w, r.h,
                        shadow != null && shadow.length == r.argb.length ? shadow : null);
                if (b != null) pixels.add(b);
                canvasShadows.put(key, copyInto(shadow, r.argb));
                canvasVersions.put(key, r.version);
            }
        }
        canvasShadows.keySet().retainAll(liveCanvases);
        canvasVersions.keySet().retainAll(liveCanvases);

        if (!headerChanged && trees.isEmpty() && pixels.isEmpty()) return null;
        long from = gen++;
        byte[] body = UiSceneCodec.encodePatch(false, from, gen, now, headerChanged ? hb : null, trees, pixels);
        byte[] data = UiSceneCodec.pack(UiSceneCodec.STEP, epoch, body, deflater);
        long[] imgs = null;
        if (headerChanged) {
            lastHeader = hb;
            imgs = imageKeys(h);
            lastImages = imgs;
        }
        steps++;
        stepBytes += data.length;
        return new Step(epoch, from, gen, false, data, imgs);
    }

    /**
     * {@code src} copied into {@code shadow} when it has the same size, else
     * into a new array (DISPLAY_PIPELINE_SPEC.md Part A7: no clone per frame).
     */
    private static int[] copyInto(int[] shadow, int[] src) {
        if (shadow == null || shadow.length != src.length) return src.clone();
        System.arraycopy(src, 0, shadow, 0, src.length);
        return shadow;
    }

    /** Changed tiles of {@code px} against {@code shadow} (all of them when there is no shadow), or null. */
    private static UiSceneCodec.PixelBlock diff(int target, int scope, int id, int[] px, int w, int h, int[] shadow) {
        if (shadow == null || shadow.length != px.length) {
            return new UiSceneCodec.PixelBlock(target, scope, id, w, h, null, px.clone());
        }
        int T = UiSceneCodec.TILE;
        int tx = (w + T - 1) / T, ty = (h + T - 1) / T;
        List<Integer> tiles = new ArrayList<>();
        int count = 0;
        for (int t = 0; t < tx * ty; t++) {
            int x0 = (t % tx) * T, y0 = (t / tx) * T;
            int x1 = Math.min(w, x0 + T), y1 = Math.min(h, y0 + T);
            boolean changed = false;
            for (int y = y0; y < y1 && !changed; y++) {
                changed = !Arrays.equals(px, y * w + x0, y * w + x1, shadow, y * w + x0, y * w + x1);
            }
            if (changed) {
                tiles.add(t);
                count += (x1 - x0) * (y1 - y0);
            }
        }
        if (tiles.isEmpty()) return null;
        int[] out = new int[count];
        int k = 0;
        int[] ta = new int[tiles.size()];
        for (int i = 0; i < ta.length; i++) {
            int t = tiles.get(i);
            ta[i] = t;
            int x0 = (t % tx) * T, y0 = (t / tx) * T;
            int x1 = Math.min(w, x0 + T), y1 = Math.min(h, y0 + T);
            for (int y = y0; y < y1; y++) {
                System.arraycopy(px, y * w + x0, out, k, x1 - x0);
                k += x1 - x0;
            }
        }
        return new UiSceneCodec.PixelBlock(target, scope, id, w, h, ta, out);
    }

    // =========================================================== header

    private UiSceneCodec.Header header(long now) {
        UiDisplay d = comp.display;
        List<UiSceneCodec.SurfaceInfo> surfaces = new ArrayList<>();
        List<UiSceneCodec.ResourceInfo> resources = new ArrayList<>();
        Map<Long, Image> imgs = new HashMap<>();
        for (UiConnection c : comp.conns.values()) {
            for (Surface s : c.surfaces.values()) {
                Integer id = ids.get(s);
                if (id == null) {
                    id = nextId++;
                    ids.put(s, id);
                }
                Integer parent = s.parent == null ? Integer.valueOf(0) : ids.get(s.parent);
                surfaces.add(new UiSceneCodec.SurfaceInfo(id, c.id, s.kind, s.role, s.layer, parent == null ? 0 : parent,
                        s.px, s.py, s.contentW, s.contentH, s.mapped, s.popupDone, s.bw, s.bh, s.pscale));
            }
            for (UiResource r : c.resources.values()) {
                long key = 0;
                if (r.kind == UiProto.ResourceKind.IMAGE && r.argb != null) {
                    key = UiSceneCodec.imageKey(r.hash, r.w, r.h);
                    imgs.put(key, new Image(r.w, r.h, r.argb));
                }
                resources.add(new UiSceneCodec.ResourceInfo(c.id, r.id, r.kind, r.w, r.h, key));
            }
        }
        // Keep image data for the hub: current ones, and dropped ones for a while (steps in flight).
        for (Map.Entry<Long, Image> e : images.entrySet()) {
            if (!imgs.containsKey(e.getKey())) recent.put(e.getKey(), e.getValue());
        }
        images.keySet().retainAll(imgs.keySet());
        images.putAll(imgs);

        List<Integer> layers = new ArrayList<>();
        for (Surface s : comp.layers) layers.add(ids.getOrDefault(s, 0));
        List<Integer> popups = new ArrayList<>();
        for (Surface s : comp.popups) popups.add(ids.getOrDefault(s, 0));
        List<UiSceneCodec.WindowInfo> windows = new ArrayList<>();
        for (UiWindow w : comp.stack) {
            windows.add(new UiSceneCodec.WindowInfo(w.id, ids.getOrDefault(w.surface, 0), w.placed, w.visible, w.states,
                    normalized(w, now), w.placeStartMs(), w.alphaStartMs()));
        }
        UiWindow f = comp.focused;
        return new UiSceneCodec.Header(d.deviceW(), d.deviceH(), d.scale(), d.tier(), d.device(), comp.safeMode,
                comp.safeMode ? comp.safeBarText() : "", f == null ? 0 : f.id, surfaces, layers, popups, windows, resources);
    }

    /**
     * A window's animation state with finished transitions zeroed, so a
     * header doesn't change (and isn't resent) just because time passed, and
     * an old start time never replays an animation on a client.
     */
    private static int[] normalized(UiWindow w, long now) {
        int[] a = w.animation();
        if (a[8] > 0 && now >= w.placeStartMs() + a[8]) {
            a[8] = 0;
            a[4] = a[5] = a[6] = a[7] = 0;
            a[9] = 0;
        }
        if (a[12] > 0 && now >= w.alphaStartMs() + a[12]) {
            a[12] = 0;
            a[11] = 255;
            a[13] = 0;
        }
        return a;
    }

    private static long[] imageKeys(UiSceneCodec.Header h) {
        return h.resources().stream().mapToLong(UiSceneCodec.ResourceInfo::key).filter(k -> k != 0).distinct().toArray();
    }

    private static byte[] encodeHeader(UiSceneCodec.Header h) {
        UiWriter w = new UiWriter(512);
        UiSceneCodec.writeHeader(w, h);
        return w.toByteArray();
    }

    // =========================================================== trees as Create ops

    /** A tree's nodes as {@code Create} operations, parents first, in child order. */
    static List<UiProto.TreeOp> createOps(NodeTree tree) {
        List<UiProto.TreeOp> out = new ArrayList<>();
        addCreates(tree.root(), out);
        return out;
    }

    private static void addCreates(NodeTree.Node parent, List<UiProto.TreeOp> out) {
        for (NodeTree.Node n : parent.children()) {
            out.add(new UiProto.TreeOp.Create(n.id, parent.id, UiCodec.APPEND, n.kind, props(n)));
            if (n.kind == UiProto.NodeKind.GROUP) addCreates(n, out);
        }
    }

    /** The props a node differs from the defaults in, sorted by tag. */
    static List<UiProto.Prop> props(NodeTree.Node n) {
        List<UiProto.Prop> p = new ArrayList<>();
        if (n.x != 0) p.add(new UiProto.Prop.X(n.x));
        if (n.y != 0) p.add(new UiProto.Prop.Y(n.y));
        if (n.w != 0) p.add(new UiProto.Prop.W(n.w));
        if (n.h != 0) p.add(new UiProto.Prop.H(n.h));
        if (n.clip) p.add(new UiProto.Prop.Clip(true));
        if (n.opacity != 255) p.add(new UiProto.Prop.Opacity(n.opacity));
        if (n.scale != 1) p.add(new UiProto.Prop.Scale(n.scale));
        if (n.hitId != 0) p.add(new UiProto.Prop.HitId(n.hitId));
        if (n.fill != 0) p.add(new UiProto.Prop.Fill(n.fill));
        if (n.radius != 0) p.add(new UiProto.Prop.Radius(n.radius));
        if (n.borderW != 0) p.add(new UiProto.Prop.BorderW(n.borderW));
        if (n.borderColor != 0) p.add(new UiProto.Prop.BorderColor(n.borderColor));
        if (!n.text.isEmpty()) p.add(new UiProto.Prop.Text(n.text));
        if (n.font != UiProto.Font.UI) p.add(new UiProto.Prop.Font(n.font));
        if (n.size != 12) p.add(new UiProto.Prop.Size(n.size));
        if (n.color != 0x000000FF) p.add(new UiProto.Prop.Color(n.color));
        if (n.resource != 0) p.add(new UiProto.Prop.Resource(n.resource));
        if (n.src != null) p.add(new UiProto.Prop.Src(n.src));
        if (n.sampling != UiProto.Sampling.NEAREST) p.add(new UiProto.Prop.Sampling(n.sampling));
        if (n.tint != 0xFFFFFFFF) p.add(new UiProto.Prop.Tint(n.tint));
        if (!n.points.isEmpty()) p.add(new UiProto.Prop.Points(n.points));
        if (n.width != 1) p.add(new UiProto.Prop.Width(n.width));
        if (n.hoverFill != null) p.add(new UiProto.Prop.HoverFill(n.hoverFill));
        if (n.hoverBorderColor != null) p.add(new UiProto.Prop.HoverBorderColor(n.hoverBorderColor));
        if (n.hoverColor != null) p.add(new UiProto.Prop.HoverColor(n.hoverColor));
        if (n.pressedFill != null) p.add(new UiProto.Prop.PressedFill(n.pressedFill));
        if (n.pressedBorderColor != null) p.add(new UiProto.Prop.PressedBorderColor(n.pressedBorderColor));
        if (n.pressedColor != null) p.add(new UiProto.Prop.PressedColor(n.pressedColor));
        if (n.scroll != 0) p.add(new UiProto.Prop.Scroll(n.scroll));
        if (n.contentW != 0) p.add(new UiProto.Prop.ContentW(n.contentW));
        if (n.contentH != 0) p.add(new UiProto.Prop.ContentH(n.contentH));
        if (n.scrollX != 0) p.add(new UiProto.Prop.ScrollX(n.scrollX));
        if (n.scrollY != 0) p.add(new UiProto.Prop.ScrollY(n.scrollY));
        if (n.dragWindow != 0) p.add(new UiProto.Prop.DragWindow(n.dragWindow));
        if (n.caret != null) p.add(new UiProto.Prop.Caret(n.caret));
        if (n.caretOf != null) p.add(new UiProto.Prop.CaretOf(n.caretOf));
        p.removeIf(x -> !UiProto.nodeAllows(n.kind, x.tag()));
        p.sort(java.util.Comparator.comparingInt(UiProto.Prop::tag));
        return p;
    }

    // =========================================================== stats

    public long steps() {
        return steps;
    }

    public long keyframes() {
        return keyframes;
    }

    public long stepBytes() {
        return stepBytes;
    }

    public long keyframeBytes() {
        return keyframeBytes;
    }

    public long generation() {
        synchronized (comp) {
            return gen;
        }
    }
}
