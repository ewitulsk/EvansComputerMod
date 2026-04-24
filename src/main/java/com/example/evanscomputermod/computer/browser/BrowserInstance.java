package com.example.evanscomputermod.computer.browser;

import org.cef.CefApp;
import org.cef.CefClient;
import org.cef.browser.CefBrowser;
import org.cef.browser.CefPaintEvent;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import javax.swing.SwingUtilities;
import java.awt.AWTEvent;
import java.awt.Component;
import java.awt.Frame;
import java.awt.GraphicsConfiguration;
import java.awt.GraphicsEnvironment;
import java.awt.event.InputEvent;
import java.awt.event.MouseEvent;
import java.awt.event.MouseWheelEvent;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;

/**
 * One off-screen CEF browser plus the staging slot for its most recent paint.
 * jcefmaven 122's {@link CefClient} already implements {@code CefRenderHandler}
 * and exposes paint callbacks via {@link CefClient#addOnPaintListener}, so we
 * just subscribe instead of overriding the handler. Each {@code onPaint}
 * callback hands us a BGRA {@link ByteBuffer} which we swap to RGBA and store
 * in a single-slot buffer guarded by {@link #frameLock}. The terminal worker
 * thread pulls frames with {@link #takeFrame()} once per rendered WASI tick.
 *
 * <p>Mouse events arrive from the WASI child via {@link #sendMouse(byte[])}
 * in the same 10-byte LE layout that {@code mouse_poll} produces. They are
 * translated to AWT {@link MouseEvent}s and dispatched to the browser's UI
 * component ({@code CefBrowserOsr}'s GLCanvas), whose installed mouse
 * listeners forward them into CEF.
 */
public final class BrowserInstance {

    private static final Logger LOG = LoggerFactory.getLogger(BrowserInstance.class);

    /**
     * Fallback source for AWT {@link MouseEvent}s if the browser's own UI
     * component is not yet available. The AWT {@code MouseEvent} constructor
     * rejects null sources. {@link Component}'s no-arg constructor — unlike
     * {@code Label}, {@code Canvas}, or {@code Frame} — does not call
     * {@code GraphicsEnvironment.checkHeadless()}, so this is safe to
     * initialize eagerly even if the headless flip hasn't taken effect.
     */
    private static final Component DUMMY_COMPONENT = new Component() {};

    final int width;
    final int height;
    private final CefClient client;
    private final CefBrowser browser;
    /**
     * Off-screen AWT container that hosts the JOGL {@code GLCanvas}. CEF's
     * OSR path ({@code CefBrowserOsr.onPaint}) silently returns when the
     * canvas's GL context is null, and the context only initializes once
     * the canvas has a native peer — i.e. once it's been added to a
     * realized, visible AWT window. We give it one, positioned off-screen
     * and transparent so the user never sees it.
     */
    private final Frame hiddenFrame;

    /** Latest paint, already byte-swapped to RGBA. Guarded by {@link #frameLock}. */
    private final Object frameLock = new Object();
    private byte[] latestFrameRgba;
    private final AtomicBoolean hasNewFrame = new AtomicBoolean(false);

    /**
     * Diagnostic counters for the OSR paint path. We log the first handful of
     * paint arrivals (match or mismatch) so a black-screen debug doesn't
     * require more instrumentation; once we know paints are flowing, the
     * counters cap and stop spamming.
     */
    private final AtomicInteger paintsSeen = new AtomicInteger(0);
    private final AtomicInteger paintsDropped = new AtomicInteger(0);

    private boolean closed = false;

    private BrowserInstance(int width, int height, CefClient client,
                            CefBrowser browser, Frame hiddenFrame) {
        this.width = width;
        this.height = height;
        this.client = client;
        this.browser = browser;
        this.hiddenFrame = hiddenFrame;
    }

