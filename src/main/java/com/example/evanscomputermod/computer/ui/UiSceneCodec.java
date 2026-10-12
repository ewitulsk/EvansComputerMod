package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.ui.proto.UiProtoException;
import com.example.evanscomputermod.computer.ui.proto.UiReader;
import com.example.evanscomputermod.computer.ui.proto.UiWriter;

import java.io.ByteArrayOutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.zip.DataFormatException;
import java.util.zip.Deflater;
import java.util.zip.Inflater;

/**
 * The scene stream's wire format (UI_PROTOCOL.md §14 phase 2a, §17): what
 * the server sends a client so it can draw a computer's GUI itself.
 *
 * <p>A message is {@code u8 kind} (bit 7 set: the body is stored, not
 * deflated, which small steps are), then for every kind but STEP a
 * {@code u32 epoch} (it changes whenever the compositor, and so the
 * generation counter, starts over; a step belongs to the epoch of the
 * keyframe before it), then the body. Bodies use the UI protocol's
 * little-endian fields ({@link UiWriter}), LEB128 varints ({@code var})
 * where counts and generations are small, and the protocol's own
 * {@code TreeOp} encoding, so a tree patch means exactly what the app sent:
 *
 * <pre>
 * STEP:     var to (from = to - 1) · u8 flags(1 header) · [var serverNowMs · Header] · Trees · Pixels
 * KEYFRAME: u32 version · var to · var serverNowMs · Header · Trees · Pixels
 * RESOURCE: u32 version · u64 key · u16 w · u16 h · premultiplied ARGB (w·h, u32 each)
 * END:      (no body) the display left GUI client rendering; tile deltas follow
 * Trees:    var n · {var surface · var n · TreeOp…}…
 * Pixels:   var n · {u8 target(0 surface, 1 canvas) · var scope · var id · u16 w · u16 h · var tiles+1 (0 = all)
 *                    · var tileIndex… · premultiplied ARGB of each 16x16 tile, rows clipped}…
 * Header:   u16 deviceW · u16 deviceH · u8 scale · u8 tier · u8 device · u8 safeMode · str safeText · u32 focusedWindow
 *           · u32 n · Surface{u32 id · u32 scope · u8 kind · u8 role · u8 layer · u32 parent · i32 x · i32 y
 *                             · u32 contentW · u32 contentH · u8 flags(1 mapped, 2 popupDone) · [pixels: u16 bw · u16 bh · u8 scale]}…
 *           · u32 n · layer surface id… · u32 n · popup surface id…
 *           · u32 n · Window{u32 id · u32 surface · u8 flags(1 placed, 2 visible) · u32 states · i32 anim[14]
 *                            · u64 placeStart · u64 alphaStart}… (bottom to top)
 *           · u32 n · Resource{u32 scope · u32 id · u8 kind · u16 w · u16 h · u64 key}…
 * </pre>
 *
 * Surface ids are scene ids the exporter assigns (not an app's own ids);
 * {@code scope} names the connection whose resources a surface's nodes use.
 * A STEP's header, when present, replaces the whole previous one. Times are
 * the server compositor's clock; the client keeps the offset to its own.
 */
public final class UiSceneCodec {
    /** Version of the scene stream; a client reports it in its hello and the server sends patches only on a match. */
    public static final int VERSION = 1;

    public static final int STEP = 1, KEYFRAME = 2, END = 3, RESOURCE = 4;
    /** {@link PixelBlock#target}: a pixel surface (by scene id) or a canvas resource (by scope and id). */
    public static final int TARGET_SURFACE = 0, TARGET_CANVAS = 1;
    public static final int TILE = 16;

    private UiSceneCodec() {}

    // ------------------------------------------------------------------ model

    public record SurfaceInfo(int id, int scope, UiProto.SurfaceKind kind, UiProto.SurfaceRole role, UiProto.Layer layer,
                              int parent, int x, int y, int contentW, int contentH, boolean mapped, boolean popupDone,
                              int bw, int bh, int pscale) {}

