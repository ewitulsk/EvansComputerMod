package com.example.evanscomputermod.computer.browser;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Global table of open {@link BrowserInstance}s keyed by i64 handle. The
 * handle is the value returned from the {@code browser_open} host function
 * into the WASI child — it's round-tripped back on every subsequent call.
 *
 * <p>Handles are process-global rather than per-ComputerInstance so that the
 * JNI-side CEF code sees a single registry to resolve. Per-instance
 * ownership is tracked separately on {@code ComputerInstance.ownedBrowsers}
 * so child-abort cleanup can close only the browsers that belong to the
 * terminating child, not every browser in the world.
 */
public final class BrowserRegistry {

    private static final Logger LOG = LoggerFactory.getLogger(BrowserRegistry.class);

    private static final AtomicLong NEXT = new AtomicLong(1);
    private static final ConcurrentHashMap<Long, BrowserInstance> BY_HANDLE =
            new ConcurrentHashMap<>();

    private BrowserRegistry() {}

    /**
     * Open a new browser pointed at {@code url} with an off-screen
     * framebuffer of {@code width × height}. Returns the handle, or
     * {@code -1} if CEF is unavailable or the browser failed to construct.
     */
    public static long open(String url, int width, int height) {
        if (!BrowserCef.isAvailable()) {
            LOG.info("browser_open rejected: CEF is not available on this runtime");
            return -1;
        }
        try {
            BrowserInstance inst = BrowserInstance.create(BrowserCef.app(), url, width, height);
            long handle = NEXT.getAndIncrement();
            BY_HANDLE.put(handle, inst);
            return handle;
        } catch (Throwable t) {
            LOG.error("browser_open failed for url={}", url, t);
            return -1;
        }
    }

    public static BrowserInstance get(long handle) {
        return BY_HANDLE.get(handle);
    }

    public static void close(long handle) {
        BrowserInstance inst = BY_HANDLE.remove(handle);
        if (inst != null) {
            try { inst.close(); } catch (Throwable t) {
                LOG.debug("browser close threw", t);
            }
        }
    }
}
