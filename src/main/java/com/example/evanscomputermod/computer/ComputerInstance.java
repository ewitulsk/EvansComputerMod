package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IWorldAccess;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.FuncType;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Memory;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.Val.Type;
import io.github.kawamuray.wasmtime.WasmFunctions;
import io.github.kawamuray.wasmtime.WasmValType;
import io.github.kawamuray.wasmtime.WasmtimeException;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import com.example.evanscomputermod.wasm.WasmManager;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;

import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.api.ComputerModuleRegistry;
import com.example.evanscomputermod.wasm.ModuleMethodInvoker;
import com.example.evanscomputermod.wasm.PeripheralManager;
import com.example.evanscomputermod.wasm.PeripheralMethodInvoker;
import com.example.evanscomputermod.wasm.WasmManager;

/**
 * Provides host functions for WASM modules running in a computer context.
 * Allows WASM modules to write to the terminal display and access the file system.
 */
public class ComputerInstance implements AutoCloseable {

    // Directory for storing computer files (in game directory)
    private static final String COMPUTER_DATA_FOLDER = "computer-data";

    private final IComputerHost host;
    private final Engine engine;
    private final Store<Void> store;
    private final Path computerStoragePath;
    private final List<VirtualMount> mounts = new ArrayList<>();
    private com.example.evanscomputermod.computer.wasi.ProcessManager processManager;
    private final com.example.evanscomputermod.computer.wasi.NetIpcBridge netIpcBridge = new com.example.evanscomputermod.computer.wasi.NetIpcBridge();

    // Registry of currently-open MP4 decoders keyed by small integer handle.
    // Used by the player WASI program via bridgeVideo*.
    private final com.example.evanscomputermod.computer.video.VideoDecoderRegistry videoRegistry
            = new com.example.evanscomputermod.computer.video.VideoDecoderRegistry();

    /** Selector for the target display of bridge video/gfx calls. */
    public static final int GFX_TARGET_TERMINAL = 0;
    public static final int GFX_TARGET_SCREEN = 1;

    // --- Bridge gfx op staging (child thread → worker thread rendezvous) ---
    //
    // The WASI child thread cannot touch the kernel's wasmtime store
    // directly — doing so deadlocks wasmtime-java's internal store
    // mutex (see bridgeSleepMs comment below). Instead, the child
    // stages one gfx op (init, frame, set_mode) into the slot below and
    // blocks on `gfxOpLock` until the worker thread drains it. The
    // worker thread calls `drainPendingGfxOps()` from the top of its
    // inner loops (hostSleepMs / checkFramebufferDirty / process_wait)
    // and writes the op directly into kernel WASM memory at GFX_BASE
    // or SCREEN_GFX_BASE, bumps the corresponding dirty counters, and
    // notifyAll's the child. After drain, the existing
    // checkFramebufferDirty path naturally picks up the counter change
    // and pushes the frame to the Java display → client sync. Java
    // never touches the display directly on the bridge path.
    private final Object gfxOpLock = new Object();
    private enum GfxOpKind { INIT, FRAME, SET_MODE }
    private GfxOpKind pendingGfxOpKind;
    private int pendingGfxOpTarget;
    private int pendingGfxOpWidth;
    private int pendingGfxOpHeight;
    private int pendingGfxOpMode;
    private byte[] pendingGfxOpPixels;

    private Instance instance;
    private Memory memory;

    // Flag to indicate the WASM module has crashed and should not be used
    private volatile boolean faulted = false;

    // Flag to signal WASM execution should be interrupted (e.g., Ctrl+T or block break)
    private volatile boolean interrupted = false;
    // Rate-limit terminal syncs to avoid flooding clients with packets
    private long lastTerminalSyncMs = 0;

    // Rate-limit fb_sync host function (enforced at Java level, not bypassable by WASM)
    private long lastFbSyncMs = 0;
    private static final long FB_SYNC_MIN_INTERVAL_MS = 50; // max 20 syncs/sec

    // Last-seen framebuffer dirty counter for auto-redraw polling
    private volatile int lastDirtyCounter = -1;

    // Worker thread for async WASM execution
    private Thread workerThread;
    private volatile boolean shutdownRequested = false;
    private final Object workerWakeSignal = new Object();

    // Flag set by worker thread when framebuffer dirty counter changes,
    // picked up by server tick to sync to clients.
    private volatile boolean needsSync = false;
    private final BlockingQueue<String> inputQueue = new LinkedBlockingQueue<>();

    /** Cached reference to the kernel's terminal_print WASM export (routes output through VTE). */
    private Func terminalPrintFunc = null;
    /** Cached reference to the kernel's handle_sock_ipc WASM export (socket IPC dispatcher). */
    private Func handleSockIpcFunc = null;

    // Interrupt system
    private static final int INTERRUPT_BUFFER_ADDR = 0x11000;
    private final ConcurrentLinkedQueue<InterruptEvent> interruptQueue = new ConcurrentLinkedQueue<>();
    private volatile boolean wasmExecuting = false;
    private int lastInterruptPayloadLen = 0;

    /**
     * An interrupt event queued for delivery to WASM.
     */
    private static class InterruptEvent {
        final int irq;
        final String payload;

        InterruptEvent(int irq, String payload) {
            this.irq = irq;
            this.payload = payload;
        }
    }

    // Counter for wasm-bindgen object reference handles
    private final java.util.concurrent.atomic.AtomicInteger nextObjectHandle = new java.util.concurrent.atomic.AtomicInteger(1);

    // Host functions (need to keep references to prevent GC)
    private final List<Func> hostFunctions = new ArrayList<>();
    private final Map<String, Extern> hostFunctionMap = new HashMap<>();

    // Network: MAC addresses derived from computerId (one per face: down=0, up=1, north=2, south=3, west=4, east=5)
    private byte[][] networkMacs;

    public ComputerInstance(IComputerHost host, byte[][] macs) {
        this.host = host;
        // Create Engine with epoch interruption enabled so Ctrl+T can
        // preempt infinite loops in WASM (including pure CPU-bound code
        // that never calls a host function where checkInterrupted() lives).
        io.github.kawamuray.wasmtime.Config config = new io.github.kawamuray.wasmtime.Config();
        config.epochInterruption(true);
        this.engine = new Engine(config);
        this.store = new Store<Void>(null, this.engine);
        // Arm the epoch deadline — the WASM trap fires when the engine's
        // epoch counter reaches this value.  We'll increment the engine's
        // epoch in interrupt() to trigger the trap.
        this.store.setEpochDeadline(1);

        // Set up computer storage directory
        UUID computerId = host.getComputerId();
        this.computerStoragePath = Path.of(COMPUTER_DATA_FOLDER, computerId.toString());
        try {
            Files.createDirectories(computerStoragePath);
            EvansComputerMod.LOGGER.info("Computer storage path: {}", computerStoragePath.toAbsolutePath());
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to create computer storage directory", e);
        }

        // Set up virtual mount table:
        // 1. User storage (read-write) — checked first, allows shadowing system programs
        mounts.add(new VirtualMount("", computerStoragePath, false));
        // 2. System programs (read-only) — mounted at server-bin/
        Path wasmBinPath = WasmManager.getWasmBinPath();
        if (wasmBinPath != null) {
            mounts.add(new VirtualMount("server-bin", wasmBinPath, true));
        }

        // Initialize WASI process manager with bridge for redstone/peripheral/sleep host calls
        com.example.evanscomputermod.computer.wasi.ChildHostBridge childBridge =
                new com.example.evanscomputermod.computer.wasi.ChildHostBridge(this);
        this.processManager = new com.example.evanscomputermod.computer.wasi.ProcessManager(
                computerStoragePath, netIpcBridge, childBridge);

        // Use provided MAC list (6 built-in + any from attached InterfaceBlocks)
        this.networkMacs = macs;

        // Register all NICs on the network hub if available
        NetworkHub hub = NetworkHub.getInstance();
        if (hub != null) {
            for (byte[] mac : this.networkMacs) {
                hub.registerNic(mac, this::queueInterrupt);
            }
        }

        // Create all host functions
        createHostFunctions();
    }

    /** Buffer in WASM memory where Java writes child stdout for terminal_print to process. */
    private static final int CHILD_OUTPUT_BUFFER = 0x12000;
    private static final int CHILD_OUTPUT_BUFFER_SIZE = 4096;

    /** Framebuffer base address in WASM memory. */
    private static final int FB_BASE = 0x20000;
    /** Graphics framebuffer base address in WASM memory. */
    private static final int GFX_BASE = 0x30000;
    /** Graphics palette offset from GFX_BASE. */
    private static final int GFX_PALETTE_OFF = 0x40;
    /** Graphics pixel data offset from GFX_BASE. */
    private static final int GFX_PIXEL_OFF = 0x400;
    /** In-world Screen cluster graphics framebuffer base in WASM memory. */
    private static final int SCREEN_GFX_BASE = 0x50000;

    // Graphics dirty counter tracking
    private volatile int lastPaletteDirtyCounter = -1;
    private volatile int lastPixelDirtyCounter = -1;

    // Screen (in-world cluster) dirty counter tracking
    private volatile int lastScreenPaletteDirtyCounter = -1;
    private volatile int lastScreenPixelDirtyCounter = -1;
    // Rate-limit screen fb_sync
    private long lastScreenFbSyncMs = 0;

    // Pending screen-header write staged by the server thread via
    // writeScreenHeader() and drained by the worker thread in
    // applyPendingScreenHeader(). See writeScreenHeader()'s javadoc for why.
    private volatile boolean hasPendingScreenHeaderWrite = false;
    private volatile int pendingScreenHeaderWidth = 0;
    private volatile int pendingScreenHeaderHeight = 0;

    /**
     * Check if the framebuffer dirty counter has changed and sync if so.
     * Called on every worker loop iteration to auto-detect display changes.
     */
    private void checkFramebufferDirty() {
        if (memory == null) return;
        // Drain any pending screen-header write staged by the server thread.
        // This is the sole place the worker thread services writeScreenHeader
        // requests — keeping the Wasmtime store single-threaded and avoiding
        // the cross-thread stall documented on writeScreenHeader.
        applyPendingScreenHeader();
        // Drain any pending gfx op staged by a WASI child thread (e.g. the
        // player pushing a decoded video frame). The op writes directly
        // into kernel WASM memory and bumps the dirty counters, so the
        // rest of this method's read path naturally sees the change and
        // propagates it to the Java display.
        drainPendingGfxOps();
        try {
            ByteBuffer buf = memory.buffer(store);
            if (buf == null || buf.capacity() < FB_BASE + 16) return;

            boolean changed = false;

            // Check text framebuffer dirty counter
            int dirty = buf.getInt(FB_BASE + 0x0C);
            if (dirty != lastDirtyCounter) {
                lastDirtyCounter = dirty;
                changed = true;
            }

            // Check graphics framebuffer dirty counters
            if (buf.capacity() >= GFX_BASE + 16) {
                int palDirty = buf.getInt(GFX_BASE + 0x08);
                int pixDirty = buf.getInt(GFX_BASE + 0x0C);
                if (palDirty != lastPaletteDirtyCounter || pixDirty != lastPixelDirtyCounter) {
                    lastPaletteDirtyCounter = palDirty;
                    lastPixelDirtyCounter = pixDirty;
                    changed = true;
                }
            }

            // Check screen cluster (in-world) gfx framebuffer dirty counters
            if (buf.capacity() >= SCREEN_GFX_BASE + 16) {
                int sMagic = (buf.get(SCREEN_GFX_BASE) & 0xFF) | ((buf.get(SCREEN_GFX_BASE + 1) & 0xFF) << 8);
                if (sMagic == 0xFB02) {
                    int sPalDirty = buf.getInt(SCREEN_GFX_BASE + 0x08);
                    int sPixDirty = buf.getInt(SCREEN_GFX_BASE + 0x0C);
                    if (sPalDirty != lastScreenPaletteDirtyCounter
                            || sPixDirty != lastScreenPixelDirtyCounter) {
                        lastScreenPaletteDirtyCounter = sPalDirty;
                        lastScreenPixelDirtyCounter = sPixDirty;
                        changed = true;
                    }
                }
            }

            if (changed) {
                readFramebufferFromWasm();
                readScreenFramebufferFromWasm();
                // Flag only — the server tick's tickSync() picks this up
                // and schedules exactly one sync task per tick. Directly
                // calling syncTerminalToClients() here would flood the
                // server task queue during tight animation loops (every
                // hostSleepMs chunk would queue another runnable).
                needsSync = true;
            }
        } catch (Exception e) {
            // Silently ignore — non-critical
        }
    }

    /**
     * Called from the server tick (on the server thread) to sync the display
     * when the worker thread has detected a framebuffer change.
     * Only calls syncToClients — the framebuffer was already read on the worker thread.
     */
    public void tickSync() {
        if (needsSync) {
            needsSync = false;
            host.syncToClients();
        }
    }

    /**
     * Read the framebuffer from WASM memory and update the host's display.
     */
    private void readFramebufferFromWasm() {
        if (memory == null) return;
        IFramebufferDisplay display = host.getFramebufferDisplay();
        if (display == null) return;

        try {
            ByteBuffer buf = memory.buffer(store);
            if (buf == null || buf.capacity() < FB_BASE + 64) return;

            // Read text framebuffer
            int width = (buf.get(FB_BASE + 2) & 0xFF) | ((buf.get(FB_BASE + 3) & 0xFF) << 8);
            int height = (buf.get(FB_BASE + 4) & 0xFF) | ((buf.get(FB_BASE + 5) & 0xFF) << 8);
            int totalSize = 64 + width * height * 4;

            if (buf.capacity() < FB_BASE + totalSize) return;

            byte[] fbData = new byte[totalSize];
            buf.position(FB_BASE);
            buf.get(fbData, 0, totalSize);

            display.setFromBytes(fbData);

            // Read graphics framebuffer if present.
            if (display instanceof TerminalDisplay td && buf.capacity() >= GFX_BASE + 64) {
                int gfxMagic = (buf.get(GFX_BASE) & 0xFF) | ((buf.get(GFX_BASE + 1) & 0xFF) << 8);
                if (gfxMagic == 0xFB02) {
                    int mode = buf.get(GFX_BASE + 2) & 0xFF;
                    if (mode > 0) {
                        int gfxW = (buf.get(GFX_BASE + 4) & 0xFF) | ((buf.get(GFX_BASE + 5) & 0xFF) << 8);
                        int gfxH = (buf.get(GFX_BASE + 6) & 0xFF) | ((buf.get(GFX_BASE + 7) & 0xFF) << 8);
                        int gfxTotalSize = GFX_PIXEL_OFF + gfxW * gfxH;

                        if (buf.capacity() >= GFX_BASE + gfxTotalSize) {
                            byte[] gfxData = new byte[gfxTotalSize];
                            buf.position(GFX_BASE);
                            buf.get(gfxData, 0, gfxTotalSize);
                            td.setGfxFromBytes(gfxData);
                        }
                    } else {
                        byte[] resetGfx = new byte[64]; // must be >= GFX_PALETTE_OFF (0x40)
                        resetGfx[0] = (byte) 0x02;
                        resetGfx[1] = (byte) 0xFB; // magic
                        // mode=0, everything else zeros
                        td.setGfxFromBytes(resetGfx);
                    }
                }
            }
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading framebuffer from WASM memory", e);
        }
    }