    public record WindowInfo(int id, int surface, boolean placed, boolean visible, int states, int[] anim,
                             long placeStart, long alphaStart) {}

    public record ResourceInfo(int scope, int id, UiProto.ResourceKind kind, int w, int h, long key) {}

    public record Header(int deviceW, int deviceH, int scale, int tier, UiProto.Device device, boolean safeMode,
                         String safeText, int focusedWindow, List<SurfaceInfo> surfaces, List<Integer> layers,
                         List<Integer> popups, List<WindowInfo> windows, List<ResourceInfo> resources) {
        public UiDisplay display() {
            return new UiDisplay(deviceW, deviceH, scale, tier, device);
        }
    }

    public record TreeBlock(int surface, List<UiProto.TreeOp> ops) {}

    /** Tiles of a {@code w x h} buffer; {@code tiles == null} means all of them, in order. */
    public record PixelBlock(int target, int scope, int id, int w, int h, int[] tiles, int[] pixels) {}

    public record Patch(boolean keyframe, long from, long to, long serverNowMs, Header header,
                        List<TreeBlock> trees, List<PixelBlock> pixels) {}

    public record Resource(long key, int w, int h, int[] argb) {}

    /** One message off the wire. */
    public record Message(int kind, int epoch, Patch patch, Resource resource) {}

    // ------------------------------------------------------------------ encode

    public static void writeHeader(UiWriter w, Header h) {
        w.u16(h.deviceW);
        w.u16(h.deviceH);
        w.u8(h.scale);
        w.u8(h.tier);
        w.u8(h.device.code);
        w.bool(h.safeMode);
        w.str(h.safeText);
        w.u32(h.focusedWindow);
        w.u32(h.surfaces.size());
        for (SurfaceInfo s : h.surfaces) {
            w.u32(s.id);
            w.u32(s.scope);
            w.u8(s.kind.code);
            w.u8(s.role.code);
            w.u8(s.layer.code);
            w.u32(s.parent);
            w.i32(s.x);
            w.i32(s.y);
            w.u32(s.contentW);
            w.u32(s.contentH);
            w.u8((s.mapped ? 1 : 0) | (s.popupDone ? 2 : 0));
            if (s.kind == UiProto.SurfaceKind.PIXELS) {
                w.u16(s.bw);
                w.u16(s.bh);
                w.u8(s.pscale);
            }
        }
        w.u32(h.layers.size());
        for (int id : h.layers) w.u32(id);
        w.u32(h.popups.size());
        for (int id : h.popups) w.u32(id);
        w.u32(h.windows.size());
        for (WindowInfo win : h.windows) {
            w.u32(win.id);
            w.u32(win.surface);
            w.u8((win.placed ? 1 : 0) | (win.visible ? 2 : 0));
            w.u32(win.states);
            for (int i = 0; i < 14; i++) w.i32(win.anim[i]);
            w.u64(win.placeStart);
            w.u64(win.alphaStart);
        }
        w.u32(h.resources.size());
        for (ResourceInfo r : h.resources) {
            w.u32(r.scope);
            w.u32(r.id);
            w.u8(r.kind.code);
            w.u16(r.w);
            w.u16(r.h);
            w.u64(r.key);
        }
    }

    /** The body of a STEP or KEYFRAME; {@code header} is pre-encoded ({@link #writeHeader}) or null (steps only). */
    public static byte[] encodePatch(boolean keyframe, long from, long to, long now, byte[] header,
                                     List<TreeBlock> trees, List<PixelBlock> pixels) {
        UiWriter w = new UiWriter(64 + (header == null ? 0 : header.length));
        if (keyframe) {
            w.u32(VERSION);
            var(w, to);
            var(w, now);
            raw(w, header);
        } else {
            if (from != to - 1) throw new IllegalArgumentException("a step goes one generation");
            var(w, to);
            w.u8(header != null ? 1 : 0);
            if (header != null) {
                var(w, now);
                raw(w, header);
            }
        }
        var(w, trees.size());
        for (TreeBlock t : trees) {
            var(w, t.surface & 0xFFFFFFFFL);
            var(w, t.ops.size());
            for (UiProto.TreeOp op : t.ops) op.enc(w);
        }
        var(w, pixels.size());
        for (PixelBlock p : pixels) {
            w.u8(p.target);
            var(w, p.scope & 0xFFFFFFFFL);
            var(w, p.id & 0xFFFFFFFFL);
            w.u16(p.w);
            w.u16(p.h);
            var(w, p.tiles == null ? 0 : p.tiles.length + 1L);
            if (p.tiles != null) for (int t : p.tiles) var(w, t);
            for (int c : p.pixels) w.u32(c);
        }
        return w.toByteArray();
    }