    /**
     * Create a new off-screen browser. Subscribes to {@code onPaint} events
     * and kicks the browser into existence synchronously so the first tick's
     * render call has a chance of producing a frame.
     */
    static BrowserInstance create(CefApp app, String url, int width, int height) {
        CefClient client = app.createClient();

        // All AWT-peer work — createBrowser (which constructs a JOGL
        // GLCanvas), the host Frame, and setVisible — must run on the EDT.
        // On Windows, AWT/JOGL's toolkit lock on a non-EDT thread is a
        // NullToolkitLock, so chooseGraphicsConfiguration resolves a 0x0
        // device handle and throws. The WASI worker that calls us here is
        // definitely not the EDT.
        AtomicReference<CefBrowser> browserRef = new AtomicReference<>();
        AtomicReference<Frame> frameRef = new AtomicReference<>();
        Runnable edtWork = () -> {
            // isOSR=true routes paints through the on-paint listener chain
            // instead of compositing to a window; isTransparent=false gives
            // us an opaque background so pages without body backgrounds
            // still read cleanly.
            CefBrowser browser = client.createBrowser(url, true, false);
            browserRef.set(browser);

            // Explicitly bind the Frame to the default screen's
            // GraphicsConfiguration so addNotify doesn't fall back through
            // a null device during peer realization.
            GraphicsConfiguration gc = GraphicsEnvironment
                    .getLocalGraphicsEnvironment()
                    .getDefaultScreenDevice()
                    .getDefaultConfiguration();
            Frame hidden = new Frame(gc);
            hidden.setUndecorated(true);
            hidden.setSize(width, height);
            hidden.setLocation(-32000, -32000);
            Component canvas = browser.getUIComponent();
            if (canvas != null) {
                canvas.setSize(width, height);
                hidden.add(canvas);
            }
            // setVisible(true) is required to trigger addNotify on the canvas;
            // the off-screen location keeps it out of the user's view.
            hidden.setVisible(true);
            frameRef.set(hidden);
            LOG.info("BrowserInstance.create EDT work done: frame displayable={} visible={}"
                            + " size={}x{} canvas={} canvasDisplayable={} canvasSize={}x{}",
                    hidden.isDisplayable(), hidden.isVisible(),
                    hidden.getWidth(), hidden.getHeight(),
                    canvas == null ? "null" : canvas.getClass().getName(),
                    canvas != null && canvas.isDisplayable(),
                    canvas == null ? -1 : canvas.getWidth(),
                    canvas == null ? -1 : canvas.getHeight());
        };
        try {
            if (SwingUtilities.isEventDispatchThread()) {
                edtWork.run();
            } else {
                SwingUtilities.invokeAndWait(edtWork);
            }
        } catch (InterruptedException ie) {
            Thread.currentThread().interrupt();
            throw new RuntimeException("interrupted while creating CEF browser on EDT", ie);
        } catch (java.lang.reflect.InvocationTargetException ite) {
            throw new RuntimeException("failed to create CEF browser on EDT", ite.getCause());
        }

        BrowserInstance inst = new BrowserInstance(width, height, client,
                browserRef.get(), frameRef.get());
        client.addOnPaintListener(inst::acceptPaint);
        browserRef.get().createImmediately();
        LOG.info("BrowserInstance.create: createImmediately returned for url={} size={}x{}",
                url, width, height);
        return inst;
    }

    private void acceptPaint(CefPaintEvent ev) {
        // Defensive: CEF can emit paints at a different size than our view
        // rect immediately after a resize. Treat mismatched sizes as no-ops
        // rather than writing out-of-bounds.
        int w = ev.getWidth();
        int h = ev.getHeight();
        if (w != width || h != height) {
            int d = paintsDropped.incrementAndGet();
            if (d <= 5) {
                LOG.info("BrowserInstance: dropped paint #{} size={}x{} (expected {}x{})",
                        d, w, h, width, height);
            }
            return;
        }
        int n = paintsSeen.incrementAndGet();
        if (n <= 3 || n % 60 == 0) {
            LOG.info("BrowserInstance: accepted paint #{} size={}x{}", n, w, h);
        }
        int bytes = w * h * 4;
        byte[] out = new byte[bytes];

        ByteBuffer src = ev.getRenderedFrame().duplicate();
        src.order(ByteOrder.nativeOrder());
        src.rewind();
        for (int i = 0; i < bytes; i += 4) {
            byte b = src.get();
            byte g = src.get();
            byte r = src.get();
            byte a = src.get();
            out[i]     = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = a;
        }

        synchronized (frameLock) {
            latestFrameRgba = out;
            hasNewFrame.set(true);
        }
    }

    private final AtomicInteger takeCalls = new AtomicInteger(0);

