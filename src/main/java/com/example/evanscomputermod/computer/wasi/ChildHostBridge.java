package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.computer.ComputerInstance;

/**
 * Thin facade exposing kernel-side host operations (redstone, peripherals, sleep)
 * to child WASI processes. Each child WASI process gets host functions registered
 * in {@link WasiFunctions} that delegate through this bridge to the parent
 * {@link ComputerInstance}, which already implements these operations for the
 * kernel WASM.
 *
 * <p>The bridge doesn't touch WASM memory directly — it accepts/returns plain
 * Java values. The WASI handlers are responsible for marshalling between WASM
 * memory and these calls.
 */
public class ChildHostBridge {

    private final ComputerInstance parent;

    public ChildHostBridge(ComputerInstance parent) {
        this.parent = parent;
    }

    // --- Redstone ---

    /** Set redstone output power on a relative side (0-5), power 0-15. Returns 0 on success. */
    public int redstoneSetOutput(int side, int power) {
        return parent.bridgeRedstoneSetOutput(side, power);
    }

    /** Read redstone input power on a relative side (0-5). Returns 0-15. */
    public int redstoneGetInput(int side) {
        return parent.bridgeRedstoneGetInput(side);
    }

    /** Read all 6 sides' redstone input power into the given array (length 6). */
    public int redstoneGetAllInput(int[] out) {
        return parent.bridgeRedstoneGetAllInput(out);
    }

    // --- Peripherals ---

    /** List all connected peripherals as JSON. */
    public String peripheralListJson() {
        return parent.bridgePeripheralListJson();
    }

    /** Get methods for a peripheral as JSON. */
    public String peripheralMethodsJson(String name) {
        return parent.bridgePeripheralMethodsJson(name);
    }

    /** Call a peripheral method with JSON args, return result JSON. */
    public String peripheralCall(String name, String method, String argsJson) {
        return parent.bridgePeripheralCall(name, method, argsJson);
    }

    // --- Sleep ---

    /** Sleep for the given number of milliseconds (clamped 0..60000). */
    public void sleepMs(int ms) {
        parent.bridgeSleepMs(ms);
    }

    // --- Raw packet capture bridge ---

    /** Enable/disable promiscuous mode on a specific interface index. */
    public int netSetPromiscuousOn(int index, int enabled) {
        return parent.bridgeNetSetPromiscuousOn(index, enabled);
    }

    /** Enable/disable pcap mirror queue on a specific interface index. */
    public int netPcapEnable(int index, int enabled) {
        return parent.bridgeNetPcapEnable(index, enabled);
    }

    /** Non-blocking read from the pcap mirror queue for an interface. */
    public byte[] netPcapRx(int index) {
        return parent.bridgeNetPcapRx(index);
    }

    // --- Video playback bridge (for the `player` WASI program) ---

    /**
     * Open an MP4 from the VFS path with the given output pixel format
     * (0 = indexed8, 1 = rgba8888). Returns a handle or {@code -1}.
     */
    public int videoOpen(String path, int targetW, int targetH, int pixelFormat) {
        return parent.bridgeVideoOpen(path, targetW, targetH, pixelFormat);
    }

    /** Return the decoder's static metadata, or {@code null} if unknown handle. */
    public com.example.evanscomputermod.computer.video.VideoDecoder.VideoInfo videoGetInfo(int handle) {
        return parent.bridgeVideoGetInfo(handle);
    }

    /**
     * Decode next frame and push it to the selected display (0 terminal,
     * 1 screen). Returns pts_ms, -1 EOF, -2 error.
     */
    public long videoDecodeToGfx(int handle, int target) {
        return parent.bridgeVideoDecodeToGfx(handle, target);
    }

    /** Seek to {@code ptsMs}. Returns 0 on success, -1 otherwise. */
    public int videoSeek(int handle, long ptsMs) {
        return parent.bridgeVideoSeek(handle, ptsMs);
    }

    /** Close and release the decoder. Returns 0 on success, -1 otherwise. */
    public int videoClose(int handle) {
        return parent.bridgeVideoClose(handle);
    }

    /** Initialize the gfx framebuffer for {@code target} with {@code w × h} pixels and mode=1. */
    public int gfxInit(int target, int w, int h) {
        return parent.bridgeGfxInit(target, w, h);
    }

    /** Switch display mode on {@code target}: 0 text / 1 gfx / 2 overlay. */
    public int gfxSetMode(int target, int mode) {
        return parent.bridgeGfxSetMode(target, mode);
    }

    /**
     * Query the attached Screen cluster's pixel dimensions. Returns
     * {@code (width << 32) | height}, or {@code -1L} if no cluster is
     * attached. Called by the player before {@code video_open} when
     * targeting the screen so the decoder is sized to the cluster's
     * native resolution.
     */
    public long screenQueryDims() {
        return parent.bridgeScreenQueryDims();
    }

    /** Power the attached Screen cluster on or off. No-op if no cluster. */
    public void screenSetPower(boolean on) {
        parent.bridgeScreenSetPower(on);
    }

    /**
     * Switch the screen cluster's pixel format (0=indexed8, 1=rgba8888).
     * Stages a worker-thread op that writes the format byte, zeros the
     * pixel region, and bumps dirty counters. Returns 0 / -1.
     */
    public int screenSetPixelFormat(int format) {
        return parent.bridgeGfxSetPixelFormat(ComputerInstance.GFX_TARGET_SCREEN, format);
    }

    /**
     * Push a full RGBA8888 frame ({@code w*h*4} bytes) into the screen
     * cluster's pixel region. Format must already be RGBA8888.
     */
    public int screenPutFrameRgba(int w, int h, byte[] rgba) {
        return parent.bridgeGfxFrameRgba(ComputerInstance.GFX_TARGET_SCREEN, w, h, rgba);
    }

    /**
     * Blit a {@code w x h} sub-rectangle of pixels at {@code (x,y)} on
     * the selected target's framebuffer. {@code pixels} is tightly
     * packed row-major at the given {@code format} (0 indexed8, 1
     * rgba8888). Out-of-bounds rects are rejected.
     */
    public int gfxBlitRect(int target, int x, int y, int w, int h, byte[] pixels, int format) {
        return parent.bridgeGfxBlitRect(target, x, y, w, h, pixels, format);
    }

    // --- Mouse capture bridge (for WASI programs that want pointer input) ---

    /** Request mouse capture. Returns 1 on success, 0 if not eligible. */
    public int mouseCaptureStart() {
        return parent.bridgeMouseCaptureStart();
    }

    /** Disable mouse capture and drop any pending events. */
    public void mouseCaptureStop() {
        parent.bridgeMouseCaptureStop();
    }

    /** Returns 1 if capture is currently enabled, 0 otherwise. */
    public int mouseCaptureIsActive() {
        return parent.bridgeMouseCaptureIsActive();
    }

    /** Pop one 10-byte mouse event into {@code out}. Returns 1 if written, 0 if empty. */
    public int mousePoll(byte[] out) {
        return parent.bridgeMousePoll(out);
    }

    // --- Computer module bridge ---

    /** List all registered computer modules as JSON metadata. */
    public String moduleListJson() {
        return parent.bridgeModuleListJson();
    }

    /** Call a computer module method with binary-encoded args, return binary result. */
    public byte[] moduleCall(String moduleName, String methodName, byte[] argsBinary) {
        return parent.bridgeModuleCall(moduleName, methodName, argsBinary);
    }
}