    private static void raw(UiWriter w, byte[] b) {
        for (byte x : b) w.u8(x);
    }

    /** LEB128 (unsigned). */
    static void var(UiWriter w, long v) {
        while ((v & ~0x7FL) != 0) {
            w.u8((int) ((v & 0x7F) | 0x80));
            v >>>= 7;
        }
        w.u8((int) v);
    }

    static long var(UiReader r) throws UiProtoException {
        long v = 0;
        for (int shift = 0; shift < 64; shift += 7) {
            int b = r.u8();
            v |= (long) (b & 0x7F) << shift;
            if ((b & 0x80) == 0) return v;
        }
        throw UiProtoException.malformed("varint too long");
    }

    private static int varInt(UiReader r, long max, String what) throws UiProtoException {
        long v = var(r);
        if (v < 0 || v > max) throw UiProtoException.malformed("out of range: " + what);
        return (int) v;
    }

    /** Bodies smaller than this are stored rather than deflated (deflate only adds bytes to them). */
    static final int RAW_BELOW = 128;
    private static final int RAW = 0x80;

    public static byte[] encodeResource(long key, int w, int h, int[] argb) {
        UiWriter out = new UiWriter(16 + argb.length * 4);
        out.u32(VERSION);
        out.u64(key);
        out.u16(w);
        out.u16(h);
        for (int c : argb) out.u32(c);
        return out.toByteArray();
    }

    /** A whole message: kind, the epoch (not for STEP), the body (deflated unless that doesn't pay). */
    public static byte[] pack(int kind, int epoch, byte[] body, Deflater deflater) {
        ByteArrayOutputStream out = new ByteArrayOutputStream(16 + (body == null ? 0 : body.length / 2));
        byte[] packed = null;
        if (body != null && body.length >= RAW_BELOW) {
            deflater.reset();
            deflater.setInput(body);
            deflater.finish();
            ByteArrayOutputStream z = new ByteArrayOutputStream(body.length / 2 + 16);
            byte[] buf = new byte[8192];
            while (!deflater.finished()) {
                int n = deflater.deflate(buf);
                z.write(buf, 0, n);
            }
            if (z.size() < body.length) packed = z.toByteArray();
        }
        boolean raw = body != null && packed == null;
        out.write(kind | (raw ? RAW : 0));
        if (kind != STEP) {
            out.write(epoch);
            out.write(epoch >>> 8);
            out.write(epoch >>> 16);
            out.write(epoch >>> 24);
        }
        if (body != null) {
            byte[] b = raw ? body : packed;
            out.write(b, 0, b.length);
        }
        return out.toByteArray();
    }

    /** The kind of a packed message. */
    public static int kindOf(byte[] data) {
        return data.length == 0 ? -1 : data[0] & 0x7F;
    }

    // ------------------------------------------------------------------ decode

