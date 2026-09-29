package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.api.wasm.WasmExport;
import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.example.evanscomputermod.api.wasm.WasmModuleHandle;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.api.wasm.WasmValType;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import com.example.evanscomputermod.wasm.WasmManager;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;

import com.example.evanscomputermod.block.TerminalBlockEntity;

/**
 * Provides host functions for WASM modules running in a computer context.
 * Allows WASM modules to write to the terminal display and access the file system.
 */
public class ComputerInstance implements AutoCloseable {

    // Directory for storing computer files (in game directory)
    private static final String COMPUTER_DATA_FOLDER = "computer-data";

    // volatile: hot-swapped by TerminalBlockEntity.adoptComputer when a
    // computer is transferred to a new BE (sable physics assembly,
    // /setblock replace, etc.). The worker thread reads `host` each tick
    // via a local copy, so reassignment is safe.
    private volatile IComputerHost host;
    private final Path computerStoragePath;
    private final List<VirtualMount> mounts = new ArrayList<>();
    private com.example.evanscomputermod.computer.wasi.ProcessManager processManager;
    private final com.example.evanscomputermod.computer.wasi.NetIpcBridge netIpcBridge = new com.example.evanscomputermod.computer.wasi.NetIpcBridge();

    // Peripheral events for programs using the `peripheral` API. Lives here
    // (not in the block entity's PeripheralHub) so subscriptions survive the
    // computer being adopted by a new block entity on a Sable move.
    private final com.example.evanscomputermod.computer.peripheral.PeripheralEventBus peripheralEvents
            = new com.example.evanscomputermod.computer.peripheral.PeripheralEventBus();

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
    // and writes the op directly into kernel WASM memory at gfxBase
    // or screenBase, bumps the corresponding dirty counters, and
    // notifyAll's the child. After drain, the existing
    // checkFramebufferDirty path naturally picks up the counter change
    // and pushes the frame to the Java display → client sync. Java
    // never touches the display directly on the bridge path.
    private final Object gfxOpLock = new Object();
    private enum GfxOpKind { INIT, FRAME, SET_MODE, SET_PIXEL_FORMAT, FRAME_RGBA, BLIT_RECT }
    private GfxOpKind pendingGfxOpKind;
    private int pendingGfxOpTarget;
    private int pendingGfxOpWidth;
    private int pendingGfxOpHeight;
    private int pendingGfxOpMode;
    private int pendingGfxOpPixelFormat;
    // x/y are only used by BLIT_RECT; other ops leave them at 0.
    private int pendingGfxOpX;
    private int pendingGfxOpY;
    private byte[] pendingGfxOpPixels;

    private WasmInstance instance;
    private WasmMemory memory;

    // Flag to indicate the WASM module has crashed and should not be used
    private volatile boolean faulted = false;

    // Flag to signal WASM execution should be interrupted (e.g., Ctrl+T or block break)
    private volatile boolean interrupted = false;
    // Set alongside `interrupted` on Ctrl+T, but NOT cleared when the
    // kernel trap is caught. Read by child-thread bridge methods
    // (bridgeVideo* / bridgeGfx* / bridgeScreen*) to immediately
    // short-circuit with an error, so a running WASI child (e.g. the
    // player) stops pushing frames while the kernel's reset_to_shell
    // clears the framebuffer. Cleared at the top of processInputOnWorker
    // so the next user keystroke starts from a clean state.
    private volatile boolean childAbortRequested = false;
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

    /** Cached reference to the kernel's handle_sock_ipc WASM export (socket IPC dispatcher). */
    private WasmExport handleSockIpcFunc = null;

    // Interrupt system
    private static final int IRQ_NETWORK = 3;
    private static final int IRQ_MOUSE = 4;
    private final ConcurrentLinkedQueue<InterruptEvent> interruptQueue = new ConcurrentLinkedQueue<>();
    // One-slot coalescing for IRQ_NETWORK. Under a packet flood (e.g. a ping
    // across a switched computer in promiscuous mode) frames arrive at
    // 1000+/sec per NIC and the Rust IRQ handler drains every pending frame
    // in a single call — any second IRQ_NETWORK event is pure overhead.
    // Without this flag the unbounded interruptQueue grows without limit and
    // the drain loop in workerLoop starves input. See the plan file for the
    // full correctness argument.
    private final java.util.concurrent.atomic.AtomicBoolean networkIrqPending =
            new java.util.concurrent.atomic.AtomicBoolean(false);
    private volatile boolean wasmExecuting = false;
    /** Next interface to check first in net_rx_frame_any (round-robin). */
    private int rxRoundRobin = 0;

    // Mouse capture: feeds the WASI mouse_poll ring. Guest programs enable
    // capture via mouse_capture_start (requires displayMode >= 1); the client
    // Screen sends events into queueInterrupt(IRQ_MOUSE, ...) while enabled.
    private static final int MOUSE_EVENT_BYTES = 10;
    private static final int MOUSE_EVENT_RING_CAPACITY = 32;
    private final java.util.ArrayDeque<byte[]> mouseEventRing = new java.util.ArrayDeque<>();
    private volatile boolean mouseCaptureEnabled = false;

    public boolean isMouseCaptureEnabled() { return mouseCaptureEnabled; }

    /**
     * An interrupt event queued for delivery to WASM.
     * Payload is either a UTF-8 string (keyboard, redstone, terminate) or
     * raw bytes (mouse). Exactly one of {@code payload} / {@code binaryPayload}
     * is non-null.
     */
    private static class InterruptEvent {
        final int irq;
        final String payload;
        final byte[] binaryPayload;

        InterruptEvent(int irq, String payload) {
            this.irq = irq;
            this.payload = payload;
            this.binaryPayload = null;
        }

        InterruptEvent(int irq, byte[] binaryPayload) {
            this.irq = irq;
            this.payload = null;
            this.binaryPayload = binaryPayload;
        }

        byte[] asBytes() {
            return binaryPayload != null
                    ? binaryPayload
                    : payload.getBytes(StandardCharsets.UTF_8);
        }
    }

    // Counter for wasm-bindgen object reference handles
    private final java.util.concurrent.atomic.AtomicInteger nextObjectHandle = new java.util.concurrent.atomic.AtomicInteger(1);

    // Host function descriptors. Built once in createHostFunctions(), passed
    // to runtime.instantiate() in loadModule() so the module's imports get
    // bound to our handlers. Listed twice per name (with "env" module and
    // bare "") so guests that import either form match.
    private final List<WasmHostFunc> hostFunctions = new ArrayList<>();

    // Network: MAC addresses derived from computerId (one per face: down=0, up=1, north=2, south=3, west=4, east=5)
    private byte[][] networkMacs;

    public IComputerHost getHost() {
        return this.host;
    }

    /**
     * Hot-swap the host backing this running computer. Used by
     * {@code TerminalBlockEntity.adoptComputer} when a BE is being
     * transferred between positions (sable physics assembly, piston
     * push, {@code /setblock}) — the live WASM instance continues to
     * run, and subsequent host callbacks (redstone, world access)
     * land on the new BE.
     */
    public void setHost(IComputerHost host) {
        this.host = host;
    }

