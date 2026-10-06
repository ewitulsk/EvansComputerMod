package com.example.evanscomputermod.computer.display;

import com.example.evanscomputermod.computer.FramebufferDiffTracker;
import com.example.evanscomputermod.computer.TerminalDisplay;
import org.junit.jupiter.api.Test;

import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The program-facing display controller: ownership, formats, single vs.
 * double buffering, rectangle checks and the vblank clock.
 */
class DisplayDeviceTest {

    private static final DisplayDevice.RefreshLimits LIMITS = new DisplayDevice.RefreshLimits() {
        @Override public int defaultHz() { return 30; }
        @Override public int maxHz() { return 60; }
    };

    private final TerminalDisplay scanout = new TerminalDisplay();
    private final AtomicInteger frames = new AtomicInteger();
    private final AtomicInteger releases = new AtomicInteger();
    private final DisplayDevice dev = new DisplayDevice("test", () -> scanout, 640 * 400 * 4,
            LIMITS, frames::incrementAndGet, releases::incrementAndGet);

    @Test
    void initTakesOwnershipAndOtherProgramsAreBusy() {
        assertEquals(DisplayDevice.OK, dev.init(5, 240, 160, TerminalDisplay.PIXEL_FORMAT_RGB565, 0));
        assertEquals(5, dev.ownerPid());
        assertEquals(DisplayDevice.E_BUSY, dev.init(6, 10, 10, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0));
        assertEquals(DisplayDevice.E_NOT_OWNER,
                dev.blit(6, 0, 0, 1, 1, new byte[2], TerminalDisplay.PIXEL_FORMAT_RGB565));
        // The scanout shows the new mode straight away.
        assertEquals(1, scanout.getDisplayMode());
        assertEquals(240, scanout.getGfxWidth());
        assertEquals(TerminalDisplay.PIXEL_FORMAT_RGB565, scanout.getPixelFormat());
        assertEquals(240 * 160 * 2, scanout.getPixelData().length);
    }

