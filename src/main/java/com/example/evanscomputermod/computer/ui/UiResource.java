package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

/**
 * One resource of a connection (§6 "Resources"): an image uploaded once with
 * {@code ui_upload} (kept as premultiplied pixels plus a content hash, which
 * phase 2 uses to skip downloads), or a canvas fed with {@code ui_put_pixels}
 * (raw rows in its format, shown at the connection's next {@code Commit}).
 */
final class UiResource {
    final int id;
    final UiProto.ResourceKind kind;
    final int w, h;
    /** {@code TerminalDisplay.PIXEL_FORMAT_*} of a canvas; RGBA8888 for images. */
    final int format;
    /** Committed premultiplied pixels; null until an image is uploaded. */
    int[] argb;
    /** Canvas: rows written since the last commit, and where. */
    byte[] pendingRaw;
    final Box.Region pendingDamage = new Box.Region();
    /** FNV-1a 64 of the uploaded RGBA (images). */
    long hash;

    UiResource(int id, UiProto.ResourceKind kind, int w, int h, int format) {
        this.id = id;
        this.kind = kind;
        this.w = w;
        this.h = h;
        this.format = format;
        if (kind == UiProto.ResourceKind.CANVAS) {
            argb = new int[w * h];
        }
    }

    /** Bytes charged against the connection's resource limit. */
    long bytes() {
        long b = (long) w * h * 4;
        if (kind == UiProto.ResourceKind.CANVAS) b += (long) w * h * TerminalDisplay.bytesPerPixel(format);
        return b;
    }

    static long fnv64(byte[] data) {
        long h = 0xcbf29ce484222325L;
        for (byte b : data) {
            h ^= (b & 0xFF);
            h *= 0x100000001b3L;
        }
        return h;
    }

    /** Apply the canvas rows written since the last commit; returns whether anything changed. */
    boolean commitCanvas() {
        if (pendingRaw == null || pendingDamage.isEmpty()) return false;
        for (Box b : pendingDamage.boxes()) {
            Px.convert(pendingRaw, format, null, w, h, b.x0(), b.y0(), b.w(), b.h(), argb);
        }
        pendingDamage.clear();
        return true;
    }
}