    /**
     * Atomically return the latest painted frame and clear the dirty bit,
     * or {@code null} if no new paint arrived since the last call.
     */
    public byte[] takeFrame() {
        int c = takeCalls.incrementAndGet();
        if (c == 1 || c == 30 || c == 300) {
            LOG.info("BrowserInstance.takeFrame: call #{} hasNewFrame={} paintsSeen={}",
                    c, hasNewFrame.get(), paintsSeen.get());
        }
        if (!hasNewFrame.get()) return null;
        synchronized (frameLock) {
            if (!hasNewFrame.getAndSet(false)) return null;
            byte[] f = latestFrameRgba;
            latestFrameRgba = null;
            return f;
        }
    }

    public int width() { return width; }
    public int height() { return height; }

    public void navigate(String url) {
        if (closed) return;
        browser.loadURL(url);
    }

    /**
     * Decode a 10-byte LE mouse event (see {@code ecm_host_abi::mouse::MouseEvent})
     * and dispatch it to the browser's UI component as an AWT event. The
     * {@code CefBrowserOsr} canvas has mouse listeners that forward dispatched
     * events into CEF. Unknown / invalid events are ignored.
     */
    public void sendMouse(byte[] ev) {
        if (closed || ev == null || ev.length < 10) return;
        ByteBuffer b = ByteBuffer.wrap(ev).order(ByteOrder.LITTLE_ENDIAN);
        int kind = b.get() & 0xFF;
        int x = b.getShort();
        int y = b.getShort();
        int buttons = b.get() & 0xFF;
        int buttonCode = b.get() & 0xFF;
        int scrollDir = b.get(); // signed i8

        int modifiers = awtModifiersFor(buttons);
        long when = System.currentTimeMillis();

        Component target = browser.getUIComponent();
        if (target == null) target = DUMMY_COMPONENT;

        AWTEvent awtEvent = switch (kind) {
            case 1 -> new MouseEvent(target, MouseEvent.MOUSE_MOVED, when, modifiers,
                    x, y, 0, false, MouseEvent.NOBUTTON);
            case 2 -> {
                int awtBtn = awtButtonFor(buttonCode);
                yield awtBtn == 0 ? null : new MouseEvent(target,
                        MouseEvent.MOUSE_PRESSED, when, modifiers,
                        x, y, 1, false, awtBtn);
            }
            case 3 -> {
                int awtBtn = awtButtonFor(buttonCode);
                yield awtBtn == 0 ? null : new MouseEvent(target,
                        MouseEvent.MOUSE_RELEASED, when, modifiers,
                        x, y, 1, false, awtBtn);
            }
            case 4 -> {
                // AWT wheel rotation: positive = toward user (scroll down).
                // Our scroll_dir is +1 for up; invert for AWT semantics.
                int rotation = -scrollDir;
                yield new MouseWheelEvent(target, MouseEvent.MOUSE_WHEEL, when,
                        modifiers, x, y, 0, false,
                        MouseWheelEvent.WHEEL_UNIT_SCROLL,
                        3 /*scrollAmount*/, rotation);
            }
            default -> null;
        };
        if (awtEvent != null) {
            target.dispatchEvent(awtEvent);
        }
    }

    private static int awtButtonFor(int buttonCode) {
        return switch (buttonCode) {
            case 0 -> MouseEvent.BUTTON1;
            case 1 -> MouseEvent.BUTTON3; // right → BUTTON3 (AWT convention)
            case 2 -> MouseEvent.BUTTON2; // middle → BUTTON2
            default -> 0;
        };
    }

    private static int awtModifiersFor(int buttons) {
        int m = 0;
        if ((buttons & 0x01) != 0) m |= InputEvent.BUTTON1_DOWN_MASK;
        if ((buttons & 0x02) != 0) m |= InputEvent.BUTTON3_DOWN_MASK;
        if ((buttons & 0x04) != 0) m |= InputEvent.BUTTON2_DOWN_MASK;
        return m;
    }

    public void close() {
        if (closed) return;
        closed = true;
        try {
            browser.close(true);
        } catch (Throwable t) {
            LOG.debug("browser.close threw", t);
        }
        try {
            client.dispose();
        } catch (Throwable t) {
            LOG.debug("client.dispose threw", t);
        }
        try {
            hiddenFrame.dispose();
        } catch (Throwable t) {
            LOG.debug("hiddenFrame.dispose threw", t);
        }
    }
}
