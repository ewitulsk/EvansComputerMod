package com.example.evanscomputermod.computer.browser;

import me.friwi.jcefmaven.CefAppBuilder;
import me.friwi.jcefmaven.MavenCefAppHandlerAdapter;
import org.cef.CefApp;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.File;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Process-wide holder for the one {@link CefApp} instance used by every
 * {@link BrowserInstance}. CEF is fundamentally single-app-per-process — you
 * cannot call {@code CefApp.getInstance()} with different settings twice —
 * so all browsers share this app.
 *
 * <p>Initialization happens in {@code EvansComputerMod.initializeCef} during
 * {@code FMLCommonSetupEvent}. On the first call, jcefmaven downloads the
 * platform-specific CEF native bundle into the configured install dir
 * (~200 MB), unpacks it, and starts CEF's helper processes. Subsequent
 * launches reuse the cached bundle and are fast.
 *
 * <p>CEF uses off-screen rendering (OSR) exclusively — no window is ever
 * created. All pixels come through {@code CefRenderHandler.onPaint} in
 * {@link BrowserInstance}.
 */
public final class BrowserCef {

    private static final Logger LOG = LoggerFactory.getLogger(BrowserCef.class);

    private static final AtomicReference<CefApp> APP = new AtomicReference<>();
    private static volatile boolean disabled;

    private BrowserCef() {}

    /**
     * Initialize CEF. Called once from {@code FMLCommonSetupEvent}. Safe to
     * call multiple times — only the first has effect. Blocks until the
     * native bundle is ready (first-run download may take minutes). A
     * failure is logged and swallowed; {@link #isAvailable()} will return
     * false and subsequent {@code browser_open} calls will fail cleanly.
     */
    public static synchronized void initialize(File installDir) {
        if (disabled || APP.get() != null) return;
        try {
            CefAppBuilder b = new CefAppBuilder();
            b.setInstallDir(installDir);
            b.getCefSettings().windowless_rendering_enabled = true;
            // Critical for our lifecycle: when the last browser closes, CEF
            // enters TERMINATED; the default handler calls System.exit, which
            // would kill the whole Minecraft JVM. We instead just null out
            // the app ref so the next initialize() call (if any) rebuilds.
            b.setAppHandler(new MavenCefAppHandlerAdapter() {
                @Override
                public void stateHasChanged(CefApp.CefAppState state) {
                    LOG.info("CEF state changed: {}", state);
                    if (state == CefApp.CefAppState.TERMINATED) {
                        APP.set(null);
                    }
                }
            });
            CefApp app = b.build();
            APP.set(app);
            LOG.info("jcef initialized: install dir {}", installDir);
        } catch (Throwable t) {
            LOG.error("jcef initialize failed — the browser WASI program will not work", t);
            disabled = true;
        }
    }

    /** Mark CEF as disabled without attempting to load it (dedicated server). */
    public static void markDisabled(String reason) {
        disabled = true;
        LOG.info("jcef disabled: {}", reason);
    }

    public static boolean isAvailable() {
        return APP.get() != null && !disabled;
    }

    /** The process-wide {@link CefApp} or {@code null} if not initialized. */
    public static CefApp app() {
        return APP.get();
    }
}
