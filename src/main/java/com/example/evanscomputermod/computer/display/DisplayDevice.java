package com.example.evanscomputermod.computer.display;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.video.Rgb332Palette;

import java.util.Arrays;
import java.util.concurrent.locks.LockSupport;
import java.util.function.Supplier;

/**
 * A graphics output of the computer — the Terminal's built-in screen or the
 * attached in-world Screen cluster — as seen by programs.
 *
 * <p>This is the "display controller" between a program and the scanout
 * ({@link TerminalDisplay}, which is what gets streamed to clients):
 * <ul>
 *   <li><b>Ownership.</b> The kernel owns every display by default and draws
 *       through its own memory regions ({@code gfxtest}, the text console).
 *       A program takes a display over with {@code gfx_init}, like a process
 *       becoming DRM master; the display goes back to the kernel when the
 *       program exits, is killed, or switches the terminal back to text mode.
 *       A display already owned by another program is busy.</li>
 *   <li><b>Framebuffers.</b> The program writes pixels into a back buffer
 *       ({@code gfx_blit_rect}). A single-buffered display scans out after
 *       every write; a double-buffered one only on {@code gfx_present},
 *       so a half-drawn frame is never visible.</li>
 *   <li><b>Refresh.</b> The display refreshes at {@link #refreshHz()}; vertical
 *       blanks happen every {@code 1/refreshHz} seconds from a fixed epoch.
 *       {@code gfx_wait_vblank} and {@code gfx_present(WAIT_VBLANK)} sleep until
 *       the next one, which gives programs a real frame clock.</li>
 * </ul>
 *
 * <p>All methods are called on the calling program's thread. Mutators are
 * synchronized on the device; the vblank wait is not, so a program waiting for
 * a vblank never blocks another thread's writes.
 */
public final class DisplayDevice {

    /** {@code gfx_init2} flag: draw into a back buffer that {@code gfx_present} flips. */
    public static final int FLAG_DOUBLE_BUFFER = 1;
    /** {@code gfx_present} flag: wait for the next vertical blank before flipping. */
    public static final int PRESENT_WAIT_VBLANK = 1;

    /** Owner id of the kernel (pids start at 1). */
    public static final int KERNEL = 0;

    /** Error codes returned to programs (negative errno-style). */
    public static final int OK = 0;
    public static final int E_INVALID = -1;
    public static final int E_BUSY = -2;
    public static final int E_NOT_OWNER = -3;
    public static final int E_NO_DEVICE = -4;
    public static final int E_INTERRUPTED = -5;

    /** Largest dimension accepted. */
    public static final int MAX_DIM = 4096;

    private final String name;
    private final Supplier<TerminalDisplay> scanout;
    private final int maxPixelBytes;
    /** Called after every scanout: schedules a stream to clients. */
    private final Runnable onFrame;
    /** Called when the kernel gets the display back. */
    private final Runnable onRelease;
    private final RefreshLimits limits;

    private int ownerPid = KERNEL;
    private int width;
    private int height;
    private int format = TerminalDisplay.PIXEL_FORMAT_INDEXED8;
    private int mode;
    private int flags;
    private final int[] palette = new int[256];
    private boolean paletteChanged;
    private byte[] back = new byte[0];
    private int refreshHz;
    private long presented;
    private final long epochNanos = System.nanoTime();

    /** Refresh-rate policy (server config). */
    public interface RefreshLimits {
        int defaultHz();
        int maxHz();
    }

    public DisplayDevice(String name, Supplier<TerminalDisplay> scanout, int maxPixelBytes,
                         RefreshLimits limits, Runnable onFrame, Runnable onRelease) {
        this.name = name;
        this.scanout = scanout;
        this.maxPixelBytes = maxPixelBytes;
        this.limits = limits;
        this.onFrame = onFrame;
        this.onRelease = onRelease;
        this.refreshHz = clampHz(limits.defaultHz());
    }

    public String name() { return name; }

    public synchronized int ownerPid() { return ownerPid; }

    /** True while a program (not the kernel) is driving this display. */
    public synchronized boolean isOwnedByProgram() { return ownerPid != KERNEL; }

    public synchronized int refreshHz() { return refreshHz; }

    // ------------------------------------------------------------------
    // Mode setting
    // ------------------------------------------------------------------

    /**
     * Take the display over (if free) and set a mode: {@code w x h} pixels in
     * {@code format}, cleared to 0, with the default RGB332 palette, shown in
     * graphics mode. Re-initializing a display the caller already owns is
     * allowed and keeps nothing.
     */
    public int init(int pid, int w, int h, int format, int flags) {
        synchronized (this) {
            if (pid <= KERNEL) return E_INVALID;
            if (ownerPid != KERNEL && ownerPid != pid) return E_BUSY;
            if (w <= 0 || h <= 0 || w > MAX_DIM || h > MAX_DIM) return E_INVALID;
            if (!TerminalDisplay.isValidPixelFormat(format)) return E_INVALID;
            long bytes = (long) w * h * TerminalDisplay.bytesPerPixel(format);
            if (bytes > maxPixelBytes) return E_INVALID;
            if (scanout.get() == null) return E_NO_DEVICE;
            ownerPid = pid;
            this.width = w;
            this.height = h;
            this.format = format;
            this.flags = flags & FLAG_DOUBLE_BUFFER;
            this.mode = 1;
            int[] rgb332 = Rgb332Palette.argb();
            System.arraycopy(rgb332, 0, palette, 0, 256);
            paletteChanged = true;
            back = new byte[(int) bytes];
            scanOutLocked();
        }
        onFrame.run();
        return OK;
    }