    /**
     * Read the screen cluster's graphics framebuffer from WASM memory
     * and apply it to the TerminalBlockEntity's screen display.
     */
    private void readScreenFramebufferFromWasm() {
        if (memory == null) return;
        if (!(host instanceof TerminalBlockEntity tbe)) return;
        TerminalDisplay sd = tbe.getScreenDisplay();
        if (sd == null) return;

        try {
            ByteBuffer buf = memory.buffer(store);
            if (buf == null || buf.capacity() < SCREEN_GFX_BASE + 64) return;

            int magic = (buf.get(SCREEN_GFX_BASE) & 0xFF) | ((buf.get(SCREEN_GFX_BASE + 1) & 0xFF) << 8);
            if (magic != 0xFB02) return;

            int mode = buf.get(SCREEN_GFX_BASE + 2) & 0xFF;
            int gfxW = (buf.get(SCREEN_GFX_BASE + 4) & 0xFF) | ((buf.get(SCREEN_GFX_BASE + 5) & 0xFF) << 8);
            int gfxH = (buf.get(SCREEN_GFX_BASE + 6) & 0xFF) | ((buf.get(SCREEN_GFX_BASE + 7) & 0xFF) << 8);
            if (gfxW == 0 || gfxH == 0) return;
            int total = GFX_PIXEL_OFF + gfxW * gfxH;
            if (buf.capacity() < SCREEN_GFX_BASE + total) return;

            byte[] gfxData = new byte[total];
            buf.position(SCREEN_GFX_BASE);
            buf.get(gfxData, 0, total);
            sd.setGfxFromBytes(gfxData);
            if (mode == 0) sd.setDisplayMode(1); // screen is always graphics-mode
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error reading screen framebuffer from WASM", e);
        }
    }

    /**
     * Write the screen cluster graphics header into WASM memory so the Rust OS
     * can discover its current resolution. Called by TerminalBlockEntity when
     * the cluster is (re)formed. Passing width=height=0 marks the screen as
     * detached (mode=0, dimensions cleared).
     *
     * <p><strong>Thread-safety:</strong> this method is called from the server
     * thread (inside {@code rescanScreenCluster}, which itself is invoked from
     * Minecraft's neighbor-update cascade on any nearby block change — e.g.
     * grass spreading via {@code SpreadingSnowyBlock.randomTick}). Touching the
     * Wasmtime store ({@code memory.buffer(store)}) from the server thread
     * while the worker thread is mid-WASM-call deadlocks on the bindings'
     * internal store mutex — the server thread blocks inside the native
     * {@code nativeBuffer} call for the full duration of the WASM call,
     * producing the gfxtest-screen freeze. Instead, stash the desired header
     * into volatile fields; the worker thread applies it in
     * {@link #applyPendingScreenHeader}, called from {@link #checkFramebufferDirty}
     * (every worker loop iteration + every hostSleepMs chunk).
     */
    public void writeScreenHeader(int gfxWidth, int gfxHeight) {
        pendingScreenHeaderWidth = gfxWidth;
        pendingScreenHeaderHeight = gfxHeight;
        hasPendingScreenHeaderWrite = true;
    }

    /**
     * Drain the pending screen-header request set by the server thread. Must
     * only be called on the worker thread — touches the Wasmtime store.
     */
    private void applyPendingScreenHeader() {
        if (!hasPendingScreenHeaderWrite) return;
        int gfxWidth = pendingScreenHeaderWidth;
        int gfxHeight = pendingScreenHeaderHeight;
        hasPendingScreenHeaderWrite = false;
        if (memory == null) return;
        try {
            ByteBuffer buf = memory.buffer(store);
            if (buf == null || buf.capacity() < SCREEN_GFX_BASE + 64) return;

            // magic
            buf.put(SCREEN_GFX_BASE, (byte) 0x02);
            buf.put(SCREEN_GFX_BASE + 1, (byte) 0xFB);
            // mode: 1 if attached, 0 if detached
            buf.put(SCREEN_GFX_BASE + 2, (byte) (gfxWidth > 0 && gfxHeight > 0 ? 1 : 0));
            buf.put(SCREEN_GFX_BASE + 3, (byte) 0);
            // width / height
            buf.put(SCREEN_GFX_BASE + 4, (byte) (gfxWidth & 0xFF));
            buf.put(SCREEN_GFX_BASE + 5, (byte) ((gfxWidth >> 8) & 0xFF));
            buf.put(SCREEN_GFX_BASE + 6, (byte) (gfxHeight & 0xFF));
            buf.put(SCREEN_GFX_BASE + 7, (byte) ((gfxHeight >> 8) & 0xFF));
            // leave dirty counters alone; Rust OS writes them
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error writing screen header", e);
        }
    }

    /**
     * Write raw bytes directly into the WASM framebuffer at FB_BASE.
     * Handles printable ASCII, \n, \r, \t. Used by process_wait to drain
     * child process stdout into the kernel's display.
     */
    private void drainBytesToFramebuffer(byte[] data, int length) {
        if (memory == null) return;
        try {
            ByteBuffer buf = memory.buffer(store);
            if (buf == null || buf.capacity() < FB_BASE + 64) return;

            int width = (buf.get(FB_BASE + 2) & 0xFF) | ((buf.get(FB_BASE + 3) & 0xFF) << 8);
            int height = (buf.get(FB_BASE + 4) & 0xFF) | ((buf.get(FB_BASE + 5) & 0xFF) << 8);
            int cx = (buf.get(FB_BASE + 6) & 0xFF) | ((buf.get(FB_BASE + 7) & 0xFF) << 8);
            int cy = (buf.get(FB_BASE + 8) & 0xFF) | ((buf.get(FB_BASE + 9) & 0xFF) << 8);

            if (width == 0 || height == 0) return;
            int cellBase = FB_BASE + 64;
            int rowBytes = width * 4;
            byte attr = 0x0A; // DEFAULT_ATTR (bright green on black)

            for (int i = 0; i < length; i++) {
                byte b = data[i];

                if (b == '\n') {
                    cx = 0;
                    cy++;
                    if (cy >= height) {
                        // Scroll up
                        for (int row = 1; row < height; row++) {
                            int src = cellBase + row * rowBytes;
                            int dst = cellBase + (row - 1) * rowBytes;
                            for (int j = 0; j < rowBytes; j++) {
                                buf.put(dst + j, buf.get(src + j));
                            }
                        }
                        // Clear last row
                        int lastRow = cellBase + (height - 1) * rowBytes;
                        for (int col = 0; col < width; col++) {
                            int off = lastRow + col * 4;
                            buf.put(off, (byte) ' ');
                            buf.put(off + 1, attr);
                            buf.put(off + 2, (byte) 0);
                            buf.put(off + 3, (byte) 0);
                        }
                        cy = height - 1;
                    }
                } else if (b == '\r') {
                    cx = 0;
                } else if (b == '\t') {
                    cx = ((cx / 8) + 1) * 8;
                    if (cx >= width) cx = width - 1;
                } else if (b >= 0x20 && b < 0x7F) {
                    if (cx >= width) {
                        cx = 0;
                        cy++;
                        if (cy >= height) {
                            // Scroll
                            for (int row = 1; row < height; row++) {
                                int src = cellBase + row * rowBytes;
                                int dst = cellBase + (row - 1) * rowBytes;
                                for (int j = 0; j < rowBytes; j++) {
                                    buf.put(dst + j, buf.get(src + j));
                                }
                            }
                            int lastRow = cellBase + (height - 1) * rowBytes;
                            for (int col = 0; col < width; col++) {
                                int off = lastRow + col * 4;
                                buf.put(off, (byte) ' ');
                                buf.put(off + 1, attr);
                                buf.put(off + 2, (byte) 0);
                                buf.put(off + 3, (byte) 0);
                            }
                            cy = height - 1;
                        }
                    }
                    int off = cellBase + (cy * width + cx) * 4;
                    buf.put(off, b);
                    buf.put(off + 1, attr);
                    buf.put(off + 2, (byte) 0);
                    buf.put(off + 3, (byte) 0);
                    cx++;
                }
            }

            // Update cursor position in header
            buf.put(FB_BASE + 6, (byte) (cx & 0xFF));
            buf.put(FB_BASE + 7, (byte) ((cx >> 8) & 0xFF));
            buf.put(FB_BASE + 8, (byte) (cy & 0xFF));
            buf.put(FB_BASE + 9, (byte) ((cy >> 8) & 0xFF));

            // Increment dirty counter
            int dirty = buf.getInt(FB_BASE + 0x0C);
            buf.putInt(FB_BASE + 0x0C, dirty + 1);

            // Sync display
            readFramebufferFromWasm();
            host.syncToClients();
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error draining to framebuffer", e);
        }
    }

    /**
     * Get the kernel's terminal_print WASM export (cached after first lookup).
     * This export routes bytes through the Rust VTE for ANSI escape sequence processing.
     */
    private Func getTerminalPrintFunc() {
        if (terminalPrintFunc == null && instance != null) {
            instance.getFunc(store, "terminal_print").ifPresent(f -> terminalPrintFunc = f);
        }
        return terminalPrintFunc;
    }

    private Func getHandleSockIpcFunc() {
        if (handleSockIpcFunc == null && instance != null) {
            instance.getFunc(store, "handle_sock_ipc").ifPresent(f -> handleSockIpcFunc = f);
        }
        return handleSockIpcFunc;
    }

    /**
     * Drain child output bytes through the kernel's VTE by calling the terminal_print
     * WASM export. This processes ANSI escape sequences (cursor movement, colors,
     * clear screen, etc.) so full-screen programs like 'edit' render correctly.
     *
     * Falls back to direct framebuffer writes if the terminal_print export is unavailable.
     */
    private void drainBytesViaVte(byte[] data, int length) {
        drainBytesViaVteNoSync(data, length);
        readFramebufferFromWasm();
        host.syncToClients();
    }

    /**
     * Write child output through the kernel's VTE without syncing to clients.
     * Used in process_wait to batch all output before a single sync, avoiding
     * the delta protocol's ack requirement from dropping intermediate updates.
     */
    private void drainBytesViaVteNoSync(byte[] data, int length) {
        Func tpFunc = getTerminalPrintFunc();
        if (tpFunc == null || memory == null) {
            drainBytesToFramebuffer(data, length);
            return;
        }

        try {
            ByteBuffer buf = memory.buffer(store);
            int remaining = length;
            int offset = 0;
            while (remaining > 0) {
                int chunk = Math.min(remaining, CHILD_OUTPUT_BUFFER_SIZE);
                buf.position(CHILD_OUTPUT_BUFFER);
                buf.put(data, offset, chunk);
                tpFunc.call(store, Val.fromI32(CHILD_OUTPUT_BUFFER), Val.fromI32(chunk));
                offset += chunk;
                remaining -= chunk;
            }
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error draining via VTE, falling back to direct writes", e);
            drainBytesToFramebuffer(data, length);
        }
    }

    /**
     * Starts the worker thread that processes WASM input asynchronously.
     * This allows the main server thread to remain responsive even if WASM enters an infinite loop.
     */
    public void startWorkerThread() {
        if (workerThread != null && workerThread.isAlive()) {
            return;  // Already running
        }

        shutdownRequested = false;
        workerThread = new Thread(this::workerLoop, "WASM-Worker-" + host.getComputerId().toString().substring(0, 8));
        workerThread.setDaemon(true);
        workerThread.start();
        EvansComputerMod.LOGGER.info("Started WASM worker thread: {}", workerThread.getName());

    }

    /**
     * Worker thread main loop - processes input and interrupts from the queues.
     */
    private void workerLoop() {
        EvansComputerMod.LOGGER.debug("WASM worker thread started");

        while (!shutdownRequested && !Thread.currentThread().isInterrupted()) {
            try {
                // Drain pending interrupts before processing input
                drainAndDeliverInterrupts();

                // Event-driven wait: wake quickly on either input or queued IRQs.
                // Keep a bounded timeout so shutdown checks remain prompt.
                String input = inputQueue.poll();
                if (input == null && interruptQueue.isEmpty() && !shutdownRequested) {
                    synchronized (workerWakeSignal) {
                        if (inputQueue.peek() == null && interruptQueue.isEmpty() && !shutdownRequested) {
                            workerWakeSignal.wait(100);
                        }
                    }
                    input = inputQueue.poll();
                }

                if (input != null) {
                    processInputOnWorker(input);
                    // Drain interrupts that arrived during input processing
                    drainAndDeliverInterrupts();
                }

                // Auto-detect framebuffer changes (boot output, async writes, etc.)
                checkFramebufferDirty();
            } catch (InterruptedException e) {
                // Thread was interrupted — this can happen from Ctrl+T's workerThread.interrupt().
                // Clear the flag and continue the loop (don't exit) so the shell remains responsive.
                // Only exit if shutdown was explicitly requested.
                Thread.interrupted(); // clear the flag
                if (shutdownRequested) {
                    break;
                }
            } catch (Throwable e) {
                // Log any unexpected errors but keep the worker running
                EvansComputerMod.LOGGER.error("Error in WASM worker thread", e);
            }
        }

        EvansComputerMod.LOGGER.debug("WASM worker thread exiting");
    }

    /**
     * Processes input on the worker thread - calls the WASM input handler.
     */
    private void processInputOnWorker(String input) {
        if (instance == null || faulted) {
            return;
        }

        Optional<Func> inputHandler = instance.getFunc(store, "on_input");
        if (inputHandler.isEmpty()) {
            inputHandler = instance.getFunc(store, "handle_input");
        }

        if (inputHandler.isPresent() && memory != null) {
            wasmExecuting = true;
            try {
                // Write the input string to WASM memory
                byte[] bytes = input.getBytes(StandardCharsets.UTF_8);
                ByteBuffer buffer = memory.buffer(store);

                // Use a fixed input buffer location (at address 0x10000)
                int inputBufferAddr = 0x10000;
                buffer.position(inputBufferAddr);
                buffer.put(bytes);

                // Call the input handler with pointer and length
                inputHandler.get().call(store, Val.fromI32(inputBufferAddr), Val.fromI32(bytes.length));

                // After successful execution, sync terminal state to clients
                syncTerminalToClients();

            } catch (WasmInterruptedException e) {
                // Interrupted execution - clear flag so OS can receive Ctrl+T and reset to shell
                EvansComputerMod.LOGGER.info("WASM execution was interrupted");
                interrupted = false;
                // Re-arm epoch deadline for next interrupt
                store.setEpochDeadline(1);
                // Clear thread's interrupted flag so worker loop continues
                Thread.interrupted();
                syncTerminalToClients();
            } catch (Throwable e) {
                // Check if this was caused by an interrupt (including epoch deadline trap)
                String msg = e.getMessage() != null ? e.getMessage().toLowerCase() : "";
                if (interrupted || msg.contains("epoch") || msg.contains("interrupt")
                        || msg.contains("trap: interrupt")) {
                    // Clear flag so OS can receive Ctrl+T and reset to shell
                    EvansComputerMod.LOGGER.info("WASM execution was interrupted (via exception: {})", e.getMessage());
                    interrupted = false;
                    // Re-arm epoch deadline for next interrupt
                    store.setEpochDeadline(1);
                    // Clear thread's interrupted flag so worker loop continues
                    Thread.interrupted();
                    syncTerminalToClients();
                    return;
                }

                // Mark as faulted to prevent further use of corrupted WASM state
                faulted = true;
                EvansComputerMod.LOGGER.error("Error in WASM execution", e);
                // Error message now logged only (framebuffer is WASM-side)
                // (faulted message logged above)
                syncTerminalToClients();
            } finally {
                wasmExecuting = false;
            }
        }
    }

    /**
     * Returns whether WASM is currently executing a call (for interrupt queueing decisions).
     */
    public boolean isWasmExecuting() {
        return wasmExecuting;
    }

    /**
     * Queues an interrupt event for delivery to WASM on the worker thread.
     * Thread-safe - can be called from any thread.
     *
     * @param irq     The interrupt number (1=KEYBOARD, 2=REDSTONE, 15=TERMINATE)
     * @param payload JSON-encoded data for the interrupt handler
     */
    public void queueInterrupt(int irq, String payload) {
        interruptQueue.offer(new InterruptEvent(irq, payload));
        wakeWorker();
    }

    /**
     * Wakes the worker loop when new input or interrupts arrive.
     */
    private void wakeWorker() {
        synchronized (workerWakeSignal) {
            workerWakeSignal.notifyAll();
        }
    }

