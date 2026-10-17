package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;

/**
 * Thin facade exposing kernel-side host operations (redstone, sleep, peripherals)
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

    // --- Peripherals (see ecm_host_abi::peripheral) ---

    /** Events queue for one program; created on its first peripheral call. */
    public PeripheralEventBus.Subscriber peripheralSubscribe() {
        return parent.getPeripheralEvents().subscribe();
    }

    public void peripheralUnsubscribe(PeripheralEventBus.Subscriber subscriber) {
        parent.getPeripheralEvents().unsubscribe(subscriber);
    }

    /** Type of the peripheral attached as {@code name}, or null if none (permission checks). */
    public String peripheralType(String name) {
        return parent.bridgePeripheralType(name);
    }

    public byte[] peripheralList() {
        return parent.bridgePeripheralList();
    }

    public byte[] peripheralMethods(String name) {
        return parent.bridgePeripheralMethods(name);
    }

    public byte[] peripheralCall(String name, String method, byte[] args) {
        return parent.bridgePeripheralCall(name, method, args);
    }

    /**
     * Next event matching {@code filter} (any if null) as an encoded ok frame,
     * or null after {@code timeoutMs} (0 = poll, negative = wait forever).
     */
    public byte[] peripheralWaitEvent(PeripheralEventBus.Subscriber subscriber, String filter, long timeoutMs) {
        try {
            PeripheralEventBus.Event e = PeripheralEventBus.await(subscriber, filter, timeoutMs);
            if (e == null) return null;
            byte[] frame = new byte[e.encoded().length + 1];
            frame[0] = PeripheralValues.STATUS_OK;
            System.arraycopy(e.encoded(), 0, frame, 1, e.encoded().length);
            return frame;
        } catch (InterruptedException ex) {
            Thread.currentThread().interrupt();
            // ProcessManager treats "interrupted" as a clean kill (exit 130).
            throw new RuntimeException("peripheral wait interrupted");
        }
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
    public com.example.evanscomputermod.api.video.VideoSource.Info videoGetInfo(int handle) {
        return parent.bridgeVideoGetInfo(handle);
    }

    /**
     * Decode next frame and push it to the selected display (0 terminal,
     * 1 screen). Returns pts_ms, -1 EOF, -2 error.
     */
    public long videoDecodeToGfx(int handle, int target) {
        return parent.bridgeVideoDecodeToGfx(handle, target);
    }

    /** Decode the next frame into {@code dst}. Returns pts_ms, -1 EOF, -2 error. */
    public long videoDecodeFrame(int handle, byte[] dst) {
        return parent.bridgeVideoDecodeFrame(handle, dst);
    }

    /** Whether video files can be decoded here (the FFmpeg sidecar is installed). */
    public boolean videoSupported() {
        return parent.bridgeVideoSupported();
    }

    // --- Upload port (files from the player's real computer; the upload sidecar) ---

    public int uploadSupported() {
        return parent.bridgeUploadSupported();
    }

    public int uploadBegin(String dir) {
        return parent.bridgeUploadBegin(dir);
    }

    public com.example.evanscomputermod.computer.upload.UploadPort.Progress uploadPoll(int ticket) {
        return parent.bridgeUploadPoll(ticket);
    }

    public int uploadCancel(int ticket) {
        return parent.bridgeUploadCancel(ticket);
    }

    /** Bytes in one decoded frame, -1 for an unknown handle. */
    public int videoFrameBytes(int handle) {
        return parent.bridgeVideoFrameBytes(handle);
    }

    /** Turn the video's sound on as s16le PCM. 0 ok, -4 no audio, -1 bad argument. */
    public int videoAudioOpen(int handle, int rate, int channels) {
        return parent.bridgeVideoAudioOpen(handle, rate, channels);
    }

    /** Take queued PCM into {@code dst}: bytes, 0 at the end, negative on error. */
    public int videoReadAudio(int handle, byte[] dst) {
        return parent.bridgeVideoReadAudio(handle, dst);
    }

    /** The file's audio format, {@code rate << 8 | channels}; 0 without audio. */
    public int videoAudioSource(int handle) {
        return parent.bridgeVideoAudioSource(handle);
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

    /** Switch the screen cluster's pixel format (clears it). Returns 0 / negative. */
    public int screenSetPixelFormat(int format) {
        return parent.bridgeGfxSetPixelFormat(ComputerInstance.GFX_TARGET_SCREEN, format);
    }

    /**
     * Replace the screen cluster's frame with {@code w*h*4} bytes of RGBA8888.
     */
    public int screenPutFrameRgba(int w, int h, byte[] rgba) {
        return parent.bridgeGfxFrameRgba(ComputerInstance.GFX_TARGET_SCREEN, w, h, rgba);
    }

    /**
     * Blit a {@code w x h} sub-rectangle of pixels at {@code (x,y)} on
     * the selected target's framebuffer. {@code pixels} is tightly
     * packed row-major in the display's own format. Out-of-bounds rects
     * and format mismatches are rejected.
     */
    public int gfxBlitRect(int target, int x, int y, int w, int h, byte[] pixels, int format) {
        return parent.bridgeGfxBlitRect(target, x, y, w, h, pixels, format);
    }

    /** Take the display over with a size, pixel format and flags (see DisplayDevice). */
    public int gfxInit2(int target, int w, int h, int format, int flags) {
        return parent.bridgeGfxInit2(target, w, h, format, flags);
    }

    /** Change the display's pixel format (clears it). */
    public int gfxSetFormat(int target, int format) {
        return parent.bridgeGfxSetPixelFormat(target, format);
    }

    public int gfxSetPalette(int target, int first, byte[] rgb) {
        return parent.bridgeGfxSetPalette(target, first, rgb);
    }

    public long gfxPresent(int target, int flags) {
        return parent.bridgeGfxPresent(target, flags);
    }

    public long gfxWaitVblank(int target) {
        return parent.bridgeGfxWaitVblank(target);
    }

    public int gfxSetRefresh(int target, int hz) {
        return parent.bridgeGfxSetRefresh(target, hz);
    }

    public com.example.evanscomputermod.computer.display.DisplayDevice.Info gfxInfo(int target) {
        return parent.bridgeGfxInfo(target);
    }

    /** Panel tier of a target (1..3), 0 without a panel. */
    public int gfxTier(int target) {
        return parent.bridgeGfxTier(target);
    }

    /** {@code gfx_modes} words, or null without a panel. */
    public int[] gfxModes(int target) {
        return parent.bridgeGfxModes(target);
    }

    // --- UI protocol (docs/gui/UI_PROTOCOL.md §3) ---

    /** {@code ui_open}: a {@code UiConnection}, or an Integer error (-4 console mode, -2 shell taken, -1 invalid). */
    public Object uiOpen(int role, int version) {
        return parent.bridgeUiOpen(role, version);
    }

    // --- Devices under /dev ---

    /** Open {@code /dev/<name>} (e.g. {@code audio}, {@code audioctl.left}), or null if there is none. */
    public WasiFileDescriptor openDevice(String name) {
        return parent.bridgeOpenDevice(name);
    }

    public boolean hasDevice(String name) {
        return parent.bridgeOpenDevice(name) != null;
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
}