    public ComputerInstance(IComputerHost host, byte[][] macs) {
        this.host = host;
        // No Engine/Store here — the runtime is selected globally at server
        // start (WasmRuntimeRegistry.select()). Each WasmInstance is pinned
        // to the worker thread that calls its exports; cancellation goes
        // through WasmInstance.requestInterrupt().

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

        // Initialize WASI process manager with bridge for redstone/sleep host calls
        com.example.evanscomputermod.computer.wasi.ChildHostBridge childBridge =
                new com.example.evanscomputermod.computer.wasi.ChildHostBridge(this);
        this.processManager = new com.example.evanscomputermod.computer.wasi.ProcessManager(
                computerStoragePath, netIpcBridge, childBridge);
        // Child output/exit and new socket requests wake the kernel worker.
        this.processManager.setActivityListener(this::notifyChildActivity);
        this.netIpcBridge.setWakeListener(this::notifyChildActivity);

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

    // --- Shared-memory layout -------------------------------------------
    // Published by the kernel's abi_layout export right after
    // instantiation (see docs/refactor/ARCHITECTURE.md §4). The kernel
    // places these buffers as statics, so they never overlap its stack.
    // -1 until the layout has been read; every access is bounded by the
    // matching *Cap.
    private int inputBuf = -1, inputCap = 0;
    private int irqBuf = -1, irqCap = 0;
    private int fbBase = -1, fbCap = 0;
    private int gfxBase = -1, gfxCap = 0;
    private int screenBase = -1, screenCap = 0;
    /** Graphics palette offset from gfxBase / screenBase. */
    private static final int GFX_PALETTE_OFF = 0x40;
    /** Graphics pixel data offset from gfxBase / screenBase. */
    private static final int GFX_PIXEL_OFF = 0x400;
    /** Pixel format byte offset within the gfx header (0=indexed8, 1=rgba8888). */
    private static final int GFX_OFF_PIXEL_FORMAT = 0x10;

    /** Pixel format constants. Mirror screen.rs / TerminalDisplay. */
    public static final int PIXEL_FORMAT_INDEXED8 = 0;
    public static final int PIXEL_FORMAT_RGBA8888 = 1;


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
        // requests — keeping the WASM instance pinned to a single thread and
        // avoiding the cross-thread stall documented on writeScreenHeader.
        applyPendingScreenHeader();
        // Drain any pending gfx op staged by a WASI child thread (e.g. the
        // player pushing a decoded video frame).
        drainPendingGfxOps();
        if (fbBase < 0) return;
        try {
            int memSize = memory.size();
            if (memSize < fbBase + 16) return;

            boolean changed = false;

            int dirty = memory.readInt(fbBase + 0x0C);
            if (dirty != lastDirtyCounter) {
                lastDirtyCounter = dirty;
                changed = true;
            }

            if (memSize >= gfxBase + 16) {
                int palDirty = memory.readInt(gfxBase + 0x08);
                int pixDirty = memory.readInt(gfxBase + 0x0C);
                if (palDirty != lastPaletteDirtyCounter || pixDirty != lastPixelDirtyCounter) {
                    lastPaletteDirtyCounter = palDirty;
                    lastPixelDirtyCounter = pixDirty;
                    changed = true;
                }
            }

            if (memSize >= screenBase + 16) {
                int sMagic = (memory.readByte(screenBase) & 0xFF) | ((memory.readByte(screenBase + 1) & 0xFF) << 8);
                if (sMagic == 0xFB02) {
                    int sPalDirty = memory.readInt(screenBase + 0x08);
                    int sPixDirty = memory.readInt(screenBase + 0x0C);
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
        if (memory == null || fbBase < 0 || gfxBase < 0) return;
        IFramebufferDisplay display = host.getFramebufferDisplay();
        if (display == null) return;

        try {
            int memSize = memory.size();
            if (memSize < fbBase + 64) return;

            int width = (memory.readByte(fbBase + 2) & 0xFF) | ((memory.readByte(fbBase + 3) & 0xFF) << 8);
            int height = (memory.readByte(fbBase + 4) & 0xFF) | ((memory.readByte(fbBase + 5) & 0xFF) << 8);
            int totalSize = 64 + width * height * 4;

            if (memSize < fbBase + totalSize) return;

            byte[] fbData = memory.readBytes(fbBase, totalSize);
            display.setFromBytes(fbData);

            if (display instanceof TerminalDisplay td && memSize >= gfxBase + 64) {
                int gfxMagic = (memory.readByte(gfxBase) & 0xFF) | ((memory.readByte(gfxBase + 1) & 0xFF) << 8);
                if (gfxMagic == 0xFB02) {
                    int mode = memory.readByte(gfxBase + 2) & 0xFF;
                    if (mode > 0) {
                        int gfxW = (memory.readByte(gfxBase + 4) & 0xFF) | ((memory.readByte(gfxBase + 5) & 0xFF) << 8);
                        int gfxH = (memory.readByte(gfxBase + 6) & 0xFF) | ((memory.readByte(gfxBase + 7) & 0xFF) << 8);
                        int format = memory.readByte(gfxBase + GFX_OFF_PIXEL_FORMAT) & 0xFF;
                        int bpp = (format == PIXEL_FORMAT_RGBA8888) ? 4 : 1;
                        int gfxTotalSize = GFX_PIXEL_OFF + gfxW * gfxH * bpp;

                        if (memSize >= gfxBase + gfxTotalSize) {
                            byte[] gfxData = memory.readBytes(gfxBase, gfxTotalSize);
                            td.setGfxFromBytes(gfxData);
                        }
                    } else {
                        byte[] resetGfx = new byte[64];
                        resetGfx[0] = (byte) 0x02;
                        resetGfx[1] = (byte) 0xFB;
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
        if (memory == null || screenBase < 0) return;
        if (!(host instanceof TerminalBlockEntity tbe)) return;
        TerminalDisplay sd = tbe.getScreenDisplay();
        if (sd == null) return;

        try {
            int memSize = memory.size();
            if (memSize < screenBase + 64) return;

            int magic = (memory.readByte(screenBase) & 0xFF) | ((memory.readByte(screenBase + 1) & 0xFF) << 8);
            if (magic != 0xFB02) return;

            int mode = memory.readByte(screenBase + 2) & 0xFF;
            int gfxW = (memory.readByte(screenBase + 4) & 0xFF) | ((memory.readByte(screenBase + 5) & 0xFF) << 8);
            int gfxH = (memory.readByte(screenBase + 6) & 0xFF) | ((memory.readByte(screenBase + 7) & 0xFF) << 8);
            int format = memory.readByte(screenBase + GFX_OFF_PIXEL_FORMAT) & 0xFF;
            int bpp = (format == PIXEL_FORMAT_RGBA8888) ? 4 : 1;
            if (gfxW == 0 || gfxH == 0) return;
            int total = GFX_PIXEL_OFF + gfxW * gfxH * bpp;
            if (memSize < screenBase + total) return;

            byte[] gfxData = memory.readBytes(screenBase, total);
            sd.setGfxFromBytes(gfxData);
            if (mode == 0) sd.setDisplayMode(1);
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
     * Minecraft's neighbor-update cascade on any nearby block change). Touching
     * the WASM instance from the server thread while the worker thread is
     * mid-WASM-call would race on the instance's owning thread. Instead, stash
     * the desired header into volatile fields; the worker thread applies it in
     * {@link #applyPendingScreenHeader}, called from
     * {@link #checkFramebufferDirty} (every worker loop iteration + every
     * hostSleepMs chunk).
     */
    public void writeScreenHeader(int gfxWidth, int gfxHeight) {
        pendingScreenHeaderWidth = gfxWidth;
        pendingScreenHeaderHeight = gfxHeight;
        hasPendingScreenHeaderWrite = true;
    }

    /**
     * Drain the pending screen-header request set by the server thread. Must
     * only be called on the worker thread.
     */
    private void applyPendingScreenHeader() {
        if (!hasPendingScreenHeaderWrite) return;
        int gfxWidth = pendingScreenHeaderWidth;
        int gfxHeight = pendingScreenHeaderHeight;
        hasPendingScreenHeaderWrite = false;
        if (memory == null || screenBase < 0) return;
        try {
            if (memory.size() < screenBase + 64) return;
            memory.writeByte(screenBase,     (byte) 0x02);
            memory.writeByte(screenBase + 1, (byte) 0xFB);
            memory.writeByte(screenBase + 2, (byte) (gfxWidth > 0 && gfxHeight > 0 ? 1 : 0));
            memory.writeByte(screenBase + 3, (byte) 0);
            memory.writeByte(screenBase + 4, (byte) (gfxWidth & 0xFF));
            memory.writeByte(screenBase + 5, (byte) ((gfxWidth >> 8) & 0xFF));
            memory.writeByte(screenBase + 6, (byte) (gfxHeight & 0xFF));
            memory.writeByte(screenBase + 7, (byte) ((gfxHeight >> 8) & 0xFF));
            memory.writeByte(screenBase + GFX_OFF_PIXEL_FORMAT, (byte) PIXEL_FORMAT_INDEXED8);
            // leave dirty counters alone; Rust OS writes them
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Error writing screen header", e);
        }
    }

    private WasmExport getHandleSockIpcFunc() {
        if (handleSockIpcFunc == null && instance != null) {
            handleSockIpcFunc = instance.export("handle_sock_ipc");
        }
        return handleSockIpcFunc;
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
     * Worker thread main loop.
     *
     * <p>The kernel never blocks, so this loop owns all waiting. It sleeps
     * until the kernel's next deadline (returned by {@code on_tick}) or until
     * an event wakes it: keyboard input, an interrupt (network frames,
     * Ctrl+T), child output/exit, or a child socket request. Then it
     * delivers the events, ticks the kernel and retries pending socket
     * requests. Kernel exports are only ever called from this thread, one at
     * a time, and no host function calls back into the kernel.
     */
    private void workerLoop() {
        EvansComputerMod.LOGGER.debug("WASM worker thread started");
        long nextDeadline = 0; // tick right after boot

        while (!shutdownRequested && !Thread.currentThread().isInterrupted()) {
            try {
                if (instance == null || faulted || memory == null) {
                    waitForWork(100);
                    continue;
                }
                boolean events = drainAndDeliverInterrupts();

                String input;
                while ((input = inputQueue.poll()) != null) {
                    processInputOnWorker(input);
                    events = true;
                }

                boolean childEvent = childActivity.getAndSet(false);
                long now = System.currentTimeMillis();
                if (events || childEvent || now >= nextDeadline || netIpcBridge.hasPending()) {
                    nextDeadline = kernelTick(now);
                }

                // Socket syscalls: new requests plus retries of requests the
                // kernel answered IPC_PENDING. They only become ready after
                // a frame, a tick or a new request -- all of which wake us.
                if (netIpcBridge.hasPending()) {
                    WasmExport sockIpc = getHandleSockIpcFunc();
                    if (sockIpc != null && netIpcBridge.servicePending(sockIpc) > 0) {
                        // Completed requests may have queued frames/timers.
                        nextDeadline = kernelTick(System.currentTimeMillis());
                    }
                }

                checkFramebufferDirty();

                long wait = nextDeadline < 0
                        ? 100
                        : Math.min(100, Math.max(0, nextDeadline - System.currentTimeMillis()));
                waitForWork(wait);
            } catch (InterruptedException e) {
                Thread.interrupted();
                if (shutdownRequested) {
                    break;
                }
            } catch (Throwable e) {
                EvansComputerMod.LOGGER.error("Error in WASM worker thread", e);
            }
        }

        EvansComputerMod.LOGGER.debug("WASM worker thread exiting");
    }

    /** Sleep until woken or {@code ms} elapse, unless work is already queued. */
    private void waitForWork(long ms) throws InterruptedException {
        if (ms <= 0) return;
        synchronized (workerWakeSignal) {
            if (inputQueue.isEmpty() && interruptQueue.isEmpty() && !childActivity.get()
                    && !netIpcBridge.hasNewRequests() && !shutdownRequested) {
                workerWakeSignal.wait(ms);
            }
        }
    }

    /** Set by child processes (output written, exit) and the IPC bridge. */
    private final java.util.concurrent.atomic.AtomicBoolean childActivity =
            new java.util.concurrent.atomic.AtomicBoolean(false);

    /** Called from child threads: output available / exited / new socket request. */
    public void notifyChildActivity() {
        childActivity.set(true);
        wakeWorker();
    }

    /** Wall-clock start of the kernel call in progress (for Ctrl+T). */
    private volatile long kernelCallStartMs = 0;
    /** Kernel calls running longer than this are considered stuck. */
    private static final long STUCK_KERNEL_CALL_MS = 250;

    /**
     * Call a kernel export. A trap caused by a host-requested interrupt
     * (Ctrl+T on a stuck call) is recovered via {@code kernel_recover}; any
     * other trap marks the computer faulted.
     */
    private long[] callKernel(String name, long... args) {
        WasmExport fn = instance.export(name);
        if (fn == null) return null;
        wasmExecuting = true;
        kernelCallStartMs = System.currentTimeMillis();
        try {
            return fn.call(args);
        } catch (WasmTrap e) {
            if (e.kind() == WasmTrap.Kind.INTERRUPTED || interrupted) {
                recoverKernel();
            } else {
                faulted = true;
                EvansComputerMod.LOGGER.error("Kernel trap in {}: {}", name, e.kind(), e);
            }
            return null;
        } catch (Throwable e) {
            if (interrupted) {
                recoverKernel();
            } else {
                faulted = true;
                EvansComputerMod.LOGGER.error("Kernel error in {}", name, e);
            }
            return null;
        } finally {
            wasmExecuting = false;
        }
    }

    private void recoverKernel() {
        EvansComputerMod.LOGGER.info("Kernel call interrupted; recovering to shell");
        interrupted = false;
        instance.clearInterrupt();
        Thread.interrupted();
        WasmExport rec = instance.export("kernel_recover");
        if (rec != null) {
            try {
                rec.call();
            } catch (Throwable t) {
                faulted = true;
                EvansComputerMod.LOGGER.error("kernel_recover failed", t);
            }
        }
    }

    /** Tick the kernel. Returns the next absolute deadline, or -1. */
    private long kernelTick(long now) {
        long[] r = callKernel("on_tick", now);
        return (r == null || r.length == 0) ? now + 100 : r[0];
    }

    /** Deliver keyboard input through the kernel's input region. */
    private void processInputOnWorker(String input) {
        if (instance == null || faulted || inputBuf < 0) {
            return;
        }
        // Fresh keystroke: a previous Ctrl+T abort is resolved by now.
        childAbortRequested = false;
        byte[] bytes = input.getBytes(StandardCharsets.UTF_8);
        for (int off = 0; off < bytes.length; off += inputCap) {
            int n = Math.min(inputCap, bytes.length - off);
            memory.writeBytes(inputBuf, bytes, off, n);
            callKernel("on_input", inputBuf, n);
            if (faulted) return;
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
        if (irq == IRQ_NETWORK) {
            // At most one pending IRQ_NETWORK in flight. The Rust handler
            // drains every frame in a single invocation, so a second event
            // would always find nothing to do. The flag is cleared before
            // delivery (see drainAndDeliverInterrupts / hostInterruptPoll)
            // so any frame arriving during the handler's drain still
            // successfully queues the next event.
            if (!networkIrqPending.compareAndSet(false, true)) {
                return;
            }
        }
        interruptQueue.offer(new InterruptEvent(irq, payload));
        wakeWorker();
    }

    /**
     * Binary-payload variant of {@link #queueInterrupt(int, String)}. Used by
     * IRQ_MOUSE (4) where the payload is a fixed-shape 10-byte little-endian
     * struct, not UTF-8 text.
     *
     * <p>For IRQ_MOUSE, this also appends the event to the WASI mouse-poll ring
     * (capacity 32, drop-oldest) so WASI children can read events via
     * {@code mouse_poll}.
     */
    public void queueInterrupt(int irq, byte[] payload) {
        if (irq == IRQ_MOUSE && payload != null && payload.length >= MOUSE_EVENT_BYTES) {
            synchronized (mouseEventRing) {
                if (mouseEventRing.size() >= MOUSE_EVENT_RING_CAPACITY) {
                    mouseEventRing.pollFirst();
                }
                byte[] copy = new byte[MOUSE_EVENT_BYTES];
                System.arraycopy(payload, 0, copy, 0, MOUSE_EVENT_BYTES);
                mouseEventRing.addLast(copy);
            }
        }
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
     * Drains the interrupt queue and delivers each event via on_interrupt().
     * Worker thread only. Returns true if anything was delivered.
     */
    private boolean drainAndDeliverInterrupts() {
        if (instance == null || faulted || irqBuf < 0) return false;
        boolean any = false;
        InterruptEvent evt;
        while ((evt = interruptQueue.poll()) != null) {
            if (evt.irq == IRQ_NETWORK) {
                // Clear the coalescing flag *before* delivery so frames that
                // arrive while the kernel drains can queue the next event.
                networkIrqPending.set(false);
            }
            byte[] payload = evt.asBytes();
            int n = Math.min(payload.length, irqCap);
            if (n > 0) memory.writeBytes(irqBuf, payload, 0, n);
            callKernel("on_interrupt", evt.irq, irqBuf, n);
            any = true;
            if (faulted) break;
        }
        return any;
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
     * Creates all host functions and adds them to {@link #hostFunctions}.
     * The list is passed to {@code runtime.instantiate()} in {@link #loadModule}
     * to bind to the module's declared imports. Each entry is registered under
     * both module name {@code "env"} and an empty module name, so guest modules
     * that import either form match.
     */
    private void createHostFunctions() {
        // === Display sync ===
        hh("fb_sync", NIL, NIL, (inst, args) -> {
            checkInterrupted();
            long now = System.currentTimeMillis();
            if (now - lastFbSyncMs >= FB_SYNC_MIN_INTERVAL_MS) {
                lastFbSyncMs = now;
                readFramebufferFromWasm();
                needsSync = true;
            }
            return null;
        });

        // === Screen (in-world cluster) host functions ===

        hh("screen_is_attached", NIL, RET_I32, (inst, args) -> {
            boolean attached = (host instanceof TerminalBlockEntity tbe) && tbe.hasScreenCluster();
            return retI32(attached ? 1 : 0);
        });

        hh("screen_get_gfx_width", NIL, RET_I32, (inst, args) -> {
            int w = 0;
            if (host instanceof TerminalBlockEntity tbe) {
                TerminalBlockEntity.ScreenClusterInfo info = tbe.getScreenClusterInfo();
                if (info != null) w = info.gfxWidth();
            }
            return retI32(w);
        });

        hh("screen_get_gfx_height", NIL, RET_I32, (inst, args) -> {
            int h = 0;
            if (host instanceof TerminalBlockEntity tbe) {
                TerminalBlockEntity.ScreenClusterInfo info = tbe.getScreenClusterInfo();
                if (info != null) h = info.gfxHeight();
            }
            return retI32(h);
        });

        hh("screen_fb_sync", NIL, NIL, (inst, args) -> {
            checkInterrupted();
            long now = System.currentTimeMillis();
            if (now - lastScreenFbSyncMs >= FB_SYNC_MIN_INTERVAL_MS) {
                lastScreenFbSyncMs = now;
                readScreenFramebufferFromWasm();
                needsSync = true;
            }
            return null;
        });

        hh("screen_set_power", I, NIL, (inst, args) -> {
            int on = (int) args[0];
            if (host instanceof TerminalBlockEntity tbe) {
                tbe.setScreenPower(on != 0);
            }
            return null;
        });

        hh("screen_set_pixel_format", I, NIL, (inst, args) -> {
            int format = (int) args[0];
            if (format != PIXEL_FORMAT_INDEXED8 && format != PIXEL_FORMAT_RGBA8888) return null;
            applyGfxSetPixelFormat(screenBase, format);
            return null;
        });

        // === File system host functions ===

        hh("file_write", IIII, RET_I32, (inst, args) ->
                retI32(hostFileWrite((int) args[0], (int) args[1], (int) args[2], (int) args[3])));

        hh("file_read", IIII, RET_I32, (inst, args) ->
                retI32(hostFileRead((int) args[0], (int) args[1], (int) args[2], (int) args[3])));

        hh("file_size", II, RET_I32, (inst, args) ->
                retI32(hostFileSize((int) args[0], (int) args[1])));

        hh("file_exists", II, RET_I32, (inst, args) ->
                retI32(hostFileExists((int) args[0], (int) args[1])));

        hh("file_delete", II, RET_I32, (inst, args) ->
                retI32(hostFileDelete((int) args[0], (int) args[1])));

        hh("file_list", II, RET_I32, (inst, args) ->
                retI32(hostFileList((int) args[0], (int) args[1])));

        hh("file_mkdir", II, RET_I32, (inst, args) ->
                retI32(hostFileMkdir((int) args[0], (int) args[1])));

        hh("file_is_dir", II, RET_I32, (inst, args) ->
                retI32(hostFileIsDir((int) args[0], (int) args[1])));

        hh("file_list_dir", IIII, RET_I32, (inst, args) ->
                retI32(hostFileListDir((int) args[0], (int) args[1], (int) args[2], (int) args[3])));

        // === Redstone ===

        hh("redstone_set_output", II, RET_I32, (inst, args) ->
                retI32(hostRedstoneSetOutput((int) args[0], (int) args[1])));

        hh("redstone_get_input", I, RET_I32, (inst, args) ->
                retI32(hostRedstoneGetInput((int) args[0])));

        hh("redstone_get_all_input", I, RET_I32, (inst, args) ->
                retI32(hostRedstoneGetAllInput((int) args[0])));

        // === Time / entropy ===
        // The kernel never sleeps or reads lines itself any more: waiting is
        // done by the worker loop (on_tick deadlines), input arrives via
        // on_input, and interrupts via on_interrupt.

        hh("get_time_ms", NIL, RET_I64, (inst, args) ->
                retI64(System.currentTimeMillis()));

        hh("__getrandom_v03_custom", II, RET_I32, (inst, args) ->
                retI32(hostGetrandom((int) args[0], (int) args[1])));

        // === Visual Editor ===

        hh("open_visual_editor", NIL, NIL, (inst, args) -> {
            checkInterrupted();
            if (host.getVisualProgramming() != null) {
                host.getVisualProgramming().openVisualEditor();
            }
            return null;
        });

        // === Network host functions (multi-interface) ===

        hh("net_get_interface_count", NIL, RET_I32, (inst, args) ->
                retI32(networkMacs.length));

        hh("net_get_interface_mac", II, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int bufPtr = (int) args[1];
            if (index < 0 || index >= networkMacs.length) return retI32(-1);
            writeBytesToMemory(networkMacs[index], bufPtr, 6);
            return retI32(6);
        });

        hh("net_tx_frame_on", III, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int bufPtr = (int) args[1];
            int frameLen = (int) args[2];
            if (index < 0 || index >= networkMacs.length || frameLen < 14 || frameLen > 1518) {
                return retI32(-1);
            }
            byte[] frame = readBytesFromMemory(bufPtr, frameLen);
            if (frame.length == 0) return retI32(-1);
            NetworkHub hub = NetworkHub.getInstance();
            if (hub != null) hub.transmit(networkMacs[index], frame);
            return retI32(0);
        });

        hh("net_rx_frame_on", III, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int bufPtr = (int) args[1];
            int bufLen = (int) args[2];
            if (index < 0 || index >= networkMacs.length) return retI32(-1);
            NetworkHub hub = NetworkHub.getInstance();
            if (hub == null) return retI32(-1);
            byte[] frame = hub.receive(networkMacs[index]);
            if (frame == null) return retI32(-1);
            int writeLen = Math.min(frame.length, bufLen);
            writeBytesToMemory(frame, bufPtr, writeLen);
            return retI32(writeLen);
        });

        hh("net_rx_frame_any", III, RET_I32, (inst, args) -> {
            int bufPtr = (int) args[0];
            int bufLen = (int) args[1];
            int ifaceIdxPtr = (int) args[2];
            NetworkHub hub = NetworkHub.getInstance();
            if (hub == null) return retI32(-1);
            // Round-robin across interfaces so one busy port can't starve
            // the others (a switch drains many frames per interrupt).
            int n = networkMacs.length;
            for (int k = 0; k < n; k++) {
                int i = (rxRoundRobin + k) % n;
                byte[] frame = hub.receive(networkMacs[i]);
                if (frame != null) {
                    rxRoundRobin = (i + 1) % n;
                    int writeLen = Math.min(frame.length, bufLen);
                    writeBytesToMemory(frame, bufPtr, writeLen);
                    memory.writeInt(ifaceIdxPtr, i);
                    return retI32(writeLen);
                }
            }
            return retI32(-1);
        });

        // Carrier: 1 if the face has a cable attached to a network segment
        // and has not been administratively disabled.
        hh("net_get_link_state", I, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            if (index < 0 || index >= networkMacs.length) return retI32(0);
            NetworkHub hub = NetworkHub.getInstance();
            return retI32(hub != null && hub.hasCarrier(networkMacs[index]) ? 1 : 0);
        });

        hh("net_set_promiscuous_on", II, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int enabled = (int) args[1];
            if (index < 0 || index >= networkMacs.length) return retI32(-1);
            NetworkHub hub = NetworkHub.getInstance();
            if (hub != null) hub.setPromiscuous(networkMacs[index], enabled != 0);
            return retI32(0);
        });

        hh("net_pcap_enable", II, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int enabled = (int) args[1];
            if (index < 0 || index >= networkMacs.length) return retI32(-1);
            NetworkHub hub = NetworkHub.getInstance();
            if (hub != null) hub.setPcapEnabled(networkMacs[index], enabled != 0);
            return retI32(0);
        });

        hh("net_pcap_rx", III, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int bufPtr = (int) args[1];
            int bufLen = (int) args[2];
            if (index < 0 || index >= networkMacs.length) return retI32(-1);
            NetworkHub hub = NetworkHub.getInstance();
            if (hub == null) return retI32(-1);
            byte[] frame = hub.pcapReceive(networkMacs[index]);
            if (frame == null) return retI32(-1);
            int writeLen = Math.min(frame.length, bufLen);
            writeBytesToMemory(frame, bufPtr, writeLen);
            return retI32(writeLen);
        });

        hh("net_set_link_state", II, RET_I32, (inst, args) -> {
            int index = (int) args[0];
            int up = (int) args[1];
            if (index < 0 || index >= networkMacs.length) return retI32(-1);
            NetworkHub linkHub = NetworkHub.getInstance();
            if (linkHub != null) linkHub.setLinkEnabled(networkMacs[index], up != 0);
            if (host instanceof TerminalBlockEntity tbe) {
                if (index < 6) {
                    tbe.setFaceDisabled(index, up == 0);
                    var server = host.getServer();
                    if (server != null) {
                        server.execute(() -> tbe.updateDisabledFaces(index, up != 0));
                    }
                }
            }
            return retI32(0);
        });

        // === Kernel extension stubs (process management, FDs, sockets, TTY) ===
        createKernelExtensionStubs();


        EvansComputerMod.LOGGER.debug("Created {} host function entries", hostFunctions.size());
    }

    // === Host-function registration helpers ===

    /** Param/result lists used by host function declarations. */
    private static final List<WasmValType> NIL = List.of();
    private static final List<WasmValType> I = List.of(WasmValType.I32);
    private static final List<WasmValType> II = List.of(WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> III = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> IIII = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> I8 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32,
                                                        WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> RET_I32 = List.of(WasmValType.I32);
    private static final List<WasmValType> RET_I64 = List.of(WasmValType.I64);
    private static final List<WasmValType> RET_F64 = List.of(WasmValType.F64);

    /** Wrap an i32 return. */
    private static long[] retI32(int v) { return WasmHostFunc.retI32(v); }

    /** Wrap an i64 return. */
    private static long[] retI64(long v) { return WasmHostFunc.retI64(v); }

    /**
     * Creates kernel-extension host functions (FD ops, process management,
     * TTY, sockets). Some are full implementations (process_*); others are
     * no-op stubs that return -1 to indicate "not implemented" so the WASM
     * module can load.
     */
    private void createKernelExtensionStubs() {
        // FD operations — stubs returning -1
        addStubI32_3("fd_open");
        addStubI32_3("fd_read");
        addStubI32_3("fd_write");
        addStubI32_1("fd_close");
        addStubI32_2("pipe_create");

        // process_spawn(path_ptr, path_len, argv_ptr, argv_len, stdin_fd, stdout_fd, stderr_fd) -> i32
        hh("process_spawn",
                List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32,
                        WasmValType.I32, WasmValType.I32, WasmValType.I32),
                RET_I32, (inst, args) -> {
            String path = readStringFromMemory((int) args[0], (int) args[1]);
            String argvStr = readStringFromMemory((int) args[2], (int) args[3]);
            if (path == null) return retI32(-1);
            MountedPath mp = resolveReadPath(path);
            if (mp == null || !java.nio.file.Files.exists(mp.realPath)) {
                EvansComputerMod.LOGGER.debug("process_spawn: file not found: {}", path);
                return retI32(-1);
            }
            String[] argv = argvStr != null ? argvStr.split("\n") : new String[]{path};
            int termW = (memory.readByte(fbBase + 2) & 0xFF) | ((memory.readByte(fbBase + 3) & 0xFF) << 8);
            int termH = (memory.readByte(fbBase + 4) & 0xFF) | ((memory.readByte(fbBase + 5) & 0xFF) << 8);
            var env = java.util.Map.of("COLUMNS", String.valueOf(termW), "LINES", String.valueOf(termH));
            int pid = processManager.spawn(mp.realPath, argv, env);
            return retI32(pid);
        });

        // process_try_wait(pid, code_ptr) -> 1 exited (code written), 0 running, -1 unknown
        hh("process_try_wait", II, RET_I32, (inst, args) -> {
            Integer code = processManager.tryWait((int) args[0]);
            if (code == null) return retI32(0);
            if (code == Integer.MIN_VALUE) return retI32(-1);
            memory.writeInt((int) args[1], code);
            return retI32(1);
        });

        // process_read_output(pid, buf, len) -> bytes (0 none), -1 unknown pid
        hh("process_read_output", III, RET_I32, (inst, args) -> {
            int len = Math.max(0, Math.min((int) args[2], 65536));
            byte[] buf = new byte[len];
            int n = processManager.readOutput((int) args[0], buf);
            if (n > 0) memory.writeBytes((int) args[1], buf, 0, n);
            return retI32(n);
        });

        // process_write_input(pid, buf, len) -> bytes accepted, -1 unknown pid
        hh("process_write_input", III, RET_I32, (inst, args) -> {
            int len = Math.max(0, Math.min((int) args[2], 65536));
            byte[] data = memory.readBytes((int) args[1], len);
            return retI32(processManager.writeInput((int) args[0], data));
        });

        hh("process_kill", II, RET_I32, (inst, args) ->
                retI32(processManager.kill((int) args[0])));

        hh("process_list", II, RET_I32, (inst, args) -> {
            String json = processManager.listProcesses();
            return retI32(writeStringToMemory(json, (int) args[0], (int) args[1]));
        });

        hh("process_state", I, RET_I32, (inst, args) -> {
            var state = processManager.getState((int) args[0]);
            return retI32(switch (state) {
                case RUNNING -> 0;
                case ZOMBIE -> 2;
            });
        });

        // TTY management (mostly stubs)
        addStubI32_2("tty_create");
        addStubI32_2("tty_attach_fd");
        addStubI32_1("tty_set_foreground");

        // tty_get_size: reads dims from the framebuffer header
        hh("tty_get_size", III, RET_I32, (inst, args) -> {
            int widthPtr = (int) args[1];
            int heightPtr = (int) args[2];
            int w = (memory.readByte(fbBase + 2) & 0xFF) | ((memory.readByte(fbBase + 3) & 0xFF) << 8);
            int h = (memory.readByte(fbBase + 4) & 0xFF) | ((memory.readByte(fbBase + 5) & 0xFF) << 8);
            memory.writeInt(widthPtr, w);
            memory.writeInt(heightPtr, h);
            return retI32(0);
        });

        EvansComputerMod.LOGGER.debug("Created kernel extension stub host functions");
    }

    private void hh(String name, List<WasmValType> p, List<WasmValType> r, WasmHostFunc.Handler h) {
        hostFunctions.add(new WasmHostFunc("env", name, p, r, h));
        hostFunctions.add(new WasmHostFunc("", name, p, r, h));
    }

    /** Stub: (i32) -> i32, returns -1. */
    private void addStubI32_1(String name) {
        hh(name, I, RET_I32, (inst, args) -> retI32(-1));
    }

    /** Stub: (i32, i32) -> i32, returns -1. */
    private void addStubI32_2(String name) {
        hh(name, II, RET_I32, (inst, args) -> retI32(-1));
    }

    /** Stub: (i32, i32, i32) -> i32, returns -1. */
    private void addStubI32_3(String name) {
        hh(name, III, RET_I32, (inst, args) -> retI32(-1));
    }

    /** Stub: takes the given param types, returns void. */
    private void addStubVoid(String name, WasmValType... paramTypes) {
        hh(name, List.of(paramTypes), NIL, (inst, args) -> null);
    }

    /** Stub: takes the given param types, returns i32(0). */
    private void addStubI32Return(String name, WasmValType... paramTypes) {
        addStubI32ReturnValue(name, 0, paramTypes);
    }

    /** Stub: takes the given param types, returns i32({@code value}). */
    private void addStubI32ReturnValue(String name, int value, WasmValType... paramTypes) {
        hh(name, List.of(paramTypes), RET_I32, (inst, args) -> retI32(value));
    }

    /**
     * Provides random bytes for getrandom 0.3. Writes {@code len} random
     * bytes into WASM memory at {@code ptr}. Returns 0 on success, -1 on
     * error / oversized request.
     */
    private int hostGetrandom(int ptr, int len) {
        if (memory == null || len <= 0 || len > 4096) {
            return -1;
        }
        try {
            byte[] bytes = new byte[len];
            com.example.evanscomputermod.computer.wasi.Entropy.fill(bytes);
            memory.writeBytes(ptr, bytes);
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error in hostGetrandom", e);
            return -1;
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
            return memory.readString(ptr, len);
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
            byte[] data = memory.readBytes(dataPtr, dataLen);

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
            memory.writeBytes(bufPtr, data, 0, bytesToRead);
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
            for (int relativeSide = 0; relativeSide < 6; relativeSide++) {
                Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
                int power = provider.getRedstoneInput(absoluteDir.ordinal());
                memory.writeInt(bufPtr + relativeSide * 4, power);
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

    public void bridgeSleepMs(int ms) {
        // NOTE: Do NOT call hostSleepMs here. hostSleepMs accesses the kernel's
        // wasm instance (via checkFramebufferDirty -> memory.read*) which
        // is not thread-safe — calling it from a child WASI thread deadlocks
        // wasmtime's internal locks. Use a plain Thread.sleep instead.
        int clamped = Math.max(0, Math.min(60_000, ms));
        if (clamped == 0) return;
        try {
            Thread.sleep(clamped);
        } catch (InterruptedException e) {
            // Re-assert the flag and throw so the calling host function
            // traps the WASM call and the child exits cleanly — same
            // semantics as NetIpcBridge.callBlocking. Without this, a
            // child in a sleep loop (e.g. ping in its 1-second wait, or
            // any background daemon) cannot be killed via Ctrl+T, and
            // the worker thread spins generating GC pressure.
            Thread.currentThread().interrupt();
            throw new RuntimeException("WASI child sleep interrupted", e);
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
    public int bridgeVideoOpen(String vfsPath, int targetW, int targetH, int pixelFormat) {
        if (childAbortRequested) return -1;
        if (vfsPath == null || vfsPath.isEmpty()) {
            EvansComputerMod.LOGGER.info("bridgeVideoOpen: rejected empty path");
            return -1;
        }
        if (targetW <= 0 || targetH <= 0) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): rejected size {}x{}", vfsPath, targetW, targetH);
            return -1;
        }
        if (pixelFormat != PIXEL_FORMAT_INDEXED8 && pixelFormat != PIXEL_FORMAT_RGBA8888) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): unsupported pixel format {}", vfsPath, pixelFormat);
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
        int decoderFormat = (pixelFormat == PIXEL_FORMAT_RGBA8888)
                ? com.example.evanscomputermod.computer.video.VideoDecoder.FORMAT_RGBA8888
                : com.example.evanscomputermod.computer.video.VideoDecoder.FORMAT_INDEXED8;
        int handle = videoRegistry.open(mp.realPath, targetW, targetH, decoderFormat);
        if (handle < 0) {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): decoder rejected {} ({}x{} fmt={}) — see previous log line",
                    vfsPath, mp.realPath.toAbsolutePath(), targetW, targetH, pixelFormat);
        } else {
            EvansComputerMod.LOGGER.info(
                    "bridgeVideoOpen({}): handle={} path={} size={}x{} fmt={}",
                    vfsPath, handle, mp.realPath.toAbsolutePath(), targetW, targetH, pixelFormat);
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
        if (childAbortRequested) return -2L;
        var d = videoRegistry.get(handle);
        if (d == null) return -2L;
        // For the screen target, bail early if no cluster is attached
        // rather than staging a frame that nothing would consume.
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -2L;
        try {
            var frame = d.next();
            if (frame == null) return -1L;
            // Route by decoder output format. The frame's `indexed` field
            // is the raw pixel bytes — interpretation depends on the
            // format the decoder was opened with.
            if (d.outputFormat() == com.example.evanscomputermod.computer.video.VideoDecoder.FORMAT_RGBA8888) {
                stageGfxOpFrameRgba(target, d.targetWidth(), d.targetHeight(), frame.indexed);
            } else {
                stageGfxOpFrame(target, d.targetWidth(), d.targetHeight(), frame.indexed);
            }
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
        if (childAbortRequested) return -1;
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
        if (childAbortRequested) return -1;
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
     * Switch the selected display's pixel format
     * ({@link #PIXEL_FORMAT_INDEXED8} or {@link #PIXEL_FORMAT_RGBA8888}).
     * The worker drains by writing the format byte into kernel WASM
     * memory, zeroing the pixel region for the new format's byte
     * count, and bumping both dirty counters so the next read picks
     * up the new layout.
     */
    public int bridgeGfxSetPixelFormat(int target, int format) {
        if (childAbortRequested) return -1;
        if (format != PIXEL_FORMAT_INDEXED8 && format != PIXEL_FORMAT_RGBA8888) return -1;
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -1;
        try {
            stageGfxOpSetPixelFormat(target, format);
            return 0;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }
    }

    /**
     * Push a full RGBA8888 frame into the selected display's pixel
     * region. Pixel byte count must equal {@code w*h*4}; the format
     * byte should already be RGBA8888 (callers should call
     * {@link #bridgeGfxSetPixelFormat} first).
     */
    public int bridgeGfxFrameRgba(int target, int w, int h, byte[] rgba) {
        if (childAbortRequested) return -1;
        if (w <= 0 || h <= 0 || w > 4096 || h > 4096) return -1;
        if (rgba == null || rgba.length < w * h * 4) return -1;
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -1;
        try {
            stageGfxOpFrameRgba(target, w, h, rgba);
            return 0;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }
    }

    /**
     * Copy a rectangle of pixels read from child linear memory into the
     * selected framebuffer. Unlike {@link #bridgeGfxFrameRgba}, this
     * addresses a sub-rectangle, so WASI children can do dirty-rect
     * updates or incremental drawing without pushing the whole frame.
     * See {@link #applyGfxBlitRect} for the format/clamping contract.
     */
    public int bridgeGfxBlitRect(int target, int x, int y, int w, int h, byte[] pixels, int format) {
        if (childAbortRequested) return -1;
        if (w <= 0 || h <= 0 || w > 4096 || h > 4096) return -1;
        if (x < 0 || y < 0) return -1;
        if (format != PIXEL_FORMAT_INDEXED8 && format != PIXEL_FORMAT_RGBA8888) return -1;
        int bpp = (format == PIXEL_FORMAT_RGBA8888) ? 4 : 1;
        if (pixels == null || pixels.length < w * h * bpp) return -1;
        if (target == GFX_TARGET_SCREEN && !hasAttachedScreen()) return -1;
        try {
            stageGfxOpBlit(target, x, y, w, h, pixels, format);
            return 0;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }
    }

    /**
     * Enable mouse capture. Only succeeds when the terminal host is in
     * graphics mode (display mode &gt;= 1) — capture would have no
     * coherent meaning in text mode, since there's no framebuffer pixel
     * the cursor position would map to. Returns 1 on success, 0 on
     * ineligible/invalid state.
     */
    public int bridgeMouseCaptureStart() {
        if (childAbortRequested) return 0;
        if (host instanceof com.example.evanscomputermod.block.TerminalBlockEntity tbe) {
            if (tbe.getDisplay().getDisplayMode() < 1) return 0;
        } else {
            return 0;
        }
        mouseCaptureEnabled = true;
        return 1;
    }

    /**
     * Disable mouse capture and drain any pending events. Subsequent
     * {@link #bridgeMousePoll} calls will return 0 until the child
     * re-enables capture.
     */
    public void bridgeMouseCaptureStop() {
        mouseCaptureEnabled = false;
        synchronized (mouseEventRing) {
            mouseEventRing.clear();
        }
    }

    public int bridgeMouseCaptureIsActive() {
        return mouseCaptureEnabled ? 1 : 0;
    }

    /**
     * Pop one mouse event (10 bytes LE) off the ring into child linear
     * memory at {@code bytes}. Returns 1 if an event was written, 0 if
     * the ring was empty. The caller supplies a 10-byte child buffer.
     */
    // --- Peripheral bridge (child WASI programs, see ecm_host_abi::peripheral) ---

    public com.example.evanscomputermod.computer.peripheral.PeripheralEventBus getPeripheralEvents() {
        return peripheralEvents;
    }

    /** Encoded result frame: LIST of [name, type]. */
    public byte[] bridgePeripheralList() {
        IComputerHost h = host;
        var hub = h != null ? h.getPeripheralHub() : null;
        return hub != null ? hub.list()
                : com.example.evanscomputermod.computer.peripheral.PeripheralValues.ok(java.util.List.of());
    }

    /** Encoded result frame: [type, [methods]] or an error. */
    public byte[] bridgePeripheralMethods(String name) {
        IComputerHost h = host;
        var hub = h != null ? h.getPeripheralHub() : null;
        return hub != null ? hub.methods(name)
                : com.example.evanscomputermod.computer.peripheral.PeripheralValues.error("no peripheral named '" + name + "'");
    }

    /** Encoded result frame of calling {@code method} on peripheral {@code name}. Blocks the calling child thread. */
    public byte[] bridgePeripheralCall(String name, String method, byte[] args) {
        IComputerHost h = host;
        var hub = h != null ? h.getPeripheralHub() : null;
        if (hub == null) {
            return com.example.evanscomputermod.computer.peripheral.PeripheralValues.error("no peripheral named '" + name + "'");
        }
        return hub.call(name, method, args, h.getServer());
    }

    public int bridgeMousePoll(byte[] bytes) {
        if (bytes == null || bytes.length < MOUSE_EVENT_BYTES) return 0;
        byte[] evt;
        synchronized (mouseEventRing) {
            evt = mouseEventRing.pollFirst();
        }
        if (evt == null) return 0;
        System.arraycopy(evt, 0, bytes, 0, MOUSE_EVENT_BYTES);
        return 1;
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
        if (childAbortRequested) return -1L;
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
        // Always allow power-OFF through, even during child abort — it's
        // the right cleanup direction. Only short-circuit power-ON so a
        // dying player can't flash the cluster back on after Ctrl+T.
        if (childAbortRequested && on) return;
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
        // Leaving graphics mode on the terminal target invalidates any
        // active mouse capture — there's no framebuffer for the cursor
        // to map into, and the WASI child that enabled capture is likely
        // exiting anyway. Drop the flag and drain the ring so a later
        // program starts clean.
        if (target == GFX_TARGET_TERMINAL && mode == 0 && mouseCaptureEnabled) {
            mouseCaptureEnabled = false;
            synchronized (mouseEventRing) {
                mouseEventRing.clear();
            }
        }
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

    private void stageGfxOpSetPixelFormat(int target, int format) throws InterruptedException {
        synchronized (gfxOpLock) {
            waitUntilSlotFree();
            pendingGfxOpKind = GfxOpKind.SET_PIXEL_FORMAT;
            pendingGfxOpTarget = target;
            pendingGfxOpPixelFormat = format;
            pendingGfxOpPixels = null;
            gfxOpLock.notifyAll();
            waitUntilDrained();
        }
    }

    private void stageGfxOpFrameRgba(int target, int w, int h, byte[] rgba) throws InterruptedException {
        // Defensive copy — caller may reuse its decode buffer.
        byte[] copy = new byte[rgba.length];
        System.arraycopy(rgba, 0, copy, 0, rgba.length);
        synchronized (gfxOpLock) {
            waitUntilSlotFree();
            pendingGfxOpKind = GfxOpKind.FRAME_RGBA;
            pendingGfxOpTarget = target;
            pendingGfxOpWidth = w;
            pendingGfxOpHeight = h;
            pendingGfxOpPixels = copy;
            gfxOpLock.notifyAll();
            waitUntilDrained();
        }
    }

    private void stageGfxOpBlit(int target, int x, int y, int w, int h, byte[] pixels, int format)
            throws InterruptedException {
        byte[] copy = new byte[pixels.length];
        System.arraycopy(pixels, 0, copy, 0, pixels.length);
        synchronized (gfxOpLock) {
            waitUntilSlotFree();
            pendingGfxOpKind = GfxOpKind.BLIT_RECT;
            pendingGfxOpTarget = target;
            pendingGfxOpX = x;
            pendingGfxOpY = y;
            pendingGfxOpWidth = w;
            pendingGfxOpHeight = h;
            pendingGfxOpPixelFormat = format;
            pendingGfxOpPixels = copy;
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
        int target, w, h, mode, format, x, y;
        byte[] pixels;
        synchronized (gfxOpLock) {
            kind = pendingGfxOpKind;
            if (kind == null) return;
            target = pendingGfxOpTarget;
            w = pendingGfxOpWidth;
            h = pendingGfxOpHeight;
            mode = pendingGfxOpMode;
            format = pendingGfxOpPixelFormat;
            x = pendingGfxOpX;
            y = pendingGfxOpY;
            pixels = pendingGfxOpPixels;
        }
        try {
            // Child abort in progress: discard the op without touching
            // kernel WASM memory. This prevents a late-arriving frame
            // from a still-dying WASI child from overwriting the state
            // that reset_to_shell just cleared.
            if (childAbortRequested) return;

            int base = (target == GFX_TARGET_SCREEN) ? screenBase : gfxBase;
            int cap = (target == GFX_TARGET_SCREEN) ? screenCap : gfxCap;
            if (base < 0) return;
            // The regions are kernel statics: a write past the cap would
            // corrupt kernel memory, so every op is checked against it.
            long need = switch (kind) {
                case INIT, FRAME -> GFX_PIXEL_OFF + (long) w * h;
                case FRAME_RGBA -> GFX_PIXEL_OFF + (long) w * h * 4;
                case SET_PIXEL_FORMAT -> GFX_PIXEL_OFF + (long) headerW(base) * headerH(base)
                        * (format == PIXEL_FORMAT_RGBA8888 ? 4 : 1);
                case BLIT_RECT -> GFX_PIXEL_OFF + (long) headerW(base) * headerH(base)
                        * (format == PIXEL_FORMAT_RGBA8888 ? 4 : 1);
                case SET_MODE -> 0x40;
            };
            if (w < 0 || h < 0 || need > cap) {
                EvansComputerMod.LOGGER.debug("Rejected gfx op {} ({}x{}): needs {} bytes, region holds {}",
                        kind, w, h, need, cap);
                return;
            }
            switch (kind) {
                case INIT             -> applyGfxInit(base, w, h);
                case FRAME            -> applyGfxFrame(base, w, h, pixels);
                case SET_MODE         -> applyGfxSetMode(base, mode);
                case SET_PIXEL_FORMAT -> applyGfxSetPixelFormat(base, format);
                case FRAME_RGBA       -> applyGfxFrameRgba(base, w, h, pixels);
                case BLIT_RECT        -> applyGfxBlitRect(base, x, y, w, h, pixels, format);
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

    private int headerW(int base) {
        return (memory.readByte(base + 4) & 0xFF) | ((memory.readByte(base + 5) & 0xFF) << 8);
    }

    private int headerH(int base) {
        return (memory.readByte(base + 6) & 0xFF) | ((memory.readByte(base + 7) & 0xFF) << 8);
    }

    /**
     * Write the gfx header (magic, mode=1, w, h), the RGB332 palette,
     * and zero pixels into kernel WASM memory at {@code base}; bump
     * both dirty counters so the worker loop picks up the change.
     * Always uses {@link #PIXEL_FORMAT_INDEXED8}; rgba init goes
     * through {@code applyGfxInitRgba} once Phase 2 lands.
     */
    private void applyGfxInit(int base, int w, int h) {
        if (memory == null) return;
        int total = GFX_PIXEL_OFF + w * h;
        if (memory.size() < base + total) return;

        memory.writeByte(base,     (byte) 0x02);
        memory.writeByte(base + 1, (byte) 0xFB);
        memory.writeByte(base + 2, (byte) 1);    // mode = gfx
        memory.writeByte(base + 3, (byte) 0);
        memory.writeShort(base + 4, (short) w);
        memory.writeShort(base + 6, (short) h);
        memory.writeByte(base + GFX_OFF_PIXEL_FORMAT, (byte) PIXEL_FORMAT_INDEXED8);

        int palDirty = memory.readInt(base + 0x08) + 1;
        int pixDirty = memory.readInt(base + 0x0C) + 1;
        memory.writeInt(base + 0x08, palDirty);
        memory.writeInt(base + 0x0C, pixDirty);

        byte[] palette = com.example.evanscomputermod.computer.video.Rgb332Palette.bytes();
        memory.writeBytes(base + GFX_PALETTE_OFF, palette);

        int pixelBase = base + GFX_PIXEL_OFF;
        memory.writeBytes(pixelBase, new byte[w * h]);
    }

    /**
     * Write the new pixel bytes into kernel WASM memory and bump the
     * pixel dirty counter. The palette is already installed by init;
     * frames don't re-push it. Pixel format is preserved — used by
     * indexed callers in Phase 1, will gain an rgba sibling in Phase 2.
     */
    private void applyGfxFrame(int base, int w, int h, byte[] pixels) {
        if (memory == null || pixels == null) return;
        int total = GFX_PIXEL_OFF + w * h;
        if (memory.size() < base + total) return;
        if (pixels.length < w * h) return;

        memory.writeByte(base,     (byte) 0x02);
        memory.writeByte(base + 1, (byte) 0xFB);
        memory.writeByte(base + 2, (byte) 1);
        memory.writeShort(base + 4, (short) w);
        memory.writeShort(base + 6, (short) h);
        memory.writeByte(base + GFX_OFF_PIXEL_FORMAT, (byte) PIXEL_FORMAT_INDEXED8);

        memory.writeBytes(base + GFX_PIXEL_OFF, pixels, 0, w * h);

        int pixDirty = memory.readInt(base + 0x0C) + 1;
        memory.writeInt(base + 0x0C, pixDirty);
    }

    /**
     * Update just the display mode byte and bump the pixel dirty
     * counter so the worker loop re-reads and propagates the change
     * to the Java display.
     */
    private void applyGfxSetMode(int base, int mode) {
        if (memory == null) return;
        if (memory.size() < base + 0x10) return;
        memory.writeByte(base + 2, (byte) mode);
        int pixDirty = memory.readInt(base + 0x0C) + 1;
        memory.writeInt(base + 0x0C, pixDirty);
    }

    /**
     * Switch the pixel format byte at {@code base + 0x10}, zero the
     * pixel region for the new format's byte count, and bump both
     * dirty counters so the next read picks up the structural change.
     */
    private void applyGfxSetPixelFormat(int base, int format) {
        if (memory == null || base < 0) return;
        if (memory.size() < base + 0x40) return;
        memory.writeByte(base + GFX_OFF_PIXEL_FORMAT, (byte) format);

        int w = (memory.readByte(base + 4) & 0xFF) | ((memory.readByte(base + 5) & 0xFF) << 8);
        int h = (memory.readByte(base + 6) & 0xFF) | ((memory.readByte(base + 7) & 0xFF) << 8);
        int bpp = (format == PIXEL_FORMAT_RGBA8888) ? 4 : 1;
        int pixBytes = w * h * bpp;
        int pixelBase = base + GFX_PIXEL_OFF;
        int cap = (base == screenBase) ? screenCap : gfxCap;
        if ((long) GFX_PIXEL_OFF + pixBytes <= cap) {
            memory.writeBytes(pixelBase, new byte[pixBytes]);
        }

        int palDirty = memory.readInt(base + 0x08) + 1;
        int pixDirty = memory.readInt(base + 0x0C) + 1;
        memory.writeInt(base + 0x08, palDirty);
        memory.writeInt(base + 0x0C, pixDirty);
    }

    /**
     * RGBA frame variant of {@link #applyGfxFrame}. Writes
     * {@code w*h*4} bytes of packed RGBA into the pixel region.
     */
    private void applyGfxFrameRgba(int base, int w, int h, byte[] pixels) {
        if (memory == null || pixels == null) return;
        int pixBytes = w * h * 4;
        int total = GFX_PIXEL_OFF + pixBytes;
        if (memory.size() < base + total) return;
        if (pixels.length < pixBytes) return;

        memory.writeByte(base,     (byte) 0x02);
        memory.writeByte(base + 1, (byte) 0xFB);
        memory.writeByte(base + 2, (byte) 1);
        memory.writeShort(base + 4, (short) w);
        memory.writeShort(base + 6, (short) h);
        memory.writeByte(base + GFX_OFF_PIXEL_FORMAT, (byte) PIXEL_FORMAT_RGBA8888);

        memory.writeBytes(base + GFX_PIXEL_OFF, pixels, 0, pixBytes);

        int pixDirty = memory.readInt(base + 0x0C) + 1;
        memory.writeInt(base + 0x0C, pixDirty);
    }

    /**
     * Copy a {@code w x h} rectangle of pixels from {@code pixels} into
     * the target framebuffer at origin ({@code x}, {@code y}).
     */
    private void applyGfxBlitRect(int base, int x, int y, int w, int h, byte[] pixels, int format) {
        if (memory == null || pixels == null) return;
        if (memory.size() < base + 0x40) return;

        int fbW = (memory.readByte(base + 4) & 0xFF) | ((memory.readByte(base + 5) & 0xFF) << 8);
        int fbH = (memory.readByte(base + 6) & 0xFF) | ((memory.readByte(base + 7) & 0xFF) << 8);
        if (fbW <= 0 || fbH <= 0) return;
        if (x < 0 || y < 0 || w <= 0 || h <= 0) return;
        if (x + w > fbW || y + h > fbH) return;

        int bpp = (format == PIXEL_FORMAT_RGBA8888) ? 4 : 1;
        if (pixels.length < w * h * bpp) return;
        int pixBytes = fbW * fbH * bpp;
        if (memory.size() < base + GFX_PIXEL_OFF + pixBytes) return;

        int pixelBase = base + GFX_PIXEL_OFF;
        int srcStride = w * bpp;
        int dstStride = fbW * bpp;
        for (int row = 0; row < h; row++) {
            int dstOff = pixelBase + (y + row) * dstStride + x * bpp;
            int srcOff = row * srcStride;
            memory.writeBytes(dstOff, pixels, srcOff, srcStride);
        }

        int pixDirty = memory.readInt(base + 0x0C) + 1;
        memory.writeInt(base + 0x0C, pixDirty);
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
            memory.writeBytes(bufPtr, bytes, 0, bytesToWrite);
            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing to WASM memory", e);
            return -1;
        }
    }

    /**
     * Reads raw bytes from WASM memory.
     */
    private byte[] readBytesFromMemory(int ptr, int len) {
        if (memory == null || len <= 0) {
            return new byte[0];
        }
        try {
            return memory.readBytes(ptr, len);
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
            memory.writeBytes(ptr, data, 0, bytesToWrite);
            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing bytes to WASM memory", e);
            return -1;
        }
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
            memory.writeBytes(bufPtr, data, 0, bytesToWrite);
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
        if (!fileName.endsWith(".wasm")) {
            fileName = fileName + ".wasm";
        }

        Path wasmFile = WasmManager.getWasmBinPath().resolve(fileName);
        if (!Files.exists(wasmFile)) {
            throw new WasmManager.WasmExecutionException("WASM file not found: " + wasmFile.toAbsolutePath());
        }

        WasmRuntime runtime = WasmManager.runtime();
        if (runtime == null) {
            throw new WasmManager.WasmExecutionException("No WASM runtime bound");
        }

        try {
            byte[] wasmBytes = Files.readAllBytes(wasmFile);
            EvansComputerMod.LOGGER.info("Loading WASM module: {}", fileName);

            WasmModuleHandle handle = runtime.compile(wasmBytes);
            checkKernelImports(handle);
            EvansComputerMod.LOGGER.info("Providing {} imports to WASM module", hostFunctions.size());

            instance = runtime.instantiate(handle, hostFunctions);
            memory = instance.memory();
            if (memory == null) {
                throw new WasmManager.WasmExecutionException("Kernel does not export 'memory'");
            }

            // Invalidate cached export handles — they belong to the previous instance.
            handleSockIpcFunc = null;

            readKernelLayout();

            EvansComputerMod.LOGGER.info("Successfully loaded WASM module: {}", fileName);

        } catch (WasmTrap e) {
            throw new WasmManager.WasmExecutionException("Failed to load WASM module: " + e.getMessage(), e);
        } catch (Exception e) {
            throw new WasmManager.WasmExecutionException("Failed to load WASM module: " + e.getMessage(), e);
        }
    }

    /**
     * Strict linking: every function the kernel imports must be implemented
     * here with the exact signature. A silently stubbed import (returning 0)
     * is how the kernel once ran with a clock stuck at zero, so a mismatch is
     * a load error instead.
     */
    private void checkKernelImports(WasmModuleHandle handle) throws WasmManager.WasmExecutionException {
        java.util.Map<String, WasmHostFunc> provided = new java.util.HashMap<>();
        for (WasmHostFunc f : hostFunctions) {
            provided.put(f.moduleName() + "::" + f.fieldName(), f);
        }
        List<String> problems = new ArrayList<>();
        for (WasmModuleHandle.ImportDescriptor imp : handle.imports()) {
            if (imp.kind != WasmModuleHandle.ImportDescriptor.Kind.FUNCTION) continue;
            WasmHostFunc f = provided.get(imp.moduleName + "::" + imp.fieldName);
            if (f == null) {
                problems.add("missing " + imp.moduleName + "::" + imp.fieldName);
            } else if (!f.params().equals(imp.params) || !f.results().equals(imp.results)) {
                problems.add("signature mismatch " + imp.fieldName + ": kernel " + imp.params + "->" + imp.results
                        + ", host " + f.params() + "->" + f.results());
            }
        }
        if (!problems.isEmpty()) {
            throw new WasmManager.WasmExecutionException("Kernel/host ABI mismatch: " + String.join("; ", problems));
        }
    }

    /**
     * Read the kernel's shared-memory layout (abi_scratch + abi_layout
     * exports) and bind every region address used by the host.
     */
    private void readKernelLayout() throws WasmManager.WasmExecutionException {
        try {
            long[] scratch = instance.callExport("abi_scratch");
            int addr = (int) scratch[0];
            long[] n = instance.callExport("abi_layout", addr, 15);
            if (n[0] < 15) {
                throw new WasmManager.WasmExecutionException("abi_layout returned " + n[0] + " words");
            }
            int[] w = new int[15];
            for (int i = 0; i < 15; i++) {
                w[i] = memory.readInt(addr + i * 4);
            }
            if (w[0] != 1) {
                throw new WasmManager.WasmExecutionException("Unsupported kernel ABI layout version " + w[0]);
            }
            inputBuf = w[1];  inputCap = w[2];
            irqBuf = w[3];    irqCap = w[4];
            netIpcBridge.setLayout(memory, w[5], w[6], w[7], w[8]);
            fbBase = w[9];    fbCap = w[10];
            gfxBase = w[11];  gfxCap = w[12];
            screenBase = w[13]; screenCap = w[14];
            EvansComputerMod.LOGGER.debug("Kernel layout: input={} irq={} fb={} gfx={} screen={}",
                    inputBuf, irqBuf, fbBase, gfxBase, screenBase);
        } catch (WasmTrap e) {
            throw new WasmManager.WasmExecutionException("Kernel layout query failed: " + e.getMessage(), e);
        }
    }

    /**
     * Executes a function from the loaded WASM module. Parameters use the
     * SPI's {@code long}-encoded ABI (see {@link WasmHostFunc}).
     */
    public WasmManager.WasmResult executeFunction(String functionName, long... params)
            throws WasmManager.WasmExecutionException {

        if (instance == null) {
            throw new WasmManager.WasmExecutionException("No WASM module loaded");
        }

        if (!instance.hasExport(functionName)) {
            throw new WasmManager.WasmExecutionException(
                    "Function '" + functionName + "' not found in module");
        }

        try {
            long[] results = instance.callExport(functionName, params);
            return new WasmManager.WasmResult(results);
        } catch (WasmTrap e) {
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

        String[] entryPoints = {"main", "_start", "start", "init"};

        for (String entryPoint : entryPoints) {
            if (instance.hasExport(entryPoint)) {
                try {
                    instance.callExport(entryPoint);
                    EvansComputerMod.LOGGER.info("Executed WASM entry point: {}", entryPoint);
                    return;
                } catch (WasmTrap e) {
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
        // Ctrl+T. The kernel itself handles the keystroke (IRQ_TERMINATE,
        // queued by the caller): it kills the foreground job and returns to
        // the prompt, leaving background jobs and a background switch alone.
        // Only a kernel call that is genuinely stuck is forcibly trapped.
        childAbortRequested = true;
        synchronized (gfxOpLock) {
            pendingGfxOpKind = null;
            pendingGfxOpPixels = null;
            gfxOpLock.notifyAll();
        }
        if (wasmExecuting && System.currentTimeMillis() - kernelCallStartMs > STUCK_KERNEL_CALL_MS) {
            EvansComputerMod.LOGGER.info("Ctrl+T: kernel call stuck, requesting interrupt");
            interrupted = true;
            if (instance != null) {
                try {
                    instance.requestInterrupt();
                } catch (Exception e) {
                    EvansComputerMod.LOGGER.warn("requestInterrupt failed: {}", e.getMessage());
                }
            }
        }
        wakeWorker();
    }

    /**
     * Checks if execution should be interrupted and throws if so.
     * Called by host functions to allow interruption of long-running WASM code.
     */
    private void checkInterrupted() {
        if (interrupted) {
            EvansComputerMod.LOGGER.info("checkInterrupted() throwing WasmTrap(INTERRUPTED)");
            throw new WasmTrap(WasmTrap.Kind.INTERRUPTED, "WASM execution interrupted");
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
                StackTraceElement[] st = workerThread.getStackTrace();
                StringBuilder sb = new StringBuilder();
                sb.append("WASM worker thread did not terminate in time; stack:\n");
                for (StackTraceElement e : st) sb.append("    at ").append(e).append('\n');
                EvansComputerMod.LOGGER.warn(sb.toString());
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
            instance = null;
        }
        hostFunctions.clear();
    }
}