    /** Decode a packed message. A STEP's epoch is reported as -1 (it is the last keyframe's). */
    public static Message unpack(byte[] data) throws UiProtoException {
        if (data.length < 1) throw UiProtoException.malformed("empty scene message");
        int kind = data[0] & 0x7F;
        boolean raw = (data[0] & RAW) != 0;
        int off = 1, epoch = -1;
        if (kind != STEP) {
            if (data.length < 5) throw UiProtoException.malformed("scene message too short");
            epoch = (data[1] & 0xFF) | (data[2] & 0xFF) << 8 | (data[3] & 0xFF) << 16 | (data[4] & 0xFF) << 24;
            off = 5;
        }
        if (kind == END) return new Message(kind, epoch, null, null);
        byte[] body = raw ? java.util.Arrays.copyOfRange(data, off, data.length) : inflate(data, off);
        UiReader r = new UiReader(body, 0, body.length);
        if (kind != STEP) {
            int version = r.u32();
            if (version != VERSION) throw UiProtoException.malformed("scene stream version " + version + ", expected " + VERSION);
        }
        return switch (kind) {
            case STEP, KEYFRAME -> new Message(kind, epoch, readPatch(r, kind == KEYFRAME), null);
            case RESOURCE -> {
                long key = r.u64();
                int w = r.u16(), h = r.u16();
                if ((long) w * h * 4 != r.remaining()) throw UiProtoException.malformed("resource size");
                int[] px = new int[w * h];
                for (int i = 0; i < px.length; i++) px[i] = r.u32();
                r.finish();
                yield new Message(kind, epoch, null, new Resource(key, w, h, px));
            }
            default -> throw UiProtoException.malformed("unknown scene message kind " + kind);
        };
    }

    private static byte[] inflate(byte[] data, int off) throws UiProtoException {
        Inflater inf = new Inflater();
        try {
            inf.setInput(data, off, data.length - off);
            ByteArrayOutputStream out = new ByteArrayOutputStream(Math.max(64, (data.length - off) * 4));
            byte[] buf = new byte[16384];
            while (!inf.finished()) {
                int n = inf.inflate(buf);
                if (n == 0 && (inf.needsInput() || inf.needsDictionary())) {
                    throw UiProtoException.malformed("truncated scene message");
                }
                out.write(buf, 0, n);
                if (out.size() > (64 << 20)) throw UiProtoException.malformed("scene message too large");
            }
            return out.toByteArray();
        } catch (DataFormatException e) {
            throw UiProtoException.malformed("bad scene message compression");
        } finally {
            inf.end();
        }
    }

    private static Patch readPatch(UiReader r, boolean keyframe) throws UiProtoException {
        long to = var(r), from, now = 0;
        Header header = null;
        if (keyframe) {
            from = 0;
            now = var(r);
            header = readHeader(r);
        } else {
            if (to < 1) throw UiProtoException.malformed("step to generation 0");
            from = to - 1;
            if ((r.u8() & 1) != 0) {
                now = var(r);
                header = readHeader(r);
            }
        }
        int nt = varInt(r, 1 << 20, "trees");
        List<TreeBlock> trees = new ArrayList<>(Math.min(nt, 1024));
        for (int i = 0; i < nt; i++) {
            int surface = (int) var(r);
            int n = varInt(r, 1 << 22, "ops");
            if (n > r.remaining() / 5) throw UiProtoException.malformed("truncated: ops");
            List<UiProto.TreeOp> ops = new ArrayList<>(n);
            for (int k = 0; k < n; k++) ops.add(UiProto.TreeOp.dec(r));
            trees.add(new TreeBlock(surface, ops));
        }
        int np = varInt(r, 1 << 20, "pixels");
        List<PixelBlock> pixels = new ArrayList<>(Math.min(np, 1024));
        for (int i = 0; i < np; i++) {
            int target = r.u8();
            int scope = (int) var(r), id = (int) var(r);
            int w = r.u16(), h = r.u16();
            int tx = (w + TILE - 1) / TILE, ty = (h + TILE - 1) / TILE;
            int nTiles = varInt(r, tx * ty + 1L, "tiles");
            int[] tiles = null;
            long count;
            if (nTiles == 0) {
                count = (long) w * h;
            } else {
                tiles = new int[nTiles - 1];
                count = 0;
                for (int k = 0; k < tiles.length; k++) {
                    int t = varInt(r, tx * ty - 1L, "tile");
                    tiles[k] = t;
                    count += (long) tileW(t, tx, w) * tileH(t, tx, h);
                }
            }
            if (count * 4 > r.remaining()) throw UiProtoException.malformed("pixel block truncated");
            int[] px = new int[(int) count];
            for (int k = 0; k < px.length; k++) px[k] = r.u32();
            pixels.add(new PixelBlock(target, scope, id, w, h, tiles, px));
        }
        r.finish();
        return new Patch(keyframe, from, to, now, header, trees, pixels);
    }

