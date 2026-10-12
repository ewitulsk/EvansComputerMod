package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.display.DisplayDevice;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;

/**
 * A legacy program drawing with {@code gfx_init2} / {@code gfx_blit_rect} /
 * {@code gfx_present} while the GUI runs gets one of these instead of the
 * Terminal's display (ROADMAP P2 "Legacy compat", like XWayland): its own
 * {@link DisplayDevice} (so double buffering, palettes, vblank waits and
 * {@code gfx_info} behave exactly as on the console) whose scanout is a
 * private frame that the compositor shows as a pixel window. The program's
 * {@code mouse_poll} gets pointer events of that window in its own pixels;
 * its keys arrive on stdin as on the console.
 */
final class LegacyWindow implements UiConnection.LegacySink {
    private static final int MOUSE_RING = 32;

    final UiManager mgr;
    final int pid;
    final String title;
    final Scanout scanout = new Scanout();
    final DisplayDevice device;
    /** The window's internal connection, made with the first frame. */
    UiConnection conn;
    private final ArrayDeque<byte[]> mouse = new ArrayDeque<>();
    volatile boolean capture;

    LegacyWindow(UiManager mgr, int pid, String title, int maxBytes, DisplayDevice.RefreshLimits limits) {
        this.mgr = mgr;
        this.pid = pid;
        this.title = title;
        this.device = new DisplayDevice("window-" + pid, () -> scanout, maxBytes, limits, mgr::legacyDirty,
                () -> mgr.legacyReleased(this));
    }

    /** The private frame the device scans out into; remembers what changed. */
    final class Scanout extends TerminalDisplay {
        boolean dirty, whole, palette;
        final List<Box> rects = new ArrayList<>();

        Scanout() {
            super(1, 1);
        }

        @Override
        public synchronized void setGfxFrame(int mode, int w, int h, int format, int[] paletteArgb, byte[] pixels) {
            super.setGfxFrame(mode, w, h, format, paletteArgb, pixels);
            dirty = true;
            whole = true;
            palette |= paletteArgb != null;
            rects.clear();
        }

        @Override
        public synchronized boolean setGfxRect(int mode, int w, int h, int format, byte[] frame, int rx, int ry, int rw, int rh) {
            boolean ok = super.setGfxRect(mode, w, h, format, frame, rx, ry, rw, rh);
            if (ok) {
                dirty = true;
                if (!whole) {
                    rects.add(Box.xywh(rx, ry, rw, rh));
                    if (rects.size() > 32) whole = true;
                }
            }
            return ok;
        }
    }

    /** Copy what the program scanned out into its window (compositor lock held). */
    void pull(Compositor comp) {
        synchronized (scanout) {
            if (!scanout.dirty) return;
            int w = scanout.getGfxWidth(), h = scanout.getGfxHeight();
            byte[] px = scanout.getPixelData();
            if (w <= 0 || h <= 0 || px == null) return;
            int fmt = scanout.getPixelFormat();
            if (conn == null) conn = comp.openLegacy(pid, title, w, h, fmt, this);
            comp.legacyFrame(conn, px, fmt, scanout.palette ? scanout.getPalette() : null, w, h,
                    scanout.whole ? null : new ArrayList<>(scanout.rects));
            scanout.dirty = false;
            scanout.whole = false;
            scanout.palette = false;
            scanout.rects.clear();
        }
    }

    // ---------------------------------------------------------------- input

    @Override
    public void pointer(int kind, int x, int y, int buttons, int button, int scroll) {
        if (!capture) return;
        // Protocol kinds -> mouse_poll kinds (MouseInputPacket): move 1, down 2, up 3, scroll 4.
        byte k = switch (kind) {
            case 2 -> 1;
            case 3 -> 2;
            case 4 -> 3;
            case 5 -> 4;
            default -> 0;
        };
        if (k == 0) return;
        byte[] e = new byte[10];
        java.nio.ByteBuffer.wrap(e).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                .put(k).putShort((short) x).putShort((short) y).put((byte) buttons)
                .put((byte) (button > 0 ? button - 1 : 0)).put((byte) Integer.signum(scroll));
        synchronized (mouse) {
            if (mouse.size() >= MOUSE_RING) mouse.pollFirst();
            mouse.addLast(e);
        }
    }

    int pollMouse(byte[] out) {
        byte[] e;
        synchronized (mouse) {
            e = mouse.pollFirst();
        }
        if (e == null) return 0;
        System.arraycopy(e, 0, out, 0, 10);
        return 1;
    }

    void stopCapture() {
        capture = false;
        synchronized (mouse) {
            mouse.clear();
        }
    }

    @Override
    public void key(int hid, int mods, boolean down) {
        // Legacy programs read keys from stdin; the typed strings arrive through UiManager.legacyText.
    }

    @Override
    public void close() {
        mgr.killProgram(pid);
    }
}