    /**
     * Drains the interrupt queue and delivers each event to WASM via on_interrupt().
     * Must only be called from the worker thread.
     */
    private void drainAndDeliverInterrupts() {
        if (instance == null || faulted || memory == null) return;

        boolean delivered = false;
        InterruptEvent evt;
        while ((evt = interruptQueue.poll()) != null) {
            deliverInterrupt(evt);
            delivered = true;
        }
        if (delivered) {
            syncTerminalToClients();
        }
    }

    /**
     * Delivers a single interrupt event to WASM by calling the on_interrupt export.
     * Writes the payload to WASM memory at INTERRUPT_BUFFER_ADDR and calls on_interrupt(irq, ptr, len).
     */
    private void deliverInterrupt(InterruptEvent evt) {
        Optional<Func> handler = instance.getFunc(store, "on_interrupt");
        if (handler.isEmpty()) return;

        try {
            byte[] payloadBytes = evt.payload.getBytes(StandardCharsets.UTF_8);
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(INTERRUPT_BUFFER_ADDR);
            buffer.put(payloadBytes);

            handler.get().call(store,
                    Val.fromI32(evt.irq),
                    Val.fromI32(INTERRUPT_BUFFER_ADDR),
                    Val.fromI32(payloadBytes.length));
        } catch (WasmInterruptedException e) {
            EvansComputerMod.LOGGER.info("Interrupt delivery was interrupted");
            interrupted = false;
            Thread.interrupted();
        } catch (Throwable e) {
            if (interrupted) {
                interrupted = false;
                Thread.interrupted();
            } else {
                EvansComputerMod.LOGGER.error("Error delivering interrupt IRQ={}", evt.irq, e);
            }
        }
    }

    /**
     * Syncs the terminal buffer to connected clients.
     * Called from the worker thread after WASM modifies the terminal.
     */
    private void syncTerminalToClients() {
        readFramebufferFromWasm();
        host.syncToClients();
    }