    @Test
    void singleBufferedBlitIsVisibleImmediately() {
        dev.init(1, 4, 2, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        int before = scanout.getPixelDirtyCounter();
        assertEquals(DisplayDevice.OK,
                dev.blit(1, 1, 1, 2, 1, new byte[]{7, 9}, TerminalDisplay.PIXEL_FORMAT_INDEXED8));
        assertArrayEquals(new byte[]{0, 0, 0, 0, 0, 7, 9, 0}, scanout.getPixelData());
        assertTrue(scanout.getPixelDirtyCounter() > before);
    }

    @Test
    void doubleBufferedBlitWaitsForPresent() {
        dev.init(1, 2, 2, TerminalDisplay.PIXEL_FORMAT_INDEXED8, DisplayDevice.FLAG_DOUBLE_BUFFER);
        dev.blit(1, 0, 0, 2, 2, new byte[]{1, 2, 3, 4}, TerminalDisplay.PIXEL_FORMAT_INDEXED8);
        assertArrayEquals(new byte[4], scanout.getPixelData(), "back buffer must not be visible yet");
        long n = dev.present(1, 0);
        assertTrue(n > 0);
        assertArrayEquals(new byte[]{1, 2, 3, 4}, scanout.getPixelData());
    }

    @Test
    void blitRejectsWrongFormatAndOutOfBoundsRects() {
        dev.init(1, 4, 4, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        // An RGBA blit into an indexed display used to be written at 4x stride.
        assertEquals(DisplayDevice.E_INVALID,
                dev.blit(1, 0, 0, 1, 1, new byte[4], TerminalDisplay.PIXEL_FORMAT_RGBA8888));
        assertEquals(DisplayDevice.E_INVALID,
                dev.blit(1, 3, 3, 2, 2, new byte[4], TerminalDisplay.PIXEL_FORMAT_INDEXED8));
        assertEquals(DisplayDevice.E_INVALID,
                dev.blit(1, 0, 0, 2, 2, new byte[3], TerminalDisplay.PIXEL_FORMAT_INDEXED8));
    }

    @Test
    void oversizedModesAreRejected() {
        assertEquals(DisplayDevice.E_INVALID, dev.init(1, 4096, 4096, TerminalDisplay.PIXEL_FORMAT_RGBA8888, 0));
        assertEquals(DisplayDevice.E_INVALID, dev.init(1, 0, 10, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0));
        assertEquals(DisplayDevice.E_INVALID, dev.init(1, 10, 10, 9, 0));
        assertEquals(DisplayDevice.KERNEL, dev.ownerPid());
    }

    @Test
    void releaseAndTextModeHandTheDisplayBack() {
        dev.init(3, 8, 8, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        dev.release(4); // not the owner: no effect
        assertEquals(3, dev.ownerPid());
        dev.release(3);
        assertEquals(DisplayDevice.KERNEL, dev.ownerPid());
        assertEquals(1, releases.get());

        dev.init(3, 8, 8, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        assertEquals(DisplayDevice.OK, dev.setMode(3, 0));
        assertEquals(DisplayDevice.KERNEL, dev.ownerPid());
        assertEquals(2, releases.get());
    }

    @Test
    void legacyFormatThenInitKeepsTheFormat() {
        // The old `player screen` order: set the format, then gfx_init.
        assertEquals(DisplayDevice.OK, dev.setFormat(2, TerminalDisplay.PIXEL_FORMAT_RGBA8888));
        assertEquals(DisplayDevice.OK, dev.initLegacy(2, 16, 9));
        assertEquals(TerminalDisplay.PIXEL_FORMAT_RGBA8888, scanout.getPixelFormat());
        assertEquals(16 * 9 * 4, scanout.getPixelData().length);
    }

    @Test
    void pushFrameResizesToTheFrame() {
        byte[] rgba = new byte[3 * 2 * 4];
        rgba[0] = 42;
        assertEquals(DisplayDevice.OK, dev.pushFrame(9, 3, 2, TerminalDisplay.PIXEL_FORMAT_RGBA8888, rgba));
        assertEquals(9, dev.ownerPid());
        assertEquals(3, scanout.getGfxWidth());
        assertEquals(42, scanout.getPixelData()[0]);
    }

    @Test
    void paletteChangesReachTheScanout() {
        dev.init(1, 2, 2, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        assertEquals(DisplayDevice.OK, dev.setPalette(1, 5, new byte[]{(byte) 0x12, (byte) 0x34, (byte) 0x56}));
        assertEquals(0xFF123456, scanout.getPalette()[5]);
        assertEquals(DisplayDevice.E_INVALID, dev.setPalette(1, 255, new byte[6]));
    }

    @Test
    void refreshIsClampedToTheServerMaximum() {
        dev.init(1, 2, 2, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        assertEquals(30, dev.refreshHz());
        assertEquals(60, dev.setRefresh(1, 240));
        assertEquals(1, dev.setRefresh(1, 0));
    }

    @Test
    void waitVblankWaitsAboutOnePeriod() {
        dev.init(1, 2, 2, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        dev.setRefresh(1, 50); // 20 ms
        long a = dev.waitVblank();
        long t0 = System.nanoTime();
        long b = dev.waitVblank();
        long elapsedMs = (System.nanoTime() - t0) / 1_000_000;
        assertEquals(a + 1, b);
        assertTrue(elapsedMs >= 15 && elapsedMs < 200, "waited " + elapsedMs + " ms");
    }

    @Test
    void interruptedVblankWaitReturnsAnError() throws Exception {
        dev.init(1, 2, 2, TerminalDisplay.PIXEL_FORMAT_INDEXED8, 0);
        dev.setRefresh(1, 1);
        long[] result = new long[1];
        Thread t = new Thread(() -> result[0] = dev.waitVblank());
        t.start();
        Thread.sleep(50);
        t.interrupt();
        t.join(2000);
        assertEquals(DisplayDevice.E_INTERRUPTED, result[0]);
    }

    @Test
    void rgb565FramesDiffAndShipAsTwoBytesPerPixel() {
        dev.init(1, 32, 16, TerminalDisplay.PIXEL_FORMAT_RGB565, 0);
        FramebufferDiffTracker tracker = new FramebufferDiffTracker();
        tracker.commitShadow(scanout);
        byte[] px = {(byte) 0xFF, (byte) 0xFF};
        dev.blit(1, 20, 3, 1, 1, px, TerminalDisplay.PIXEL_FORMAT_RGB565);
        FramebufferDiffTracker.GfxDelta delta = tracker.computeGfxDelta(scanout);
        assertNotNull(delta);
        assertEquals(1, delta.changedTileIndices().size());
        assertEquals(1, delta.changedTileIndices().get(0)); // second 16x16 tile in the top row
        assertEquals(16 * 16 * 2, delta.changedTileData().get(0).length);
    }
}
