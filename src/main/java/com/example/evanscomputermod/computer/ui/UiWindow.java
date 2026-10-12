package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;

/**
 * A window as the shell sees it (§8): a compositor-wide id for a window-role
 * surface, its placement and opacity (both animatable with a declarative
 * transition, run by the compositor at the display's refresh rate),
 * visibility and window states.
 */
final class UiWindow {
    final int id;
    final Surface surface;
    /** Placed by the shell (or safe mode) at least once: only placed windows show. */
    boolean placed;
    boolean visible = true;
    int states;
    /** Size and states last sent in a Configure. */
    int cfgW = -1, cfgH = -1, cfgStates = -1;

    // Placement animation: from -> to over [t0, t0 + ms).
    private int fx, fy, fw, fh;
    int x, y, w, h;
    private long t0;
    private int ms;
    private UiProto.Easing easing = UiProto.Easing.LINEAR;

    private int fromAlpha = 255;
    int alpha = 255;
    private long at0;
    private int ams;
    private UiProto.Easing aeasing = UiProto.Easing.LINEAR;

    UiWindow(int id, Surface surface) {
        this.id = id;
        this.surface = surface;
    }

    void place(int nx, int ny, int nw, int nh, UiProto.Transition tr, long nowMs) {
        int[] cur = rect(nowMs);
        if (placed && tr != null && tr.ms() > 0) {
            fx = cur[0];
            fy = cur[1];
            fw = cur[2];
            fh = cur[3];
            t0 = nowMs;
            ms = tr.ms();
            easing = tr.easing();
        } else {
            ms = 0;
        }
        x = nx;
        y = ny;
        w = nw;
        h = nh;
        placed = true;
    }

    void setAlpha(int a, UiProto.Transition tr, long nowMs) {
        int cur = alpha(nowMs);
        if (tr != null && tr.ms() > 0) {
            fromAlpha = cur;
            at0 = nowMs;
            ams = tr.ms();
            aeasing = tr.easing();
        } else {
            ams = 0;
        }
        alpha = a;
    }

    /**
     * The placement and opacity animation state, for a client mirror
     * ({@link SceneExporter} writes it, {@link UiSceneMirror} reads it back
     * with {@link #restoreAnimation}): {x, y, w, h, fx, fy, fw, fh, ms,
     * easing, alpha, fromAlpha, ams, aeasing} plus the two start times.
     */
    int[] animation() {
        return new int[]{x, y, w, h, fx, fy, fw, fh, ms, easing.code, alpha, fromAlpha, ams, aeasing.code};
    }

    long placeStartMs() {
        return t0;
    }

    long alphaStartMs() {
        return at0;
    }

    void restoreAnimation(int[] a, long placeStart, long alphaStart) throws com.example.evanscomputermod.computer.ui.proto.UiProtoException {
        x = a[0];
        y = a[1];
        w = a[2];
        h = a[3];
        fx = a[4];
        fy = a[5];
        fw = a[6];
        fh = a[7];
        ms = a[8];
        easing = UiProto.Easing.fromCode(a[9], "easing");
        alpha = a[10];
        fromAlpha = a[11];
        ams = a[12];
        aeasing = UiProto.Easing.fromCode(a[13], "easing");
        t0 = placeStart;
        at0 = alphaStart;
    }

    boolean animating(long nowMs) {
        return (ms > 0 && nowMs < t0 + ms) || (ams > 0 && nowMs < at0 + ams);
    }

    /** Current {x, y, w, h} in lu. */
    int[] rect(long nowMs) {
        if (ms <= 0 || nowMs >= t0 + ms) return new int[]{x, y, w, h};
        int p = ease(easing, (int) ((nowMs - t0) * 1024 / ms));
        return new int[]{lerp(fx, x, p), lerp(fy, y, p), lerp(fw, w, p), lerp(fh, h, p)};
    }

    int alpha(long nowMs) {
        if (ams <= 0 || nowMs >= at0 + ams) return alpha;
        return lerp(fromAlpha, alpha, ease(aeasing, (int) ((nowMs - at0) * 1024 / ams)));
    }

    private static int lerp(int a, int b, int p) {
        return a + (int) (((long) (b - a) * p) / 1024);
    }

    /** Easing on a 0..1024 progress, integer only (deterministic for the client in phase 2). */
    static int ease(UiProto.Easing e, int p) {
        p = Math.max(0, Math.min(1024, p));
        return switch (e) {
            case LINEAR -> p;
            case EASE_IN -> p * p / 1024;
            case EASE_OUT -> 1024 - (1024 - p) * (1024 - p) / 1024;
            case EASE_IN_OUT -> p < 512 ? 2 * p * p / 1024 : 1024 - 2 * (1024 - p) * (1024 - p) / 1024;
        };
    }
}