    /**
     * Legacy {@code gfx_init(target, w, h)}: single-buffered. Keeps a format the
     * caller already chose with {@code set_format} (the old player called
     * {@code screen_set_pixel_format} before {@code gfx_init}); otherwise indexed8.
     */
    public int initLegacy(int pid, int w, int h) {
        int fmt;
        synchronized (this) {
            fmt = (ownerPid == pid) ? format : TerminalDisplay.PIXEL_FORMAT_INDEXED8;
        }
        return init(pid, w, h, fmt, 0);
    }

    /** Change the pixel format, keeping the size; pixels are cleared. Takes a free display over. */
    public int setFormat(int pid, int newFormat) {
        synchronized (this) {
            if (!TerminalDisplay.isValidPixelFormat(newFormat)) return E_INVALID;
            if (ownerPid == KERNEL) {
                // Not initialized yet: remember the format for the coming init.
                if (pid <= KERNEL) return E_INVALID;
                ownerPid = pid;
                format = newFormat;
                width = height = 0;
                back = new byte[0];
                return OK;
            }
            if (ownerPid != pid) return E_NOT_OWNER;
            long bytes = (long) width * height * TerminalDisplay.bytesPerPixel(newFormat);
            if (bytes > maxPixelBytes) return E_INVALID;
            format = newFormat;
            back = new byte[(int) bytes];
            if (width > 0 && height > 0) scanOutLocked();
        }
        onFrame.run();
        return OK;
    }

    /**
     * Display mode: 0 text, 1 graphics, 2 graphics with the text console on top.
     * Mode 0 hands the display back to the kernel.
     */
    public int setMode(int pid, int newMode) {
        if (newMode < 0 || newMode > 2) return E_INVALID;
        if (newMode == 0) {
            synchronized (this) {
                if (ownerPid == KERNEL) return OK;
                if (ownerPid != pid) return E_NOT_OWNER;
            }
            release(pid);
            return OK;
        }
        synchronized (this) {
            if (ownerPid != pid) return E_NOT_OWNER;
            mode = newMode;
            if (width > 0 && height > 0) scanOutLocked();
        }
        onFrame.run();
        return OK;
    }

    /** Set {@code count} palette entries from {@code rgb} (3 bytes each), starting at {@code first}. */
    public int setPalette(int pid, int first, byte[] rgb) {
        synchronized (this) {
            if (ownerPid != pid) return E_NOT_OWNER;
            int count = rgb.length / 3;
            if (first < 0 || count <= 0 || first + count > 256) return E_INVALID;
            for (int i = 0; i < count; i++) {
                palette[first + i] = 0xFF000000 | ((rgb[i * 3] & 0xFF) << 16)
                        | ((rgb[i * 3 + 1] & 0xFF) << 8) | (rgb[i * 3 + 2] & 0xFF);
            }
            paletteChanged = true;
            if ((flags & FLAG_DOUBLE_BUFFER) != 0) return OK;
            scanOutLocked();
        }
        onFrame.run();
        return OK;
    }

    public int setRefresh(int pid, int hz) {
        synchronized (this) {
            if (ownerPid != pid) return E_NOT_OWNER;
            refreshHz = clampHz(hz);
            return refreshHz;
        }
    }

    // ------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------

    /**
     * Copy a {@code w x h} rectangle, tightly packed in {@code srcFormat}, to
     * {@code (x, y)} of the back buffer. The format must be the display's.
     * The rectangle must lie inside the display.
     */
    public int blit(int pid, int x, int y, int w, int h, byte[] pixels, int srcFormat) {
        synchronized (this) {
            if (ownerPid != pid) return E_NOT_OWNER;
            if (srcFormat != format) return E_INVALID;
            if (x < 0 || y < 0 || w <= 0 || h <= 0 || x + w > width || y + h > height) return E_INVALID;
            int bpp = TerminalDisplay.bytesPerPixel(format);
            int rowBytes = w * bpp;
            if (pixels == null || pixels.length < rowBytes * h) return E_INVALID;
            int stride = width * bpp;
            if (x == 0 && w == width) {
                System.arraycopy(pixels, 0, back, y * stride, rowBytes * h);
            } else {
                for (int row = 0; row < h; row++) {
                    System.arraycopy(pixels, row * rowBytes, back, (y + row) * stride + x * bpp, rowBytes);
                }
            }
            if ((flags & FLAG_DOUBLE_BUFFER) != 0) return OK;
            scanOutLocked();
        }
        onFrame.run();
        return OK;
    }