    /**
     * Creates all host functions and stores them in a map for lookup by name.
     */
    private void createHostFunctions() {
        // fb_sync() -> void
        // Hint from the WASM OS to read the framebuffer and sync to clients now.
        // Rate-limited at the Java level (max 20/sec) to prevent WASM from flooding the server.
        // The actual client-facing delta sync is scheduled by tickSync() on the
        // server tick — this path only pumps the WASM memory into `display` and
        // flags that a sync is due.
        Func fbSyncFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{}),
                (caller, params, results) -> {
                    checkInterrupted();
                    long now = System.currentTimeMillis();
                    if (now - lastFbSyncMs >= FB_SYNC_MIN_INTERVAL_MS) {
                        lastFbSyncMs = now;
                        readFramebufferFromWasm();
                        needsSync = true;
                    }
                });
        hostFunctions.add(fbSyncFunc);
        hostFunctionMap.put("fb_sync", Extern.fromFunc(fbSyncFunc));

        // === Screen (in-world cluster) host functions ===

        // screen_is_attached() -> i32 (1 if a screen cluster is attached, else 0)
        Func screenIsAttachedFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    boolean attached = (host instanceof TerminalBlockEntity tbe) && tbe.hasScreenCluster();
                    results[0] = Val.fromI32(attached ? 1 : 0);
                });
        hostFunctions.add(screenIsAttachedFunc);
        hostFunctionMap.put("screen_is_attached", Extern.fromFunc(screenIsAttachedFunc));

        // screen_get_gfx_width() -> i32
        Func screenGetWidthFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int w = 0;
                    if (host instanceof TerminalBlockEntity tbe) {
                        TerminalBlockEntity.ScreenClusterInfo info = tbe.getScreenClusterInfo();
                        if (info != null) w = info.gfxWidth();
                    }
                    results[0] = Val.fromI32(w);
                });
        hostFunctions.add(screenGetWidthFunc);
        hostFunctionMap.put("screen_get_gfx_width", Extern.fromFunc(screenGetWidthFunc));

        // screen_get_gfx_height() -> i32
        Func screenGetHeightFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int h = 0;
                    if (host instanceof TerminalBlockEntity tbe) {
                        TerminalBlockEntity.ScreenClusterInfo info = tbe.getScreenClusterInfo();
                        if (info != null) h = info.gfxHeight();
                    }
                    results[0] = Val.fromI32(h);
                });
        hostFunctions.add(screenGetHeightFunc);
        hostFunctionMap.put("screen_get_gfx_height", Extern.fromFunc(screenGetHeightFunc));

        // screen_fb_sync() -> void
        // Reads the screen framebuffer from WASM memory and flags the host
        // for a client sync. Rate-limited at 20/sec like fb_sync. The
        // server tick's tickSync() actually schedules the delta sync task —
        // calling host.syncToClients() directly from here would flood the
        // server task queue during tight animation loops (e.g. the palette
        // animation in gfxtest screen).
        Func screenFbSyncFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{}),
                (caller, params, results) -> {
                    checkInterrupted();
                    long now = System.currentTimeMillis();
                    if (now - lastScreenFbSyncMs >= FB_SYNC_MIN_INTERVAL_MS) {
                        lastScreenFbSyncMs = now;
                        readScreenFramebufferFromWasm();
                        needsSync = true;
                    }
                });
        hostFunctions.add(screenFbSyncFunc);
        hostFunctionMap.put("screen_fb_sync", Extern.fromFunc(screenFbSyncFunc));

        // screen_set_power(on: i32) -> void
        // Turn the attached screen cluster on (nonzero) or off (0). When
        // off, the BER stops rendering the quad and every member block's
        // ACTIVE blockstate becomes false so its face reverts to the
        // "no signal" inactive texture. The cluster is not torn down.
        Func screenSetPowerFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int on = params[0].i32();
                    if (host instanceof TerminalBlockEntity tbe) {
                        tbe.setScreenPower(on != 0);
                    }
                });
        hostFunctions.add(screenSetPowerFunc);
        hostFunctionMap.put("screen_set_power", Extern.fromFunc(screenSetPowerFunc));

        // === File System Host Functions ===

        // file_write(path_ptr, path_len, data_ptr, data_len) -> bytes_written or -1
        Func fileWriteFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    int dataPtr = params[2].i32();
                    int dataLen = params[3].i32();
                    results[0] = Val.fromI32(hostFileWrite(pathPtr, pathLen, dataPtr, dataLen));
                });
        hostFunctions.add(fileWriteFunc);
        hostFunctionMap.put("file_write", Extern.fromFunc(fileWriteFunc));

        // file_read(path_ptr, path_len, buf_ptr, buf_len) -> bytes_read or -1
        Func fileReadFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostFileRead(pathPtr, pathLen, bufPtr, bufLen));
                });
        hostFunctions.add(fileReadFunc);
        hostFunctionMap.put("file_read", Extern.fromFunc(fileReadFunc));

        // file_size(path_ptr, path_len) -> file size or -1
        Func fileSizeFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileSize(pathPtr, pathLen));
                });
        hostFunctions.add(fileSizeFunc);
        hostFunctionMap.put("file_size", Extern.fromFunc(fileSizeFunc));

        // file_exists(path_ptr, path_len) -> 1 if exists, 0 if not
        Func fileExistsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileExists(pathPtr, pathLen));
                });
        hostFunctions.add(fileExistsFunc);
        hostFunctionMap.put("file_exists", Extern.fromFunc(fileExistsFunc));

        // file_delete(path_ptr, path_len) -> 1 on success, 0 on failure
        Func fileDeleteFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileDelete(pathPtr, pathLen));
                });
        hostFunctions.add(fileDeleteFunc);
        hostFunctionMap.put("file_delete", Extern.fromFunc(fileDeleteFunc));

        // file_list(buf_ptr, buf_len) -> bytes written (newline-separated filenames)
        Func fileListFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileList(bufPtr, bufLen));
                });
        hostFunctions.add(fileListFunc);
        hostFunctionMap.put("file_list", Extern.fromFunc(fileListFunc));

        // file_mkdir(path_ptr, path_len) -> 0 on success, -1 on error
        Func fileMkdirFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileMkdir(pathPtr, pathLen));
                });
        hostFunctions.add(fileMkdirFunc);
        hostFunctionMap.put("file_mkdir", Extern.fromFunc(fileMkdirFunc));

        // file_is_dir(path_ptr, path_len) -> 1 if directory, 0 if not
        Func fileIsDirFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileIsDir(pathPtr, pathLen));
                });
        hostFunctions.add(fileIsDirFunc);
        hostFunctionMap.put("file_is_dir", Extern.fromFunc(fileIsDirFunc));

        // file_list_dir(path_ptr, path_len, buf_ptr, buf_len) -> bytes written, or -1 on error
        Func fileListDirFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostFileListDir(pathPtr, pathLen, bufPtr, bufLen));
                });
        hostFunctions.add(fileListDirFunc);
        hostFunctionMap.put("file_list_dir", Extern.fromFunc(fileListDirFunc));

        // === Redstone Output ===

        // redstone_set_output(side: i32, power: i32) -> i32
        Func redstoneSetOutputFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int side = params[0].i32();
                    int power = params[1].i32();
                    results[0] = Val.fromI32(hostRedstoneSetOutput(side, power));
                });
        hostFunctions.add(redstoneSetOutputFunc);
        hostFunctionMap.put("redstone_set_output", Extern.fromFunc(redstoneSetOutputFunc));

        // === Sleep Function ===

        // sleep_ms(milliseconds: i32) -> void
        Func sleepMsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int ms = params[0].i32();
                    hostSleepMs(ms);
                });
        hostFunctions.add(sleepMsFunc);
        hostFunctionMap.put("sleep_ms", Extern.fromFunc(sleepMsFunc));

        // === Line Input Function ===

        // terminal_read_line(prompt_ptr: i32, prompt_len: i32, buf_ptr: i32, buf_len: i32) -> i32
        Func readLineFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int promptPtr = params[0].i32();
                    int promptLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostReadLine(promptPtr, promptLen, bufPtr, bufLen));
                });
        hostFunctions.add(readLineFunc);
        hostFunctionMap.put("terminal_read_line", Extern.fromFunc(readLineFunc));

        // === Custom getrandom for getrandom 0.3 ===
        // __getrandom_v03_custom(ptr: i32, len: i32) -> i32
        Func getrandomFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    results[0] = Val.fromI32(hostGetrandom(ptr, len));
                });
        hostFunctions.add(getrandomFunc);
        hostFunctionMap.put("__getrandom_v03_custom", Extern.fromFunc(getrandomFunc));

        // === CC:Tweaked Peripheral Integration ===
        // peripheral_list(buf_ptr: i32, buf_len: i32) -> i32 (bytes written, or -1 on error)
        Func peripheralListFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostPeripheralList(bufPtr, bufLen));
                });
        hostFunctions.add(peripheralListFunc);
        hostFunctionMap.put("peripheral_list", Extern.fromFunc(peripheralListFunc));

        // peripheral_get_methods(name_ptr, name_len, buf_ptr, buf_len) -> i32 (bytes written, or -1 on error)
        Func peripheralGetMethodsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int namePtr = params[0].i32();
                    int nameLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostPeripheralGetMethods(namePtr, nameLen, bufPtr, bufLen));
                });
        hostFunctions.add(peripheralGetMethodsFunc);
        hostFunctionMap.put("peripheral_get_methods", Extern.fromFunc(peripheralGetMethodsFunc));

        // peripheral_call(name_ptr, name_len, method_ptr, method_len, args_ptr, args_len, result_ptr, result_len) -> i32
        Func peripheralCallFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int namePtr = params[0].i32();
                    int nameLen = params[1].i32();
                    int methodPtr = params[2].i32();
                    int methodLen = params[3].i32();
                    int argsPtr = params[4].i32();
                    int argsLen = params[5].i32();
                    int resultPtr = params[6].i32();
                    int resultLen = params[7].i32();
                    results[0] = Val.fromI32(hostPeripheralCall(namePtr, nameLen, methodPtr, methodLen, argsPtr, argsLen, resultPtr, resultLen));
                });
        hostFunctions.add(peripheralCallFunc);
        hostFunctionMap.put("peripheral_call", Extern.fromFunc(peripheralCallFunc));

        // === Redstone Input ===

        // redstone_get_input(side: i32) -> i32
        Func redstoneGetInputFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int side = params[0].i32();
                    results[0] = Val.fromI32(hostRedstoneGetInput(side));
                });
        hostFunctions.add(redstoneGetInputFunc);
        hostFunctionMap.put("redstone_get_input", Extern.fromFunc(redstoneGetInputFunc));

        // redstone_get_all_input(buf_ptr: i32) -> i32 (writes 6 i32 values, returns 0 on success)
        Func redstoneGetAllInputFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    results[0] = Val.fromI32(hostRedstoneGetAllInput(bufPtr));
                });
        hostFunctions.add(redstoneGetAllInputFunc);
        hostFunctionMap.put("redstone_get_all_input", Extern.fromFunc(redstoneGetAllInputFunc));

        // === Interrupt Support ===

        // interrupt_poll(buf_ptr: i32, buf_len: i32) -> i32 (returns IRQ number, or -1 if none)
        Func interruptPollFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostInterruptPoll(bufPtr, bufLen));
                });
        hostFunctions.add(interruptPollFunc);
        hostFunctionMap.put("interrupt_poll", Extern.fromFunc(interruptPollFunc));

        // interrupt_poll_len() -> i32 (returns payload length of last polled interrupt)
        Func interruptPollLenFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(lastInterruptPayloadLen);
                });
        hostFunctions.add(interruptPollLenFunc);
        hostFunctionMap.put("interrupt_poll_len", Extern.fromFunc(interruptPollLenFunc));

        // === Visual Editor ===
        // open_visual_editor() -> void
        Func openVisualEditorFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{}),
                (caller, params, results) -> {
                    checkInterrupted();
                    if (host.getVisualProgramming() != null) {
                        host.getVisualProgramming().openVisualEditor();
                    }
                });
        hostFunctions.add(openVisualEditorFunc);
        hostFunctionMap.put("open_visual_editor", Extern.fromFunc(openVisualEditorFunc));

        // === Module Call Bridge (Annotation-Driven Auto-Registration) ===

        // module_call(module_ptr, module_len, method_ptr, method_len, args_ptr, args_len, result_ptr, result_len) -> i32
        Func moduleCallFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int modulePtr = params[0].i32();
                    int moduleLen = params[1].i32();
                    int methodPtr = params[2].i32();
                    int methodLen = params[3].i32();
                    int argsPtr = params[4].i32();
                    int argsLen = params[5].i32();
                    int resultPtr = params[6].i32();
                    int resultLen = params[7].i32();
                    results[0] = Val.fromI32(hostModuleCall(modulePtr, moduleLen, methodPtr, methodLen, argsPtr, argsLen, resultPtr, resultLen));
                });
        hostFunctions.add(moduleCallFunc);
        hostFunctionMap.put("module_call", Extern.fromFunc(moduleCallFunc));

        // module_list(buf_ptr, buf_len) -> i32
        Func moduleListFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostModuleList(bufPtr, bufLen));
                });
        hostFunctions.add(moduleListFunc);
        hostFunctionMap.put("module_list", Extern.fromFunc(moduleListFunc));

        // === Network Host Functions (multi-interface) ===

        // net_get_interface_count() -> i32
        Func netGetInterfaceCountFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(networkMacs.length);
                });
        hostFunctions.add(netGetInterfaceCountFunc);
        hostFunctionMap.put("net_get_interface_count", Extern.fromFunc(netGetInterfaceCountFunc));

        // net_get_interface_mac(index: i32, buf_ptr: i32) -> i32
        Func netGetInterfaceMacFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    writeBytesToMemory(networkMacs[index], bufPtr, 6);
                    results[0] = Val.fromI32(6);
                });
        hostFunctions.add(netGetInterfaceMacFunc);
        hostFunctionMap.put("net_get_interface_mac", Extern.fromFunc(netGetInterfaceMacFunc));

        // net_tx_frame_on(index: i32, buf_ptr: i32, frame_len: i32) -> i32
        Func netTxFrameOnFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    int frameLen = params[2].i32();
                    if (index < 0 || index >= networkMacs.length || frameLen < 14 || frameLen > 1518) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    byte[] frame = readBytesFromMemory(bufPtr, frameLen);
                    if (frame.length == 0) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub != null) {
                        hub.transmit(networkMacs[index], frame);
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netTxFrameOnFunc);
        hostFunctionMap.put("net_tx_frame_on", Extern.fromFunc(netTxFrameOnFunc));

        // net_rx_frame_on(index: i32, buf_ptr: i32, buf_len: i32) -> i32
        Func netRxFrameOnFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    byte[] frame = hub.receive(networkMacs[index]);
                    if (frame == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    int writeLen = Math.min(frame.length, bufLen);
                    writeBytesToMemory(frame, bufPtr, writeLen);
                    results[0] = Val.fromI32(writeLen);
                });
        hostFunctions.add(netRxFrameOnFunc);
        hostFunctionMap.put("net_rx_frame_on", Extern.fromFunc(netRxFrameOnFunc));

        // net_rx_frame_any(buf_ptr: i32, buf_len: i32, iface_idx_ptr: i32) -> i32
        Func netRxFrameAnyFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    int ifaceIdxPtr = params[2].i32();
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    for (int i = 0; i < networkMacs.length; i++) {
                        byte[] frame = hub.receive(networkMacs[i]);
                        if (frame != null) {
                            int writeLen = Math.min(frame.length, bufLen);
                            writeBytesToMemory(frame, bufPtr, writeLen);
                            // Write interface index as little-endian i32
                            byte[] idxBytes = new byte[]{
                                (byte)(i & 0xFF), (byte)((i >> 8) & 0xFF),
                                (byte)((i >> 16) & 0xFF), (byte)((i >> 24) & 0xFF)
                            };
                            writeBytesToMemory(idxBytes, ifaceIdxPtr, 4);
                            results[0] = Val.fromI32(writeLen);
                            return;
                        }
                    }
                    results[0] = Val.fromI32(-1);
                });
        hostFunctions.add(netRxFrameAnyFunc);
        hostFunctionMap.put("net_rx_frame_any", Extern.fromFunc(netRxFrameAnyFunc));

        // net_set_promiscuous_on(index: i32, enabled: i32) -> i32
        Func netSetPromiscuousOnFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int enabled = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub != null) {
                        hub.setPromiscuous(networkMacs[index], enabled != 0);
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netSetPromiscuousOnFunc);
        hostFunctionMap.put("net_set_promiscuous_on", Extern.fromFunc(netSetPromiscuousOnFunc));

        // net_pcap_enable(index: i32, enabled: i32) -> i32
        // Enable/disable packet capture mirror queue on an interface.
        Func netPcapEnableFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int enabled = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub != null) {
                        hub.setPcapEnabled(networkMacs[index], enabled != 0);
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netPcapEnableFunc);
        hostFunctionMap.put("net_pcap_enable", Extern.fromFunc(netPcapEnableFunc));

        // net_pcap_rx(index: i32, buf_ptr: i32, buf_len: i32) -> i32
        // Non-blocking receive from the pcap mirror queue (does not consume from main rx queue).
        Func netPcapRxFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    byte[] frame = hub.pcapReceive(networkMacs[index]);
                    if (frame == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    int writeLen = Math.min(frame.length, bufLen);
                    writeBytesToMemory(frame, bufPtr, writeLen);
                    results[0] = Val.fromI32(writeLen);
                });
        hostFunctions.add(netPcapRxFunc);
        hostFunctionMap.put("net_pcap_rx", Extern.fromFunc(netPcapRxFunc));

        // net_set_link_state(index: i32, up: i32) -> i32
        // Notifies the host that link state changed (for visual cable disconnect).
        Func netSetLinkStateFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int up = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    // Notify the terminal block entity to update visuals
                    if (host instanceof TerminalBlockEntity tbe) {
                        if (index < 6) {
                            tbe.setFaceDisabled(index, up == 0);
                            // Schedule block update on server thread
                            var server = host.getServer();
                            if (server != null) {
                                server.execute(() -> tbe.updateDisabledFaces(index, up != 0));
                            }
                        }
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netSetLinkStateFunc);
        hostFunctionMap.put("net_set_link_state", Extern.fromFunc(netSetLinkStateFunc));

        // === Kernel Extension Stubs ===
        // These host functions were added during the OS-to-kernel transformation.
        // They are no-op stubs for now; full implementations will be added for Java parity.
        // See simulator/src/host/ for reference implementations.
        createKernelExtensionStubs();

        // === wasm-bindgen stubs ===
        // These are stubs for wasm-bindgen functions that RustPython's dependencies require.
        // Most of these are never actually called in our non-browser environment.
        createWasmBindgenStubs();

        EvansComputerMod.LOGGER.debug("Created {} host functions", hostFunctions.size());
    }

    /**
     * Creates no-op stub host functions for the kernel extension imports
     * (FD operations, process management, TTY, sockets).
     * These allow the WASM module to load. Full implementations will be
     * added as Java parity work progresses (see KERN-042 through KERN-045).
     */
    private void createKernelExtensionStubs() {
        // --- File Descriptor operations ---
        // fd_open(path_ptr: i32, path_len: i32, flags: i32) -> i32
        addStubI32_3("fd_open");
        // fd_read(fd: i32, buf_ptr: i32, buf_len: i32) -> i32
        addStubI32_3("fd_read");
        // fd_write(fd: i32, buf_ptr: i32, buf_len: i32) -> i32
        addStubI32_3("fd_write");
        // fd_close(fd: i32) -> i32
        addStubI32_1("fd_close");
        // pipe_create(read_fd_ptr: i32, write_fd_ptr: i32) -> i32
        addStubI32_2("pipe_create");

        // --- Process management (real implementations) ---
        // process_spawn(path_ptr, path_len, argv_ptr, argv_len, stdin_fd, stdout_fd, stderr_fd) -> i32
        {
            Func f = new Func(store,
                    new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                    (caller, params, results) -> {
                        String path = readStringFromMemory(params[0].i32(), params[1].i32());
                        String argvStr = readStringFromMemory(params[2].i32(), params[3].i32());
                        if (path == null) {
                            results[0] = Val.fromI32(-1);
                            return;
                        }
                        // Resolve path through mount table
                        MountedPath mp = resolveReadPath(path);
                        if (mp == null || !java.nio.file.Files.exists(mp.realPath)) {
                            EvansComputerMod.LOGGER.debug("process_spawn: file not found: {}", path);
                            results[0] = Val.fromI32(-1);
                            return;
                        }
                        String[] argv = argvStr != null ? argvStr.split("\n") : new String[]{path};
                        // Pass terminal dimensions as env vars (like COLUMNS/LINES in Linux)
                        ByteBuffer fbBuf = memory.buffer(store);
                        int termW = (fbBuf.get(FB_BASE + 2) & 0xFF) | ((fbBuf.get(FB_BASE + 3) & 0xFF) << 8);
                        int termH = (fbBuf.get(FB_BASE + 4) & 0xFF) | ((fbBuf.get(FB_BASE + 5) & 0xFF) << 8);
                        var env = java.util.Map.of("COLUMNS", String.valueOf(termW), "LINES", String.valueOf(termH));
                        int pid = processManager.spawn(mp.realPath, argv, env);
                        results[0] = Val.fromI32(pid);
                    });
            hostFunctions.add(f);
            hostFunctionMap.put("process_spawn", Extern.fromFunc(f));
        }
        // process_wait(pid: i32) -> i32
        {
            Func f = new Func(store,
                    new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                    (caller, params, results) -> {
                        int pid = params[0].i32();
                        var stdoutPipe = processManager.getChildOutputPipe(pid);
                        var stdinPipe = processManager.getChildInputPipe(pid);

                        // Poll loop: forward input + drain output while waiting
                        byte[] buf = new byte[4096];
                        long lastSyncMs = 0;
                        while (true) {
                            checkInterrupted();
                            boolean hadOutput = false;
                            boolean hadInput = false;

                            // Keep kernel-side interrupt processing alive while waiting
                            // on a foreground child process (e.g. tcpdump). Without this,
                            // IRQ_NETWORK events are not dispatched and ARP/ICMP handling stalls.
                            drainAndDeliverInterrupts();

                            // Service WASI-child-staged gfx ops (e.g. the player
                            // pushing a decoded video frame) AND re-sync the Java
                            // display from kernel wasm. checkFramebufferDirty drains
                            // the pending op at its top, then inspects the dirty
                            // counters — the drain just bumped them, so it will
                            // read the new frame into the display and flag
                            // needsSync for the next server tick.
                            checkFramebufferDirty();

                            // Forward keyboard input to child's stdin
                            String input = inputQueue.poll();
                            if (input != null && stdinPipe != null) {
                                byte[] inputBytes = input.getBytes(java.nio.charset.StandardCharsets.UTF_8);
                                stdinPipe.write(inputBytes);
                                hadInput = true;
                            }

                            // Drain child stdout through VTE (no sync yet — batch for single sync)
                            if (stdoutPipe != null) {
                                int n = stdoutPipe.tryRead(buf);
                                if (n > 0) {
                                    drainBytesViaVteNoSync(buf, n);
                                    hadOutput = true;
                                }
                            }

                            // Service pending socket IPC requests from child
                            Func sockIpc = getHandleSockIpcFunc();
                            int servicedIpc = 0;
                            if (sockIpc != null && memory != null) {
                                servicedIpc = netIpcBridge.servicePending(store, memory, sockIpc);
                            }

                            // Periodically sync framebuffer to clients so interactive
                            // programs (edit, python REPL, etc.) display in real time
                            if (hadOutput) {
                                long now = System.currentTimeMillis();
                                if (now - lastSyncMs >= FB_SYNC_MIN_INTERVAL_MS) {
                                    lastSyncMs = now;
                                    readFramebufferFromWasm();
                                    host.syncToClients();
                                }
                            }

                            // Check if process exited
                            var state = processManager.getState(pid);
                            if (state == com.example.evanscomputermod.computer.wasi.ProcessManager.ProcessState.ZOMBIE) {
                                // Final drain — all remaining output through VTE without syncing
                                if (stdoutPipe != null) {
                                    int n;
                                    while ((n = stdoutPipe.tryRead(buf)) > 0) {
                                        drainBytesViaVteNoSync(buf, n);
                                    }
                                }
                                if (stdinPipe != null) stdinPipe.closeWrite();
                                // Clean up kernel socket state for this child
                                Func sockIpcCleanup = getHandleSockIpcFunc();
                                if (sockIpcCleanup != null && memory != null) {
                                    netIpcBridge.servicePending(store, memory, sockIpcCleanup);
                                    try {
                                        sockIpcCleanup.call(store,
                                                Val.fromI32(pid),
                                                Val.fromI32(com.example.evanscomputermod.computer.wasi.SocketFd.SOCK_DESTROY_SESSION),
                                                Val.fromI32(0x13000), Val.fromI32(0),
                                                Val.fromI32(0x14000), Val.fromI32(0));
                                    } catch (Exception ignored) {}
                                }
                                // Force keyframe so the complete output always reaches the client
                                // (bypasses delta protocol ack check that would drop this sync)
                                host.forceNextKeyframe();
                                readFramebufferFromWasm();
                                host.syncToClients();
                                int exitCode = processManager.waitForExit(pid);
                                results[0] = Val.fromI32(exitCode);
                                return;
                            }

                            try {
                                // Avoid coarse fixed sleeping: block until new IPC arrives,
                                // but keep short wakeups after local work for responsiveness.
                                long waitMs = (hadOutput || hadInput || servicedIpc > 0) ? 5L : 50L;
                                netIpcBridge.waitForPending(waitMs);
                            } catch (InterruptedException e) {
                                Thread.interrupted();
                                if (interrupted) throw new WasmInterruptedException("interrupted");
                            }
                        }
                    });
            hostFunctions.add(f);
            hostFunctionMap.put("process_wait", Extern.fromFunc(f));
        }
        // process_kill(pid: i32, signal: i32) -> i32
        {
            Func f = new Func(store,
                    new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                    (caller, params, results) -> {
                        results[0] = Val.fromI32(processManager.kill(params[0].i32()));
                    });
            hostFunctions.add(f);
            hostFunctionMap.put("process_kill", Extern.fromFunc(f));
        }
        // process_list(buf_ptr: i32, buf_len: i32) -> i32
        {
            Func f = new Func(store,
                    new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                    (caller, params, results) -> {
                        String json = processManager.listProcesses();
                        results[0] = Val.fromI32(writeStringToMemory(json, params[0].i32(), params[1].i32()));
                    });
            hostFunctions.add(f);
            hostFunctionMap.put("process_list", Extern.fromFunc(f));
        }
        // process_state(pid: i32) -> i32 (0=running, 2=zombie, -1=not found)
        {
            Func f = new Func(store,
                    new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                    (caller, params, results) -> {
                        var state = processManager.getState(params[0].i32());
                        results[0] = Val.fromI32(switch (state) {
                            case RUNNING -> 0;
                            case ZOMBIE -> 2;
                        });
                    });
            hostFunctions.add(f);
            hostFunctionMap.put("process_state", Extern.fromFunc(f));
        }

        // --- TTY management ---
        // tty_create(width: i32, height: i32) -> i32
        addStubI32_2("tty_create");
        // tty_attach_fd(tty_id: i32, mode: i32) -> i32
        addStubI32_2("tty_attach_fd");
        // tty_set_foreground(tty_id: i32) -> i32
        addStubI32_1("tty_set_foreground");
        // tty_get_size(tty_id: i32, width_ptr: i32, height_ptr: i32) -> i32
        // Reads terminal dimensions from the framebuffer header.
        {
            Func f = new Func(store,
                    new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                    (caller, params, results) -> {
                        int widthPtr = params[1].i32();
                        int heightPtr = params[2].i32();
                        ByteBuffer buf = memory.buffer(store);
                        int w = (buf.get(FB_BASE + 2) & 0xFF) | ((buf.get(FB_BASE + 3) & 0xFF) << 8);
                        int h = (buf.get(FB_BASE + 4) & 0xFF) | ((buf.get(FB_BASE + 5) & 0xFF) << 8);
                        buf.order(java.nio.ByteOrder.LITTLE_ENDIAN);
                        buf.putInt(widthPtr, w);
                        buf.putInt(heightPtr, h);
                        results[0] = Val.fromI32(0);
                    });
            hostFunctions.add(f);
            hostFunctionMap.put("tty_get_size", Extern.fromFunc(f));
        }

        EvansComputerMod.LOGGER.debug("Created kernel extension stub host functions");
    }

    /** Stub: (i32) -> i32, returns -1 */
    private void addStubI32_1(String name) {
        Func f = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> results[0] = Val.fromI32(-1));
        hostFunctions.add(f);
        hostFunctionMap.put(name, Extern.fromFunc(f));
    }

    /** Stub: (i32, i32) -> i32, returns -1 */
    private void addStubI32_2(String name) {
        Func f = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> results[0] = Val.fromI32(-1));
        hostFunctions.add(f);
        hostFunctionMap.put(name, Extern.fromFunc(f));
    }

    /** Stub: (i32, i32, i32) -> i32, returns -1 */
    private void addStubI32_3(String name) {
        Func f = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> results[0] = Val.fromI32(-1));
        hostFunctions.add(f);
        hostFunctionMap.put(name, Extern.fromFunc(f));
    }

    /**
     * Creates stub functions for wasm-bindgen imports.
     * These are required by RustPython's dependencies (chrono, js-sys, etc.)
     * but won't be called in our non-browser WASM environment.
     *
     * Type signatures from WASM analysis:
     * - type 0: (i32, i32, i32) -> void
     * - type 1: (i32) -> void
     * - type 2: (i32, i32) -> i32
     * - type 3: (i32, i32, i32) -> i32
     * - type 4: (i32) -> i32
     * - type 7: () -> i32
     * - type 16: (i32, i32) -> void
     * - type 20: (i32) -> f64
     * - type 21: () -> f64
     */
    private void createWasmBindgenStubs() {
        // wbindgen core functions
        addStubVoid("__wbindgen_describe", Type.I32);  // type 1: (i32) -> void
        addStubI32Return("__wbindgen_describe_cast", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32
        addStubVoid("__wbindgen_object_drop_ref", Type.I32);  // type 1: (i32) -> void - no-op, we don't track drops
        addObjectCloneRef("__wbindgen_object_clone_ref");  // type 4: (i32) -> i32 - return new handle

        // Date/time functions - properly implemented for chrono/time support
        addDateNew("__wbg_new_b2db8aa2650f793a");  // Date(timestamp) - type 4: (i32) -> i32
        addTimezoneOffset("__wbg_getTimezoneOffset_45389e26d6f46823");  // type 20: (i32) -> f64
        addDateNew0("__wbg_new_0_23cedd11d9b40c9d");  // Date() for now - type 7: () -> i32
        addGetTime("__wbg_getTime_ad1e9878a735af08");  // type 20: (i32) -> f64
        addDateNow("__wbg_now_2c70f2474e348581");  // type 21: () -> f64

        // Boolean checks - all type 4: (i32) -> i32
        addIsObject("__wbg___wbindgen_is_object_ce774f3490692386");  // Return true for non-zero handles
        addStubI32ReturnValue("__wbg___wbindgen_is_string_704ef9c8fc131030", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_function_8d400b8b1af978cd", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_undefined_f6b95eab589e0269", 1, Type.I32);  // Return true (undefined)

        // Crypto/random functions - return valid handles so getrandom can use them
        addCryptoObject("__wbg_crypto_574e78ad8b13b65f");  // type 4: (i32) -> i32
        addStubI32Return("__wbg_msCrypto_a61aeb35a24c1329", Type.I32);  // type 4: (i32) -> i32 - return 0 (no msCrypto)
        addRandomFillSync("__wbg_randomFillSync_ac0988aba3254290");  // type 16: (i32, i32) -> void
        addGetRandomValues("__wbg_getRandomValues_b8f5dbd5f3995a9e");  // type 16: (i32, i32) -> void

        // Node.js functions
        addStubI32Return("__wbg_process_dc0fbacc7c1c06f7", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_versions_c01dfd4722a88165", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_node_905d3e251edff8a2", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_require_60cc747a6bc5215a");  // type 7: () -> i32

        // Function call stubs
        addStubI32Return("__wbg_call_3020136f7a2d6e44", Type.I32, Type.I32, Type.I32);  // type 3: (i32, i32, i32) -> i32
        addStubI32Return("__wbg_call_abb4ff46ce38be40", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32

        // Global/window accessors - type 7: () -> i32
        addStubI32Return("__wbg_static_accessor_GLOBAL_769e6b65d6557335");
        addStubI32Return("__wbg_static_accessor_GLOBAL_THIS_60cf02db4de8e1c1");
        addStubI32Return("__wbg_static_accessor_WINDOW_a8924b26aa92d024");
        addStubI32Return("__wbg_static_accessor_SELF_08f5a74c69739274");

        // Array functions - return valid handles
        addUint8ArrayNew("__wbg_new_with_length_aa5eaf41d35235e5");  // type 4: (i32) -> i32
        addUint8ArraySubarray("__wbg_subarray_845f2f5bce7d061a");  // type 3: (i32, i32, i32) -> i32
        addUint8ArrayLength("__wbg_length_22ac23eaec9d8053");  // type 4: (i32) -> i32

        // Misc functions
        addStubI32Return("__wbg_new_no_args_cb138f77cf6151ee", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32
        addStubVoid("__wbg_prototypesetcall_dfe9b766cdc1f1fd", Type.I32, Type.I32, Type.I32);  // type 0: (i32, i32, i32) -> void

        // Error handling functions - these need special implementations to read error messages
        addErrorHandler("__wbg_error_d01e9edc65d6e61f");  // console.error
        addThrowHandler("__wbg___wbindgen_throw_dd24417ed36fc46e");  // throw exception

        // Externref table functions
        addStubVoid("__wbindgen_externref_table_set_null", Type.I32);  // type 1: (i32) -> void - no-op
        addExternrefTableGrow("__wbindgen_externref_table_grow");  // type 4: (i32) -> i32
    }

    /**
     * Adds a special handler for __wbg_error that logs the error message from WASM memory.
     */
    private void addErrorHandler(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    String msg = readStringFromMemory(ptr, len);
                    EvansComputerMod.LOGGER.error("WASM error ({}): {}", name, msg);
                    EvansComputerMod.LOGGER.error("WASM Error: {}", msg);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a special handler for __wbindgen_throw that logs the error and throws an exception.
     */
    private void addThrowHandler(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    String msg = readStringFromMemory(ptr, len);
                    EvansComputerMod.LOGGER.error("WASM throw ({}): {}", name, msg);
                    EvansComputerMod.LOGGER.error("WASM Throw: {}", msg);
                    throw new RuntimeException("WASM throw: " + msg);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a stub function that takes parameters and returns void.
     */
    private void addStubVoid(String name, Type... paramTypes) {
        final String funcName = name;
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.trace("WASM stub called: {} (void) with {} params", funcName, params.length);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a stub function that takes parameters and returns i32 (0).
     */
    private void addStubI32Return(String name, Type... paramTypes) {
        addStubI32ReturnValue(name, 0, paramTypes);
    }

    /**
     * Adds a stub function that takes parameters and returns a specific i32 value.
     */
    private void addStubI32ReturnValue(String name, int returnValue, Type... paramTypes) {
        final String funcName = name;
        final int retVal = returnValue;
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.trace("WASM stub called: {} -> i32({}) with {} params", funcName, retVal, params.length);
                    results[0] = Val.fromI32(retVal);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a stub function that takes parameters and returns a specific f64 value.
     */
    private void addStubF64Return(String name, double returnValue, Type... paramTypes) {
        final String funcName = name;
        final double retVal = returnValue;
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.trace("WASM stub called: {} -> f64({}) with {} params", funcName, retVal, params.length);
                    results[0] = Val.fromF64(retVal);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Date/Time Implementation ====================
    // These functions provide real time support for chrono and RustPython's time module

    /** Counter for allocating "Date object handles" */
    private int nextDateHandle = 1;

    /**
     * Date.now() - Returns current timestamp in milliseconds.
     * Signature: () -> f64
     */
    private void addDateNow(String name) {
        Func func = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    double now = (double) System.currentTimeMillis();
                    EvansComputerMod.LOGGER.debug("WASM Date.now() -> {}", now);
                    results[0] = Val.fromF64(now);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * new Date() - Creates a Date for current time.
     * Signature: () -> i32 (returns handle)
     */
    private void addDateNew0(String name) {
        Func func = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = nextDateHandle++;
                    EvansComputerMod.LOGGER.debug("WASM new Date() -> handle {}", handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * new Date(timestamp) - Creates a Date from a timestamp.
     * Signature: (i32) -> i32 (returns handle)
     */
    private void addDateNew(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = nextDateHandle++;
                    EvansComputerMod.LOGGER.debug("WASM new Date(timestamp) -> handle {}", handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Date.getTime() - Returns timestamp in milliseconds.
     * Signature: (i32) -> f64
     * Note: We don't track Date objects, so always return current time.
     */
    private void addGetTime(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    double now = (double) System.currentTimeMillis();
                    EvansComputerMod.LOGGER.debug("WASM Date.getTime() -> {}", now);
                    results[0] = Val.fromF64(now);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Date.getTimezoneOffset() - Returns timezone offset in minutes.
     * Signature: (i32) -> f64
     */
    private void addTimezoneOffset(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    // JavaScript returns offset as (UTC - local) in minutes
                    // Java returns (local - UTC) in milliseconds, so we need to negate and convert
                    int offsetMs = java.util.TimeZone.getDefault().getRawOffset();
                    double offsetMinutes = -offsetMs / 60000.0;
                    EvansComputerMod.LOGGER.debug("WASM Date.getTimezoneOffset() -> {} minutes", offsetMinutes);
                    results[0] = Val.fromF64(offsetMinutes);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Object Reference Management ====================
    // wasm-bindgen uses an externref table to track JS objects. We simulate this
    // by returning incrementing handles.

    /**
     * __wbindgen_object_clone_ref - Clone an object reference.
     * Signature: (i32) -> i32
     * Returns a new handle for the "cloned" object.
     */
    private void addObjectCloneRef(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int newHandle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM object_clone_ref({}) -> {}", params[0].i32(), newHandle);
                    results[0] = Val.fromI32(newHandle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbindgen_externref_table_grow - Grow the externref table.
     * Signature: (i32) -> i32
     * Returns the previous table size (we just return current handle counter).
     */
    private void addExternrefTableGrow(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int delta = params[0].i32();
                    int oldSize = nextObjectHandle.get();
                    nextObjectHandle.addAndGet(delta);
                    EvansComputerMod.LOGGER.debug("WASM externref_table_grow({}) -> {} (old size)", delta, oldSize);
                    results[0] = Val.fromI32(oldSize);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Crypto/Random Implementation ====================

    /** Random number generator for crypto functions */
    private final java.security.SecureRandom secureRandom = new java.security.SecureRandom();

    /**
     * __wbg_crypto_* - Get the crypto object.
     * Signature: (i32) -> i32
     * Returns a handle to a "crypto" object (non-zero so it's not null).
     */
    private void addCryptoObject(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    // Return a valid handle so the caller knows crypto is available
                    int handle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM crypto object requested -> handle {}", handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_getRandomValues_* - Fill a Uint8Array with random values.
     * Signature: (i32, i32) -> void
     * First param is the crypto object handle, second is the Uint8Array handle.
     * We need to fill the array in WASM memory with random bytes.
     */
    private void addGetRandomValues(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    // The second param is a Uint8Array handle, but we don't track the actual array
                    // Instead, we'll just log this was called - the actual random is handled by __getrandom_v03_custom
                    EvansComputerMod.LOGGER.debug("WASM getRandomValues called (crypto={}, array={})", params[0].i32(), params[1].i32());
                    // The real random generation happens via __getrandom_v03_custom which we already implement
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_randomFillSync_* - Node.js crypto.randomFillSync.
     * Signature: (i32, i32) -> void
     * First param is the crypto object handle, second is the buffer handle.
     */
    private void addRandomFillSync(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.debug("WASM randomFillSync called (crypto={}, buffer={})", params[0].i32(), params[1].i32());
                    // Similar to getRandomValues - the real random is via __getrandom_v03_custom
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Uint8Array Implementation ====================

    /**
     * __wbg_new_with_length_* - Create a new Uint8Array with given length.
     * Signature: (i32) -> i32
     * Returns a handle to the new array.
     */
    private void addUint8ArrayNew(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int length = params[0].i32();
                    int handle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM new Uint8Array({}) -> handle {}", length, handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_subarray_* - Get a subarray view.
     * Signature: (i32, i32, i32) -> i32
     * Returns a handle to the subarray.
     */
    private void addUint8ArraySubarray(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM Uint8Array.subarray({}, {}, {}) -> handle {}",
                            params[0].i32(), params[1].i32(), params[2].i32(), handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_length_* - Get array length.
     * Signature: (i32) -> i32
     * Returns the length of the array.
     */
    private void addUint8ArrayLength(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    // We don't track actual arrays, so return a reasonable default
                    // The actual length should be tracked by the WASM code
                    EvansComputerMod.LOGGER.debug("WASM Uint8Array.length({}) -> 0", params[0].i32());
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Type Checking Functions ====================

    /**
     * __wbg___wbindgen_is_object - Check if a handle refers to an object.
     * Signature: (i32) -> i32
     * Returns 1 (true) for non-zero handles, 0 (false) for zero (null).
     * This is critical for getrandom to detect the crypto object.
     */
    private void addIsObject(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = params[0].i32();
                    // Non-zero handles are valid objects
                    int result = (handle != 0) ? 1 : 0;
                    EvansComputerMod.LOGGER.debug("WASM is_object({}) -> {}", handle, result);
                    results[0] = Val.fromI32(result);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Host function: provides random bytes for getrandom 0.3.
     */
    private int hostGetrandom(int ptr, int len) {
        if (memory == null || len <= 0 || len > 4096) {
            return -1;
        }

        try {
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(ptr);

            // Simple xorshift64* PRNG (same as Rust side)
            java.util.Random random = new java.util.Random();
            byte[] bytes = new byte[len];
            random.nextBytes(bytes);
            buffer.put(bytes);

            return 0;  // Success
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error in hostGetrandom", e);
            return -1;
        }
    }

    /**
     * Creates the imports list in the order required by the module.
     * Uses the module's import list to determine the correct order.
     */
    private List<Extern> createImportsForModule(io.github.kawamuray.wasmtime.Module module) {
        List<Extern> imports = new ArrayList<>();

        // Get the module's imports and iterate in order
        var moduleImports = module.imports();

        for (var importType : moduleImports) {
            String moduleName = importType.module();
            String name = importType.name();

            EvansComputerMod.LOGGER.debug("Module requires import: {}::{}", moduleName, name);

            // Look up the host function by name
            if (hostFunctionMap.containsKey(name)) {
                imports.add(hostFunctionMap.get(name));
                EvansComputerMod.LOGGER.debug("  -> Matched to host function: {}", name);
            } else {
                // Unknown import - this will cause instantiation to fail
                // Log a warning so we know what's missing
                EvansComputerMod.LOGGER.warn("Unknown WASM import: {}::{} (type: {})",
                        moduleName, name, importType.type());

                // Try to provide a stub based on the import type
                Extern stub = createStubImport(importType);
                if (stub != null) {
                    imports.add(stub);
                    EvansComputerMod.LOGGER.debug("  -> Created stub for: {}", name);
                }
            }
        }

        return imports;
    }

    /**
     * Creates a stub import for unknown imports.
     * Generates a no-op function matching the import's type signature.
     */
    /**
     * Creates a stub import for unknown imports by introspecting the WASM
     * module's expected type signature and generating a matching no-op function.
     * Returns default values (0 for integers, 0.0 for floats) for any results.
     */
    private Extern createStubImport(io.github.kawamuray.wasmtime.ImportType importType) {
        try {
            io.github.kawamuray.wasmtime.ImportType.Type externType = importType.type();

            if (externType != io.github.kawamuray.wasmtime.ImportType.Type.FUNC) {
                EvansComputerMod.LOGGER.warn("Cannot create stub for non-function import: {}::{}",
                        importType.module(), importType.name());
                return null;
            }

            // Get the actual function type from the module's import declaration
            FuncType funcType = importType.func();
            Type[] paramTypes = funcType.getParams();
            Type[] resultTypes = funcType.getResults();

            // Create a no-op function matching the exact signature
            Func f = new Func(store, funcType, (caller, params, results) -> {
                // Return default values for all results
                for (int i = 0; i < results.length; i++) {
                    results[i] = switch (resultTypes[i]) {
                        case I32 -> Val.fromI32(0);
                        case I64 -> Val.fromI64(0);
                        case F32 -> Val.fromF32(0.0f);
                        case F64 -> Val.fromF64(0.0);
                        default -> Val.fromI32(0);
                    };
                }
            });
            hostFunctions.add(f);

            EvansComputerMod.LOGGER.debug("Auto-stubbed import: {}::{} ({} params, {} results)",
                    importType.module(), importType.name(), paramTypes.length, resultTypes.length);
            return Extern.fromFunc(f);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.warn("Failed to create stub import for {}::{}: {}",
                    importType.module(), importType.name(), e.getMessage());
            return null;
        }
    }

    // hostTerminalWrite removed — the WASM OS now writes directly to the
    // memory-mapped framebuffer. The host reads it via readFramebufferFromWasm().

    /**
     * Reads a string from WASM memory.
     */
    private String readStringFromMemory(int ptr, int len) {
        if (memory == null || len <= 0 || len > 4096) {
            return null;
        }
        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] bytes = new byte[len];
            buffer.position(ptr);
            buffer.get(bytes, 0, len);
            return new String(bytes, StandardCharsets.UTF_8);
        } catch (Exception e) {
            return null;
        }
    }

    /** Maximum path depth to prevent resource exhaustion. */
    private static final int MAX_PATH_DEPTH = 10;
    /** Maximum total path length. */
    private static final int MAX_PATH_LENGTH = 256;

    /** Resolved mount result: the real filesystem path and whether the mount is read-only. */
    private record MountedPath(Path realPath, boolean readOnly) {}

    /**
     * Resolve a virtual path through the mount table.
     * For read operations: tries each mount in order, returns first that exists.
     * For write operations: use resolveWritePath() instead.
     */
    private MountedPath resolveReadPath(String filename) {
        if (filename == null || filename.isEmpty()) return null;
        // Strip leading slash (Rust side resolves CWD, may produce absolute-looking paths)
        if (filename.startsWith("/")) filename = filename.substring(1);

        for (VirtualMount mount : mounts) {
            if (mount.matches(filename)) {
                Path resolved = mount.resolve(filename);
                if (resolved != null && Files.exists(resolved)) {
                    return new MountedPath(resolved, mount.isReadOnly());
                }
            }
        }
        // No mount had an existing file — return the first mount's resolution for "not found"
        for (VirtualMount mount : mounts) {
            if (mount.matches(filename)) {
                Path resolved = mount.resolve(filename);
                if (resolved != null) {
                    return new MountedPath(resolved, mount.isReadOnly());
                }
            }
        }
        return null;
    }

    /**
     * Resolve a virtual path for write operations.
     * Always resolves against the first writable mount that matches.
     */
    private MountedPath resolveWritePath(String filename) {
        if (filename == null || filename.isEmpty()) return null;
        if (filename.startsWith("/")) filename = filename.substring(1);

        for (VirtualMount mount : mounts) {
            if (mount.matches(filename) && !mount.isReadOnly()) {
                Path resolved = mount.resolve(filename);
                if (resolved != null) {
                    return new MountedPath(resolved, false);
                }
            }
        }
        return null;
    }

    /**
     * Legacy compatibility: resolve path for operations that just need any valid path.
     * Used by hostFileExists, hostFileIsDir, etc. that check existence themselves.
     */
    private Path sanitizePath(String filename) {
        MountedPath mp = resolveReadPath(filename);
        return mp != null ? mp.realPath : null;
    }

    /**
     * Host function: writes data to a file.
     */
    private int hostFileWrite(int pathPtr, int pathLen, int dataPtr, int dataLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String filename = readStringFromMemory(pathPtr, pathLen);
        MountedPath mp = resolveWritePath(filename);
        if (mp == null) {
            return -1;
        }
        Path filePath = mp.realPath;

        if (dataLen < 0 || dataLen > 1024 * 1024) { // 1MB max file size
            return -1;
        }

        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] data = new byte[dataLen];
            buffer.position(dataPtr);
            buffer.get(data, 0, dataLen);

            // Create parent directories if needed (for nested paths like "dir/file.txt")
            Path parent = filePath.getParent();
            if (parent != null && !Files.exists(parent)) {
                Files.createDirectories(parent);
            }

            Files.write(filePath, data, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING);
            EvansComputerMod.LOGGER.debug("Wrote {} bytes to file: {}", dataLen, filename);
            return dataLen;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing file: {}", filename, e);
            return -1;
        }
    }

    /**
     * Host function: reads data from a file.
     */
    private int hostFileRead(int pathPtr, int pathLen, int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null || !Files.exists(filePath)) {
            return -1;
        }

        try {
            byte[] data = Files.readAllBytes(filePath);
            int bytesToRead = Math.min(data.length, bufLen);

            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(data, 0, bytesToRead);

            EvansComputerMod.LOGGER.debug("Read {} bytes from file: {}", bytesToRead, filename);
            return bytesToRead;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading file: {}", filename, e);
            return -1;
        }
    }

    /**
     * Host function: gets the size of a file.
     */
    private int hostFileSize(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null || !Files.exists(filePath)) {
            return -1;
        }

        try {
            return (int) Files.size(filePath);
        } catch (Exception e) {
            return -1;
        }
    }

    /**
     * Host function: checks if a file exists.
     */
    private int hostFileExists(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null) {
            return 0;
        }
        return Files.exists(filePath) ? 1 : 0;
    }

    /**
     * Host function: deletes a file.
     */
    private int hostFileDelete(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        MountedPath mp = resolveWritePath(filename);
        if (mp == null) return 0;
        Path filePath = mp.realPath;
        if (!Files.exists(filePath)) {
            return 0;
        }

        try {
            Files.delete(filePath);
            EvansComputerMod.LOGGER.debug("Deleted file: {}", filename);
            return 1;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error deleting file: {}", filename, e);
            return 0;
        }
    }

    private int hostFileMkdir(int pathPtr, int pathLen) {
        String dirname = readStringFromMemory(pathPtr, pathLen);
        MountedPath mp = resolveWritePath(dirname);
        if (mp == null) {
            return -1;
        }
        Path dirPath = mp.realPath;
        try {
            Files.createDirectories(dirPath);
            EvansComputerMod.LOGGER.debug("Created directory: {}", dirname);
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error creating directory: {}", dirname, e);
            return -1;
        }
    }

    private int hostFileIsDir(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        if (filename == null || filename.isEmpty()) {
            return 1; // Empty path = root, which is a directory
        }
        Path filePath = sanitizePath(filename);
        if (filePath == null) {
            return 0;
        }
        return Files.isDirectory(filePath) ? 1 : 0;
    }

    private int hostFileListDir(int pathPtr, int pathLen, int bufPtr, int bufLen) {
        if (memory == null) {
            return -1;
        }
        String dirname = readStringFromMemory(pathPtr, pathLen);
        if (dirname == null) dirname = "";

        // Collect entries from all matching mounts (merge results, user storage first)
        java.util.Set<String> seen = new java.util.LinkedHashSet<>();
        boolean foundDir = false;

        for (VirtualMount mount : mounts) {
            String resolvedDirname = dirname.isEmpty() ? "" : dirname;
            if (mount.matches(resolvedDirname) || resolvedDirname.isEmpty()) {
                Path dirPath;
                if (resolvedDirname.isEmpty() && mount.getPrefix().isEmpty()) {
                    dirPath = mount.getRealRoot();
                } else if (mount.matches(resolvedDirname)) {
                    dirPath = mount.resolve(resolvedDirname);
                } else {
                    continue;
                }
                if (dirPath != null && Files.isDirectory(dirPath)) {
                    foundDir = true;
                    try (var stream = Files.list(dirPath)) {
                        stream.forEach(p -> {
                            String prefix = Files.isDirectory(p) ? "d:" : "f:";
                            seen.add(prefix + p.getFileName().toString());
                        });
                    } catch (Exception e) {
                        // continue to next mount
                    }
                }
            }
        }

        // When listing root, add virtual mount point directories
        if (dirname.isEmpty()) {
            for (VirtualMount mount : mounts) {
                if (!mount.getPrefix().isEmpty()) {
                    seen.add("d:" + mount.getPrefix());
                }
            }
        }

        if (!foundDir && seen.isEmpty()) {
            return -1;
        }

        String listing = seen.stream().sorted().collect(Collectors.joining("\n"));
        return writeStringToMemory(listing, bufPtr, bufLen);
    }

    /**
     * Converts a relative side index to an absolute Minecraft Direction.
     * Relative sides are based on the host's facing direction.
     *
     * @param relativeSide 0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT
     * @return The absolute Direction
     */
    private Direction relativeToAbsolute(int relativeSide) {
        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider != null) {
            return provider.relativeToAbsolute(relativeSide);
        }
        // Fallback: assume NORTH facing
        Direction facing = Direction.NORTH;
        return switch (relativeSide) {
            case 0 -> Direction.DOWN;
            case 1 -> Direction.UP;
            case 2 -> facing;                      // FRONT - the direction the screen faces
            case 3 -> facing.getOpposite();        // BACK - opposite of front
            case 4 -> facing.getCounterClockWise(); // LEFT - to the left of the terminal
            case 5 -> facing.getClockWise();       // RIGHT - to the right of the terminal
            default -> Direction.NORTH;
        };
    }

    /**
     * Host function: sets redstone output power for a specific side.
     * @param relativeSide The relative side index (0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT)
     * @param power The power level (0-15)
     * @return 0 on success, -1 on failure
     */
    private int hostRedstoneSetOutput(int relativeSide, int power) {
        if (relativeSide < 0 || relativeSide > 5 || power < 0 || power > 15) {
            return -1;
        }

        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) {
            return -1;
        }

        try {
            // Convert relative side to absolute direction
            Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
            int absoluteSide = absoluteDir.ordinal();

            // Schedule the redstone update on the main server thread
            if (host.getServer() != null) {
                host.getServer().execute(() -> {
                    provider.setRedstoneOutput(absoluteSide, power);
                });
            } else {
                // Fallback: direct call (might be on main thread already)
                provider.setRedstoneOutput(absoluteSide, power);
            }
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error setting redstone output", e);
            return -1;
        }
    }

    /**
     * Host function: reads redstone input power for a specific relative side.
     * @param relativeSide The relative side index (0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT)
     * @return The power level (0-15), or -1 on invalid side
     */
    private int hostRedstoneGetInput(int relativeSide) {
        if (relativeSide < 0 || relativeSide > 5) {
            return -1;
        }

        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) {
            return 0;
        }

        try {
            Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
            return provider.getRedstoneInput(absoluteDir.ordinal());
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading redstone input", e);
            return -1;
        }
    }

    /**
     * Host function: reads all 6 redstone input levels into a WASM memory buffer.
     * Writes 6 little-endian i32 values (24 bytes) at buf_ptr, in relative side order.
     * @param bufPtr WASM memory address to write to
     * @return 0 on success, -1 on failure
     */
    private int hostRedstoneGetAllInput(int bufPtr) {
        if (memory == null) return -1;

        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) {
            return -1;
        }

        try {
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            for (int relativeSide = 0; relativeSide < 6; relativeSide++) {
                Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
                int power = provider.getRedstoneInput(absoluteDir.ordinal());
                // Write as little-endian i32
                buffer.put((byte) (power & 0xFF));
                buffer.put((byte) ((power >> 8) & 0xFF));
                buffer.put((byte) ((power >> 16) & 0xFF));
                buffer.put((byte) ((power >> 24) & 0xFF));
            }
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading all redstone inputs", e);
            return -1;
        }
    }

    // --- ChildHostBridge accessors (called from child WASI processes) ---
    // These delegate to the existing kernel host implementations but accept
    // plain Java values instead of touching WASM memory, so they can be called
    // by child processes that have their own separate WASM memory.

    public int bridgeRedstoneSetOutput(int side, int power) {
        return hostRedstoneSetOutput(side, power);
    }

    public int bridgeRedstoneGetInput(int side) {
        return hostRedstoneGetInput(side);
    }

    public int bridgeRedstoneGetAllInput(int[] out) {
        if (out == null || out.length < 6) return -1;
        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) return -1;
        try {
            for (int relativeSide = 0; relativeSide < 6; relativeSide++) {
                Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
                out[relativeSide] = provider.getRedstoneInput(absoluteDir.ordinal());
            }
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading all redstone inputs (bridge)", e);
            return -1;
        }
    }

    public String bridgePeripheralListJson() {
        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) return "[]";
        return pm.listPeripheralsAsJson();
    }

    public String bridgePeripheralMethodsJson(String name) {
        if (name == null) return "{\"ok\":false,\"error\":\"Invalid peripheral name\"}";
        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            return "{\"ok\":false,\"error\":\"CC:Tweaked not available\"}";
        }
        return pm.getMethodNamesAsJson(name);
    }

    public String bridgePeripheralCall(String name, String method, String argsJson) {
        if (name == null || method == null) {
            return "{\"ok\":false,\"error\":\"Invalid arguments\"}";
        }
        if (argsJson == null || argsJson.isEmpty()) argsJson = "[]";
        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            return "{\"ok\":false,\"error\":\"CC:Tweaked not available\"}";
        }
        var peripheralOpt = pm.getPeripheral(name);
        if (peripheralOpt.isEmpty()) {
            return "{\"ok\":false,\"error\":\"Peripheral not found: " + name + "\"}";
        }
        PeripheralMethodInvoker invoker = getPeripheralInvoker();
        var server = host.getServer();
        return invoker.invokeMethod(peripheralOpt.get().getPeripheral(), method, argsJson, server);
    }

    public void bridgeSleepMs(int ms) {
        // NOTE: Do NOT call hostSleepMs here. hostSleepMs accesses the kernel's
        // wasmtime store (via checkFramebufferDirty -> memory.buffer(store)) which
        // is not thread-safe — calling it from a child WASI thread deadlocks
        // wasmtime's internal locks. Use a plain Thread.sleep instead.
        int clamped = Math.max(0, Math.min(60_000, ms));
        if (clamped == 0) return;
        try {
            Thread.sleep(clamped);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    // --- Child networking capture bridge ---

    public int bridgeNetSetPromiscuousOn(int index, int enabled) {
        if (networkMacs == null || index < 0 || index >= networkMacs.length) return -1;
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return -1;
        hub.setPromiscuous(networkMacs[index], enabled != 0);
        return 0;
    }

    public int bridgeNetPcapEnable(int index, int enabled) {
        if (networkMacs == null || index < 0 || index >= networkMacs.length) return -1;
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return -1;
        hub.setPcapEnabled(networkMacs[index], enabled != 0);
        return 0;
    }

    public byte[] bridgeNetPcapRx(int index) {
        if (networkMacs == null || index < 0 || index >= networkMacs.length) return null;
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return null;
        return hub.pcapReceive(networkMacs[index]);
    }

    // --- Video playback bridge (WASI player support) ---
    // These methods are called from the child's WASI host-function threads
    // (via ChildHostBridge). They must NOT touch the kernel's wasmtime store
    // — the only kernel state they mutate is `TerminalDisplay` (Java heap,
    // guarded by its own lock) and the volatile `needsSync` flag. The server
    // tick thread picks up `needsSync` and forwards the display to clients.

    /**
     * Open an MP4 from the VFS path. Returns a non-negative handle, or -1
     * on any failure (missing file, codec not supported, etc). Logs a
     * descriptive reason at INFO level so the user can diagnose failures
     * (wrong filename, wrong directory, bad codec) without recompiling.
     */
    public int bridgeVideoOpen(String vfsPath, int targetW, int targetH) {
        if (vfsPath == null || vfsPath.isEmpty()) {
            EvansComputerMod.LOGGER.info("bridgeVideoOpen: rejected empty path");
            return -1;
        }
        if (targetW <= 0 || targetH <= 0) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): rejected size {}x{}", vfsPath, targetW, targetH);
            return -1;
        }
        MountedPath mp = resolveReadPath(vfsPath);
        if (mp == null || mp.realPath == null) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): no mount matched", vfsPath);
            return -1;
        }
        if (!Files.exists(mp.realPath)) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): resolved to {} but file does not exist",
                    vfsPath, mp.realPath.toAbsolutePath());
            return -1;
        }
        int handle = videoRegistry.open(mp.realPath, targetW, targetH);
        if (handle < 0) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): decoder rejected {} ({}x{}) — see previous log line",
                    vfsPath, mp.realPath.toAbsolutePath(), targetW, targetH);
        } else {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): handle={} path={} size={}x{}",
                    vfsPath, handle, mp.realPath.toAbsolutePath(), targetW, targetH);
        }
        return handle;
    }

    /**
     * Return the decoder's static metadata. Caller owns the result; returns
     * null if the handle is unknown.
     */
    public com.example.evanscomputermod.computer.video.VideoDecoder.VideoInfo bridgeVideoGetInfo(int handle) {
        var d = videoRegistry.get(handle);
        return d == null ? null : d.info();
    }

    /**
     * Decode the next video frame on the CHILD thread (FFmpeg is pure
     * Java/native — no wasmtime access required), then stage the resulting
     * pixel bytes as a FRAME op and block until the worker thread has
     * written them into kernel WASM memory at the target's gfx region
     * and bumped the pixel dirty counter. The existing
     * {@link #checkFramebufferDirty} flow then picks up the change and
     * pushes it to clients through the Java display like any other
     * kernel-driven gfx update.
     *
     * @param target {@link #GFX_TARGET_TERMINAL} or {@link #GFX_TARGET_SCREEN}
     * @return presentation timestamp in ms, -1 on EOF, -2 on any error
     */
    public long bridgeVideoDecodeToGfx(int handle, int target) {
        var d = videoRegistry.get(handle);
        if (d == null) return -2L;
        // For the screen target, bail early if no cluster is attached
        // rather than staging a frame that nothing would consume.
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -2L;
        try {
            var frame = d.next();
            if (frame == null) return -1L;
            stageGfxOpFrame(target, d.targetWidth(), d.targetHeight(), frame.indexed);
            return frame.ptsMs;
        } catch (IOException e) {
            EvansComputerMod.LOGGER.debug("bridgeVideoDecodeToGfx failed", e);
            return -2L;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -2L;
        }
    }

    /** Seek the given video to approximately {@code ptsMs} milliseconds. */
    public int bridgeVideoSeek(int handle, long ptsMs) {
        var d = videoRegistry.get(handle);
        if (d == null) return -1;
        try {
            d.seek(ptsMs);
            return 0;
        } catch (IOException e) {
            return -1;
        }
    }

    /** Close a decoder and free its native resources. */
    public int bridgeVideoClose(int handle) {
        if (videoRegistry.get(handle) == null) return -1;
        videoRegistry.close(handle);
        return 0;
    }

    /**
     * Initialize the selected display's graphics framebuffer: set magic
     * + mode=1 + dimensions, install the RGB332 palette, clear pixels,
     * and bump both dirty counters so the worker loop picks it up.
     * Drains on the worker thread; blocks the child until applied.
     */
    public int bridgeGfxInit(int target, int w, int h) {
        if (w <= 0 || h <= 0 || w > 4096 || h > 4096) return -1;
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -1;
        try {
            stageGfxOpInit(target, w, h);
            return 0;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }
    }

    /**
     * Switch the selected display's mode byte in kernel WASM memory
     * (0 text, 1 gfx, 2 overlay). Does not touch pixel/palette state.
     * Mode=0 is how the player program cleanly returns control to the
     * text shell on exit for the terminal target. Drains on the worker
     * thread; blocks the child until applied.
     */
    public int bridgeGfxSetMode(int target, int mode) {
        if (mode < 0 || mode > 2) return -1;
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -1;
        try {
            stageGfxOpSetMode(target, mode);
            return 0;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }
    }

    /**
     * Query the attached Screen cluster's pixel dimensions. Returns
     * {@code (width << 32) | height} if a cluster is attached, or
     * {@code -1L} otherwise. Packed into a single {@code long} so the WASI
     * host function can write both values to child memory in one call.
     *
     * <p>Reads from {@code ScreenClusterInfo} rather than
     * {@code TerminalDisplay.getGfxWidth()} because the cluster info is
     * populated as soon as the cluster is (re)formed, while the display's
     * own width/height are only set when something has actually drawn to
     * it — the player might call this before the kernel has written
     * anything to the screen framebuffer.
     *
     * <p>Thread-safety: reads a {@code volatile} field on TerminalBlockEntity,
     * so it's safe to call directly from the child thread without staging.
     */
    public long bridgeScreenQueryDims() {
        if (host instanceof com.example.evanscomputermod.block.TerminalBlockEntity tbe) {
            var info = tbe.getScreenClusterInfo();
            if (info != null && info.gfxWidth() > 0 && info.gfxHeight() > 0) {
                return ((long) info.gfxWidth() << 32) | (info.gfxHeight() & 0xFFFFFFFFL);
            }
        }
        return -1L;
    }

    /** True if this computer's host currently has an attached screen cluster. */
    private boolean hasAttachedScreen() {
        return host instanceof com.example.evanscomputermod.block.TerminalBlockEntity tbe
                && tbe.hasScreenCluster();
    }

    /**
     * Turn the attached Screen cluster on or off. Safe to call from
     * the child thread — {@code setScreenPower} internally schedules
     * the level mutation on the server executor. No-op if no cluster
     * is attached.
     */
    public void bridgeScreenSetPower(boolean on) {
        if (host instanceof com.example.evanscomputermod.block.TerminalBlockEntity tbe) {
            tbe.setScreenPower(on);
        }
    }

    /** Close every open decoder. Called on shutdown. */
    public void bridgeVideoCloseAll() {
        videoRegistry.closeAll();
        // Release any child thread blocked waiting for a drain. The
        // worker thread is tearing down; the child should unblock with
        // an error instead of hanging forever.
        synchronized (gfxOpLock) {
            pendingGfxOpKind = null;
            gfxOpLock.notifyAll();
        }
    }

    // --- Gfx op staging (child thread) ---
    //
    // These helpers run on the WASI child thread. They install a
    // pending op into the shared slot and then wait for the worker
    // thread to drain it. `drainPendingGfxOps()` runs only on the
    // worker thread and actually writes to kernel WASM memory.

    private void stageGfxOpInit(int target, int w, int h) throws InterruptedException {
        synchronized (gfxOpLock) {
            waitUntilSlotFree();
            pendingGfxOpKind = GfxOpKind.INIT;
            pendingGfxOpTarget = target;
            pendingGfxOpWidth = w;
            pendingGfxOpHeight = h;
            pendingGfxOpPixels = null;
            gfxOpLock.notifyAll();
            waitUntilDrained();
        }
    }

    private void stageGfxOpFrame(int target, int w, int h, byte[] pixels) throws InterruptedException {
        // Defensive copy — the decoder reuses its internal frame buffer.
        byte[] copy = new byte[pixels.length];
        System.arraycopy(pixels, 0, copy, 0, pixels.length);
        synchronized (gfxOpLock) {
            waitUntilSlotFree();
            pendingGfxOpKind = GfxOpKind.FRAME;
            pendingGfxOpTarget = target;
            pendingGfxOpWidth = w;
            pendingGfxOpHeight = h;
            pendingGfxOpPixels = copy;
            gfxOpLock.notifyAll();
            waitUntilDrained();
        }
    }

    private void stageGfxOpSetMode(int target, int mode) throws InterruptedException {
        synchronized (gfxOpLock) {
            waitUntilSlotFree();
            pendingGfxOpKind = GfxOpKind.SET_MODE;
            pendingGfxOpTarget = target;
            pendingGfxOpMode = mode;
            pendingGfxOpPixels = null;
            gfxOpLock.notifyAll();
            waitUntilDrained();
        }
    }

    // Caller must hold gfxOpLock. Waits until a previous op has been
    // drained by the worker thread (or until interrupted).
    private void waitUntilSlotFree() throws InterruptedException {
        while (pendingGfxOpKind != null) {
            gfxOpLock.wait(100);
        }
    }

    // Caller must hold gfxOpLock. Waits until the op we just staged
    // has been cleared by the worker thread.
    private void waitUntilDrained() throws InterruptedException {
        long deadline = System.currentTimeMillis() + 5_000;
        while (pendingGfxOpKind != null) {
            long remaining = deadline - System.currentTimeMillis();
            if (remaining <= 0) {
                // Worker thread never drained — release the slot so the
                // next attempt doesn't hang behind this one.
                EvansComputerMod.LOGGER.warn("gfx op drain timed out after 5s");
                pendingGfxOpKind = null;
                return;
            }
            gfxOpLock.wait(Math.min(remaining, 100));
        }
    }

    /**
     * Drain the single pending gfx op, applying it to kernel WASM
     * memory and bumping the relevant dirty counters. Must only be
     * called on the worker thread — touches the Wasmtime store. Safe
     * to call frequently; it's a no-op when the slot is empty.
     *
     * <p>Does NOT call {@code readFramebufferFromWasm} itself — that
     * is done by {@link #checkFramebufferDirty} on its next pass,
     * which also handles dirty-counter tracking and notifySync. The
     * worker loop must call checkFramebufferDirty after the drain to
     * propagate the change to the Java display.
     */
    private void drainPendingGfxOps() {
        GfxOpKind kind;
        int target, w, h, mode;
        byte[] pixels;
        synchronized (gfxOpLock) {
            kind = pendingGfxOpKind;
            if (kind == null) return;
            target = pendingGfxOpTarget;
            w = pendingGfxOpWidth;
            h = pendingGfxOpHeight;
            mode = pendingGfxOpMode;
            pixels = pendingGfxOpPixels;
        }
        try {
            int base = (target == GFX_TARGET_SCREEN) ? SCREEN_GFX_BASE : GFX_BASE;
            switch (kind) {
                case INIT     -> applyGfxInit(base, w, h);
                case FRAME    -> applyGfxFrame(base, w, h, pixels);
                case SET_MODE -> applyGfxSetMode(base, mode);
            }
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("drainPendingGfxOps failed for {}", kind, e);
        } finally {
            synchronized (gfxOpLock) {
                pendingGfxOpKind = null;
                pendingGfxOpPixels = null;
                gfxOpLock.notifyAll();
            }
        }
    }

    /**
     * Write the gfx header (magic, mode=1, w, h), the RGB332 palette,
     * and zero pixels into kernel WASM memory at {@code base}; bump
     * both dirty counters so the worker loop picks up the change.
     */
    private void applyGfxInit(int base, int w, int h) {
        if (memory == null) return;
        ByteBuffer buf = memory.buffer(store);
        if (buf == null) return;
        int total = GFX_PIXEL_OFF + w * h;
        if (buf.capacity() < base + total) return;

        buf.order(ByteOrder.LITTLE_ENDIAN);

        // Header.
        buf.put(base + 0, (byte) 0x02);
        buf.put(base + 1, (byte) 0xFB);
        buf.put(base + 2, (byte) 1);    // mode = gfx
        buf.put(base + 3, (byte) 0);
        buf.putShort(base + 4, (short) w);
        buf.putShort(base + 6, (short) h);

        // Bump dirty counters (read-modify-write). The kernel OS may
        // have previously been using this region, so we don't assume
        // any particular starting value.
        int palDirty = buf.getInt(base + 0x08) + 1;
        int pixDirty = buf.getInt(base + 0x0C) + 1;
        buf.putInt(base + 0x08, palDirty);
        buf.putInt(base + 0x0C, pixDirty);

        // Palette: copy the 768-byte RGB332 palette into offset 0x40.
        byte[] palette = com.example.evanscomputermod.computer.video.Rgb332Palette.bytes();
        for (int i = 0; i < palette.length; i++) {
            buf.put(base + GFX_PALETTE_OFF + i, palette[i]);
        }

        // Pixels: zero-fill. (Optional; the first decoded frame will
        // overwrite anyway, but start from a known state to avoid a
        // one-frame flash of stale kernel content.)
        int pixelBase = base + GFX_PIXEL_OFF;
        for (int i = 0; i < w * h; i++) {
            buf.put(pixelBase + i, (byte) 0);
        }
    }

    /**
     * Write the new pixel bytes into kernel WASM memory and bump the
     * pixel dirty counter. The palette is already installed by init;
     * frames don't re-push it.
     */
    private void applyGfxFrame(int base, int w, int h, byte[] pixels) {
        if (memory == null || pixels == null) return;
        ByteBuffer buf = memory.buffer(store);
        if (buf == null) return;
        int total = GFX_PIXEL_OFF + w * h;
        if (buf.capacity() < base + total) return;
        if (pixels.length < w * h) return;

        buf.order(ByteOrder.LITTLE_ENDIAN);

        // Make sure the header still says "this much gfx, mode=1" —
        // the decoder may run after an unrelated kernel gfx program
        // reset the region. Cheap to re-write every frame.
        buf.put(base + 0, (byte) 0x02);
        buf.put(base + 1, (byte) 0xFB);
        buf.put(base + 2, (byte) 1);
        buf.putShort(base + 4, (short) w);
        buf.putShort(base + 6, (short) h);

        int pixelBase = base + GFX_PIXEL_OFF;
        for (int i = 0; i < w * h; i++) {
            buf.put(pixelBase + i, pixels[i]);
        }

        int pixDirty = buf.getInt(base + 0x0C) + 1;
        buf.putInt(base + 0x0C, pixDirty);
    }

    /**
     * Update just the display mode byte and bump the pixel dirty
     * counter so the worker loop re-reads and propagates the change
     * to the Java display.
     */
    private void applyGfxSetMode(int base, int mode) {
        if (memory == null) return;
        ByteBuffer buf = memory.buffer(store);
        if (buf == null) return;
        if (buf.capacity() < base + 0x10) return;
        buf.put(base + 2, (byte) mode);
        int pixDirty = buf.getInt(base + 0x0C) + 1;
        buf.putInt(base + 0x0C, pixDirty);
    }

    /**
     * Host function: polls for the next pending interrupt.
     * Writes the payload into WASM memory at bufPtr (up to bufLen bytes).
     * Returns the IRQ number (>= 0) if an interrupt was polled, or -1 if none pending.
     * This is a pull-based API that avoids WASM reentrancy.
     */
    private int hostInterruptPoll(int bufPtr, int bufLen) {
        checkInterrupted();
        InterruptEvent evt = interruptQueue.poll();
        if (evt == null) {
            lastInterruptPayloadLen = 0;
            return -1;
        }

        // Write payload to WASM memory
        if (memory != null) {
            byte[] payloadBytes = evt.payload.getBytes(StandardCharsets.UTF_8);
            int writeLen = Math.min(payloadBytes.length, bufLen);
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(payloadBytes, 0, writeLen);
            lastInterruptPayloadLen = writeLen;
        } else {
            lastInterruptPayloadLen = 0;
        }

        return evt.irq;
    }

    /**
     * Host function: sleeps for the specified number of milliseconds.
     * This blocks the WASM execution but not the game server (since WASM runs on a background thread).
     * The Rust side handles chunking sleeps for interrupt delivery.
     * @param milliseconds Time to sleep (clamped to 0-60000ms)
     */
    private void hostSleepMs(int milliseconds) {
        checkInterrupted();
        int clampedMs = Math.max(0, Math.min(60000, milliseconds));
        try {
            // Sleep in chunks so we can sync the framebuffer during blocking loops.
            // The kernel calls sleep_ms(10) inside tcp_accept/tcp_recv/tcp_connect
            // loops, so checking every 50ms ensures the display stays updated even
            // when on_input() is blocked.
            long remaining = clampedMs;
            while (remaining > 0) {
                long chunk = Math.min(50, remaining);
                Thread.sleep(chunk);
                remaining -= chunk;
                checkInterrupted();
                checkFramebufferDirty();
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new WasmInterruptedException("Sleep interrupted");
        }
    }

    /**
     * Host function: reads a line of text input from the user.
     * Displays the prompt, then blocks while consuming keystrokes from inputQueue
     * until Enter is pressed. Echoes characters and handles backspace.
     *
     * @param promptPtr WASM memory address of prompt string (already displayed by Rust side)
     * @param promptLen Length of prompt string (unused -- Rust displays it)
     * @param bufPtr    WASM memory address to write the result line
     * @param bufLen    Maximum bytes to write
     * @return Number of bytes written, or -1 on error
     */
    private int hostReadLine(int promptPtr, int promptLen, int bufPtr, int bufLen) {
        if (interrupted) return -2; // Interrupted before we started

        StringBuilder lineBuffer = new StringBuilder();

        while (!shutdownRequested) {
            if (interrupted) return -2; // Interrupted -- return gracefully, let Rust handle reset

            try {
                String input = inputQueue.poll(100, TimeUnit.MILLISECONDS);
                if (input == null) continue;

                for (int i = 0; i < input.length(); i++) {
                    char c = input.charAt(i);

                    if (c == '\n' || c == '\r') {
                        // Enter pressed -- echo newline and return the line
                        // Rust OS handles echo via VTE
                        syncTerminalToClients();

                        // Write result to WASM memory
                        byte[] resultBytes = lineBuffer.toString().getBytes(StandardCharsets.UTF_8);
                        int writeLen = Math.min(resultBytes.length, bufLen);
                        if (memory != null && writeLen > 0) {
                            ByteBuffer buffer = memory.buffer(store);
                            buffer.position(bufPtr);
                            buffer.put(resultBytes, 0, writeLen);
                        }
                        return writeLen;
                    } else if (c == 0x14) {
                        // Ctrl+T -- return interrupt code, let Rust side handle reset
                        return -2;
                    } else if (c == 8 || c == 127) {
                        // Backspace
                        if (lineBuffer.length() > 0) {
                            lineBuffer.deleteCharAt(lineBuffer.length() - 1);
                            // Rust OS handles backspace echo via VTE
                        }
                    } else if (c >= 32) {
                        // Printable character
                        if (lineBuffer.length() < bufLen) {
                            lineBuffer.append(c);
                            // Rust OS handles character echo via VTE
                        }
                    }
                    // Ignore other control characters
                }

                syncTerminalToClients();
            } catch (InterruptedException e) {
                // Thread was interrupted (by wasmHost.interrupt()) -- return gracefully
                Thread.interrupted(); // Clear the flag
                return -2;
            }
        }

        return -1; // Shutdown requested
    }

    // === CC:Tweaked Peripheral Host Functions ===

    // Peripheral manager and invoker (lazily initialized)
    private PeripheralManager peripheralManager;
    private PeripheralMethodInvoker peripheralInvoker;

    // Module method invoker for annotation-driven auto-registration
    private ModuleMethodInvoker moduleMethodInvoker;

    /**
     * Gets or creates the peripheral manager for this computer.
     */
    private PeripheralManager getPeripheralManager() {
        IWorldAccess worldAccess = host.getWorldAccess();
        if (peripheralManager == null && worldAccess != null) {
            peripheralManager = new PeripheralManager(worldAccess);
            peripheralManager.scanPeripherals();
        }
        return peripheralManager;
    }

    /**
     * Gets or creates the peripheral invoker.
     */
    private PeripheralMethodInvoker getPeripheralInvoker() {
        if (peripheralInvoker == null) {
            peripheralInvoker = new PeripheralMethodInvoker();
        }
        return peripheralInvoker;
    }

    /**
     * Rescans peripherals. Called when neighbors change or after world reload.
     * This will create the peripheral manager if it doesn't exist yet.
     */
    public void rescanPeripherals() {
        PeripheralManager pm = getPeripheralManager();
        if (pm != null) {
            pm.scanPeripherals();
        }
    }

    /**
     * Host function: lists all connected peripherals as JSON.
     * Returns bytes written to buffer, or -1 on error.
     */
    private int hostPeripheralList(int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            // Return empty array if CC is not available
            String json = "[]";
            return writeStringToMemory(json, bufPtr, bufLen);
        }

        String json = pm.listPeripheralsAsJson();
        return writeStringToMemory(json, bufPtr, bufLen);
    }

    /**
     * Host function: gets method names for a peripheral.
     * Returns bytes written to buffer, or -1 on error.
     */
    private int hostPeripheralGetMethods(int namePtr, int nameLen, int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String peripheralName = readStringFromMemory(namePtr, nameLen);
        if (peripheralName == null) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"Invalid peripheral name\"}", bufPtr, bufLen);
        }

        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"CC:Tweaked not available\"}", bufPtr, bufLen);
        }

        String json = pm.getMethodNamesAsJson(peripheralName);
        return writeStringToMemory(json, bufPtr, bufLen);
    }

    /**
     * Host function: calls a peripheral method with JSON arguments.
     * Returns bytes written to result buffer, or -1 on error.
     */
    private int hostPeripheralCall(int namePtr, int nameLen, int methodPtr, int methodLen,
                                    int argsPtr, int argsLen, int resultPtr, int resultLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String peripheralName = readStringFromMemory(namePtr, nameLen);
        String methodName = readStringFromMemory(methodPtr, methodLen);
        String argsJson = argsLen > 0 ? readStringFromMemory(argsPtr, argsLen) : "[]";

        if (peripheralName == null || methodName == null) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"Invalid arguments\"}", resultPtr, resultLen);
        }

        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"CC:Tweaked not available\"}", resultPtr, resultLen);
        }

        // Find the peripheral
        var peripheralOpt = pm.getPeripheral(peripheralName);
        if (peripheralOpt.isEmpty()) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"Peripheral not found: " + peripheralName + "\"}", resultPtr, resultLen);
        }

        // Call the method
        PeripheralMethodInvoker invoker = getPeripheralInvoker();
        var server = host.getServer();
        String resultJson = invoker.invokeMethod(peripheralOpt.get().getPeripheral(), methodName, argsJson, server);

        return writeStringToMemory(resultJson, resultPtr, resultLen);
    }

    /**
     * Writes a string to WASM memory. Returns bytes written or -1 on error.
     */
    private int writeStringToMemory(String str, int bufPtr, int bufLen) {
        if (memory == null || str == null) {
            return -1;
        }

        try {
            byte[] bytes = str.getBytes(StandardCharsets.UTF_8);
            int bytesToWrite = Math.min(bytes.length, bufLen);

            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(bytes, 0, bytesToWrite);

            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing to WASM memory", e);
            return -1;
        }
    }

    // ==================== Module Call Bridge ====================

    /**
     * Gets or creates the module method invoker.
     */
    private ModuleMethodInvoker getModuleMethodInvoker() {
        if (moduleMethodInvoker == null) {
            moduleMethodInvoker = new ModuleMethodInvoker();
        }
        return moduleMethodInvoker;
    }

    /**
     * Host function: calls a registered module method using binary protocol.
     * Args and result are binary-encoded (not JSON).
     */
    private int hostModuleCall(int modulePtr, int moduleLen, int methodPtr, int methodLen,
                               int argsPtr, int argsLen, int resultPtr, int resultLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String moduleName = readStringFromMemory(modulePtr, moduleLen);
        String methodName = readStringFromMemory(methodPtr, methodLen);

        if (moduleName == null || methodName == null) {
            byte[] errorResult = ModuleMethodInvoker.serializeError("Invalid arguments");
            return writeBytesToMemory(errorResult, resultPtr, resultLen);
        }

        // Read args as raw binary (no string conversion)
        byte[] argsBinary = readBytesFromMemory(argsPtr, argsLen);

        ModuleMethodInvoker invoker = getModuleMethodInvoker();
        byte[] resultBinary = invoker.invokeMethod(host, moduleName, methodName, argsBinary);

        return writeBytesToMemory(resultBinary, resultPtr, resultLen);
    }

    /**
     * Reads raw bytes from WASM memory.
     */
    private byte[] readBytesFromMemory(int ptr, int len) {
        if (memory == null || len <= 0) {
            return new byte[0];
        }
        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] bytes = new byte[len];
            buffer.position(ptr);
            buffer.get(bytes, 0, len);
            return bytes;
        } catch (Exception e) {
            return new byte[0];
        }
    }

    /**
     * Writes raw bytes to WASM memory. Returns bytes written or -1 on error.
     */
    private int writeBytesToMemory(byte[] data, int ptr, int maxLen) {
        if (memory == null || data == null) {
            return -1;
        }
        try {
            int bytesToWrite = Math.min(data.length, maxLen);
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(ptr);
            buffer.put(data, 0, bytesToWrite);
            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing bytes to WASM memory", e);
            return -1;
        }
    }

    /**
     * Host function: returns JSON metadata of all registered modules.
     */
    private int hostModuleList(int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String json = ComputerModuleRegistry.getMetadataJson();
        return writeStringToMemory(json, bufPtr, bufLen);
    }

    /**
     * Host function: lists all files in the computer's storage.
     */
    private int hostFileList(int bufPtr, int bufLen) {
        if (memory == null) {
            return -1;
        }

        try {
            if (!Files.exists(computerStoragePath)) {
                return 0;
            }

            String fileList = Files.list(computerStoragePath)
                    .filter(Files::isRegularFile)
                    .map(p -> p.getFileName().toString())
                    .collect(Collectors.joining("\n"));

            byte[] data = fileList.getBytes(StandardCharsets.UTF_8);
            int bytesToWrite = Math.min(data.length, bufLen);

            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(data, 0, bytesToWrite);

            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error listing files", e);
            return -1;
        }
    }

    /**
     * Loads and instantiates a WASM module.
     * Queries the module's imports and provides them in the correct order.
     *
     * @param fileName The name of the WASM file (with or without .wasm extension)
     * @throws WasmManager.WasmExecutionException If loading fails
     */
    public void loadModule(String fileName) throws WasmManager.WasmExecutionException {
        // Ensure .wasm extension
        if (!fileName.endsWith(".wasm")) {
            fileName = fileName + ".wasm";
        }

        Path wasmFile = WasmManager.getWasmBinPath().resolve(fileName);

        if (!Files.exists(wasmFile)) {
            throw new WasmManager.WasmExecutionException("WASM file not found: " + wasmFile.toAbsolutePath());
        }

        try {
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromFile(engine, wasmFile.toString());

            EvansComputerMod.LOGGER.info("Loading WASM module: {}", fileName);

            // Create imports in the order required by the module
            List<Extern> imports = createImportsForModule(module);

            EvansComputerMod.LOGGER.info("Providing {} imports to WASM module", imports.size());

            // Create instance with imports
            instance = new Instance(store, module, imports);

            // Get the memory export for reading strings
            Optional<Memory> memoryOpt = instance.getMemory(store, "memory");
            if (memoryOpt.isPresent()) {
                memory = memoryOpt.get();
                EvansComputerMod.LOGGER.debug("Got WASM memory export");
            } else {
                EvansComputerMod.LOGGER.warn("WASM module does not export 'memory'");
            }

            EvansComputerMod.LOGGER.info("Successfully loaded WASM module: {}", fileName);

        } catch (WasmtimeException e) {
            throw new WasmManager.WasmExecutionException("Failed to load WASM module: " + e.getMessage(), e);
        }
    }

    /**
     * Executes a function from the loaded WASM module.
     *
     * @param functionName The name of the function to execute
     * @param params Parameters to pass to the function
     * @return The result of the function
     * @throws WasmManager.WasmExecutionException If execution fails
     */
    public WasmManager.WasmResult executeFunction(String functionName, Val... params)
            throws WasmManager.WasmExecutionException {

        if (instance == null) {
            throw new WasmManager.WasmExecutionException("No WASM module loaded");
        }

        try {
            Func func = instance.getFunc(store, functionName)
                    .orElseThrow(() -> new WasmManager.WasmExecutionException(
                            "Function '" + functionName + "' not found in module"));

            Val[] results = func.call(store, params);
            return new WasmManager.WasmResult(results);

        } catch (WasmtimeException e) {
            throw new WasmManager.WasmExecutionException("WASM execution error: " + e.getMessage(), e);
        }
    }

    /**
     * Executes the main/start function if it exists.
     * This must be called from the worker thread or before the worker thread is started.
     */
    public void executeMain() throws WasmManager.WasmExecutionException {
        if (instance == null) {
            throw new WasmManager.WasmExecutionException("No WASM module loaded");
        }

        // Try common entry point names
        String[] entryPoints = {"main", "_start", "start", "init"};

        for (String entryPoint : entryPoints) {
            Optional<Func> funcOpt = instance.getFunc(store, entryPoint);
            if (funcOpt.isPresent()) {
                try {
                    funcOpt.get().call(store);
                    EvansComputerMod.LOGGER.info("Executed WASM entry point: {}", entryPoint);
                    return;
                } catch (WasmtimeException e) {
                    throw new WasmManager.WasmExecutionException(
                            "Error executing " + entryPoint + ": " + e.getMessage(), e);
                }
            }
        }

        EvansComputerMod.LOGGER.warn("No entry point found in WASM module");
    }

    /**
     * Clears the interrupt flag, allowing WASM execution to resume.
     * Called after the OS has reset to shell mode following a Ctrl+T interrupt.
     */
    public void clearInterrupt() {
        interrupted = false;
        EvansComputerMod.LOGGER.debug("WASM interrupt flag cleared");
    }

    /**
     * Checks if the WASM module has faulted and should not be used.
     */
    public boolean isFaulted() {
        return faulted;
    }

    /**
     * Checks if the WASM execution has been interrupted.
     */
    public boolean isInterrupted() {
        return interrupted;
    }

    /**
     * Signals the WASM module to interrupt execution.
     * This sets the interrupt flag which is checked by host functions.
     * When a host function sees the interrupt flag, it throws an InterruptedException
     * to abort WASM execution.
     */
    public void interrupt() {
        interrupted = true;
        EvansComputerMod.LOGGER.info("WASM execution interrupt requested");
        // Increment the engine epoch — this causes any running WASM call to
        // trap immediately with an epoch-deadline-exceeded error, even if
        // the code is in a pure CPU-bound loop that never calls a host function.
        try {
            engine.incrementEpoch();
        } catch (Exception e) {
            EvansComputerMod.LOGGER.warn("Failed to increment epoch: {}", e.getMessage());
        }
        // Also interrupt the worker thread in case it's blocked (e.g., in Thread.sleep())
        if (workerThread != null && workerThread.isAlive()) {
            workerThread.interrupt();
        }
    }

    /**
     * Checks if execution should be interrupted and throws if so.
     * Called by host functions to allow interruption of long-running WASM code.
     */
    private void checkInterrupted() {
        if (interrupted) {
            EvansComputerMod.LOGGER.info("checkInterrupted() throwing WasmInterruptedException");
            throw new WasmInterruptedException("WASM execution interrupted");
        }
    }

    /**
     * Exception thrown when WASM execution is interrupted.
     */
    public static class WasmInterruptedException extends RuntimeException {
        public WasmInterruptedException(String message) {
            super(message);
        }
    }

    /**
     * Queues input to be processed by the WASM worker thread.
     * This is called when the user enters input in the terminal.
     * The input is processed asynchronously to keep the main server thread responsive.
     *
     * @param line The input line from the user
     */
    public void sendInput(String line) {
        if (instance == null || faulted) {
            if (faulted) {
                EvansComputerMod.LOGGER.error("WASM faulted - close and reopen terminal to reset");
            }
            return;
        }

        // Queue the input for the worker thread
        try {
            inputQueue.put(line);
            wakeWorker();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            EvansComputerMod.LOGGER.warn("Interrupted while queuing input");
        }
    }

    /** Get the primary network MAC address (interface 0). */
    public byte[] getNetworkMac() {
        return networkMacs[0];
    }

    /** Get all network MAC addresses. */
    public byte[][] getNetworkMacs() {
        return networkMacs;
    }

    @Override
    public void close() {
        // Unregister all NICs from network hub
        NetworkHub hub = NetworkHub.getInstance();
        if (hub != null && networkMacs != null) {
            for (byte[] mac : networkMacs) {
                hub.unregisterNic(mac);
            }
        }

        // Signal worker thread to stop
        shutdownRequested = true;
        interrupted = true;  // Also set interrupt to abort any running WASM

        // Interrupt and wait for worker thread to finish
        if (workerThread != null && workerThread.isAlive()) {
            workerThread.interrupt();
            try {
                workerThread.join(1000);  // Wait up to 1 second
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            if (workerThread.isAlive()) {
                EvansComputerMod.LOGGER.warn("WASM worker thread did not terminate in time");
            }
        }

        // Clear input queue
        inputQueue.clear();

        // Free FFmpeg decoder state before tearing down WASM.
        try {
            videoRegistry.closeAll();
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.warn("Error closing video decoder registry", t);
        }

        // Close WASM resources
        if (instance != null) {
            instance.close();
        }
        for (Func func : hostFunctions) {
            func.close();
        }
        hostFunctions.clear();
        hostFunctionMap.clear();
        // Only close the store - the engine is owned by the store
        store.close();
    }
}