    static int tileW(int t, int tilesX, int w) {
        int x0 = (t % tilesX) * TILE;
        return Math.min(TILE, w - x0);
    }

    static int tileH(int t, int tilesX, int h) {
        int y0 = (t / tilesX) * TILE;
        return Math.min(TILE, h - y0);
    }

    static Header readHeader(UiReader r) throws UiProtoException {
        int dw = r.u16(), dh = r.u16(), scale = r.u8(), tier = r.u8();
        UiProto.Device device = UiProto.Device.fromCode(r.u8(), "device");
        if (dw < 1 || dh < 1 || scale < 1) throw UiProtoException.malformed("bad display");
        boolean safe = r.bool("safeMode");
        String safeText = r.str(1 << 16, "safeText");
        int focused = r.u32();
        int ns = r.count(1 << 16, 30, "surfaces");
        List<SurfaceInfo> surfaces = new ArrayList<>(ns);
        for (int i = 0; i < ns; i++) {
            int id = r.u32(), scope = r.u32();
            UiProto.SurfaceKind kind = UiProto.SurfaceKind.fromCode(r.u8(), "kind");
            UiProto.SurfaceRole role = UiProto.SurfaceRole.fromCode(r.u8(), "role");
            UiProto.Layer layer = UiProto.Layer.fromCode(r.u8(), "layer");
            int parent = r.u32(), x = r.i32(), y = r.i32(), cw = r.u32(), ch = r.u32();
            int flags = r.u8();
            int bw = 0, bh = 0, ps = 1;
            if (kind == UiProto.SurfaceKind.PIXELS) {
                bw = r.u16();
                bh = r.u16();
                ps = Math.max(1, r.u8());
            }
            surfaces.add(new SurfaceInfo(id, scope, kind, role, layer, parent, x, y, cw, ch, (flags & 1) != 0,
                    (flags & 2) != 0, bw, bh, ps));
        }
        List<Integer> layers = ids(r, "layers");
        List<Integer> popups = ids(r, "popups");
        int nw = r.count(1 << 16, 82, "windows");
        List<WindowInfo> windows = new ArrayList<>(nw);
        for (int i = 0; i < nw; i++) {
            int id = r.u32(), surface = r.u32(), flags = r.u8(), states = r.u32();
            int[] anim = new int[14];
            for (int k = 0; k < 14; k++) anim[k] = r.i32();
            long ps = r.u64(), as = r.u64();
            windows.add(new WindowInfo(id, surface, (flags & 1) != 0, (flags & 2) != 0, states, anim, ps, as));
        }
        int nr = r.count(1 << 20, 21, "resources");
        List<ResourceInfo> resources = new ArrayList<>(nr);
        for (int i = 0; i < nr; i++) {
            int scope = r.u32(), id = r.u32();
            UiProto.ResourceKind kind = UiProto.ResourceKind.fromCode(r.u8(), "resource kind");
            int w = r.u16(), h = r.u16();
            long key = r.u64();
            resources.add(new ResourceInfo(scope, id, kind, w, h, key));
        }
        return new Header(dw, dh, scale, tier, device, safe, safeText, focused, surfaces, layers, popups, windows, resources);
    }

    private static List<Integer> ids(UiReader r, String what) throws UiProtoException {
        int n = r.count(1 << 16, 4, what);
        List<Integer> out = new ArrayList<>(n);
        for (int i = 0; i < n; i++) out.add(r.u32());
        return out;
    }

    /** An image's cache key: its content hash with its size folded in (the same bytes at another size differ). */
    public static long imageKey(long hash, int w, int h) {
        long k = hash ^ (((long) w << 32) | (h & 0xFFFFFFFFL)) * 0x9E3779B97F4A7C15L;
        return k == 0 ? 1 : k;
    }
}