    /**
     * Replace the whole frame, resizing or changing format if needed (used by
     * the host video decoder, which produces frames at its own size).
     */
    public int pushFrame(int pid, int w, int h, int fmt, byte[] pixels) {
        synchronized (this) {
            if (ownerPid != pid && ownerPid != KERNEL) return E_BUSY;
            if (!TerminalDisplay.isValidPixelFormat(fmt)) return E_INVALID;
            if (w <= 0 || h <= 0 || w > MAX_DIM || h > MAX_DIM) return E_INVALID;
            int bytes = w * h * TerminalDisplay.bytesPerPixel(fmt);
            if (bytes > maxPixelBytes || pixels == null || pixels.length < bytes) return E_INVALID;
            if (scanout.get() == null) return E_NO_DEVICE;
            if (ownerPid == KERNEL) {
                ownerPid = pid;
                mode = 1;
                System.arraycopy(Rgb332Palette.argb(), 0, palette, 0, 256);
                paletteChanged = true;
            }
            if (mode == 0) mode = 1;
            if (w != width || h != height || fmt != format || back.length != bytes) {
                width = w;
                height = h;
                format = fmt;
                back = new byte[bytes];
            }
            System.arraycopy(pixels, 0, back, 0, bytes);
            scanOutLocked();
        }
        onFrame.run();
        return OK;
    }

    /**
     * Show the back buffer. With {@link #PRESENT_WAIT_VBLANK}, first sleep until
     * the next vertical blank. Returns the number of frames presented so far, or
     * a negative error.
     */
    public long present(int pid, int presentFlags) {
        synchronized (this) {
            if (ownerPid != pid) return E_NOT_OWNER;
        }
        if ((presentFlags & PRESENT_WAIT_VBLANK) != 0) {
            if (waitVblank() < 0) return E_INTERRUPTED;
        }
        long n;
        synchronized (this) {
            if (ownerPid != pid) return E_NOT_OWNER;
            if (width <= 0 || height <= 0) return E_INVALID;
            scanOutLocked();
            n = presented;
        }
        onFrame.run();
        return n;
    }

    // ------------------------------------------------------------------
    // Timing
    // ------------------------------------------------------------------

    /** Vertical blanks since the display was created. */
    public long vblankCount() {
        long period = periodNanos();
        return (System.nanoTime() - epochNanos) / period;
    }

    /**
     * Sleep until the next vertical blank. Returns its number, or
     * {@link #E_INTERRUPTED} if the thread was interrupted (program killed).
     */
    public long waitVblank() {
        long period = periodNanos();
        long next = (System.nanoTime() - epochNanos) / period + 1;
        long deadline = epochNanos + next * period;
        while (true) {
            long remaining = deadline - System.nanoTime();
            if (remaining <= 0) return next;
            LockSupport.parkNanos(this, remaining);
            if (Thread.currentThread().isInterrupted()) return E_INTERRUPTED;
        }
    }

    private long periodNanos() {
        int hz;
        synchronized (this) {
            hz = refreshHz;
        }
        return 1_000_000_000L / Math.max(1, hz);
    }

    private int clampHz(int hz) {
        int max = Math.max(1, limits.maxHz());
        return Math.max(1, Math.min(max, hz));
    }

    // ------------------------------------------------------------------
    // Info / lifecycle
    // ------------------------------------------------------------------

    /** Snapshot for {@code gfx_info}. */
    public record Info(int width, int height, int format, int mode, int refreshHz, int flags,
                       int ownerPid, long vblank, long presented) {}

    public Info info() {
        long vb = vblankCount();
        synchronized (this) {
            return new Info(width, height, format, mode, refreshHz, flags, ownerPid, vb, presented);
        }
    }

    /** Give the display back to the kernel if {@code pid} owns it. */
    public void release(int pid) {
        synchronized (this) {
            if (ownerPid == KERNEL || ownerPid != pid) return;
            resetLocked();
        }
        onRelease.run();
    }

    /** Give the display back to the kernel whoever owns it (Ctrl+T, shutdown). */
    public void releaseAll() {
        synchronized (this) {
            if (ownerPid == KERNEL) return;
            resetLocked();
        }
        onRelease.run();
    }

    private void resetLocked() {
        ownerPid = KERNEL;
        width = height = 0;
        mode = 0;
        flags = 0;
        format = TerminalDisplay.PIXEL_FORMAT_INDEXED8;
        back = new byte[0];
        refreshHz = clampHz(limits.defaultHz());
        Arrays.fill(palette, 0);
    }

    /** Copy the back buffer to the scanout. Caller holds the monitor. */
    private void scanOutLocked() {
        TerminalDisplay out = scanout.get();
        if (out == null || width <= 0 || height <= 0) return;
        out.setGfxFrame(mode, width, height, format, paletteChanged ? palette : null, back);
        paletteChanged = false;
        presented++;
    }
}
