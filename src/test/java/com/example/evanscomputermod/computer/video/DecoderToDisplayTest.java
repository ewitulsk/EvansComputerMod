package com.example.evanscomputermod.computer.video;

import com.example.evanscomputermod.computer.TerminalDisplay;
import org.junit.jupiter.api.Test;

import java.net.URL;
import java.nio.file.Path;
import java.nio.file.Paths;

import static org.junit.jupiter.api.Assertions.*;

/**
 * End-to-end (Java only) test: decoder → gfx blob → TerminalDisplay.
 * Validates that a decoded frame round-trips into display state that is
 * structurally correct (dimensions, palette, pixels), without touching any
 * WASM memory or Minecraft runtime. This is the headless stand-in for the
 * "static frame in-game" step of the implementation plan.
 */
class DecoderToDisplayTest {

    private static Path sampleMp4() throws Exception {
        URL url = DecoderToDisplayTest.class.getClassLoader()
                .getResource("video/sample.mp4");
        assertNotNull(url, "sample.mp4 missing");
        return Paths.get(url.toURI());
    }

    @Test
    void single_frame_lands_in_terminal_display_with_correct_geometry() throws Exception {
        final int W = 320;
        final int H = 200;

        TerminalDisplay td = new TerminalDisplay();
        // TerminalDisplay starts in text mode with no gfx state.
        assertEquals(0, td.getDisplayMode());
        assertEquals(0, td.getGfxWidth());
        assertEquals(0, td.getGfxHeight());

        byte[] palette = Rgb332Palette.bytes();

        try (VideoDecoder d = VideoDecoder.open(sampleMp4(), W, H)) {
            VideoDecoder.DecodedFrame frame = d.next();
            assertNotNull(frame, "decoder produced no frame");
            assertEquals(W * H, frame.indexed.length);

            byte[] blob = GfxFrameBlob.build(W, H, /*mode*/ 1,
                    palette, frame.indexed,
                    /*paletteDirty*/ 1, /*pixelDirty*/ 1);

            td.setGfxFromBytes(blob);
        }

        // Verify TerminalDisplay accepted the frame.
        assertEquals(1, td.getDisplayMode(), "display must be in gfx mode");
        assertEquals(W, td.getGfxWidth());
        assertEquals(H, td.getGfxHeight());
        assertNotNull(td.getPixelData());
        assertEquals(W * H, td.getPixelData().length);

        // Palette sanity: index 0 must be black, index 255 full white.
        int[] argb = td.getPalette();
        assertEquals(0xFF000000, argb[0],   "palette[0] should be opaque black");
        assertEquals(0xFFFFFFFF, argb[255], "palette[255] should be opaque white");

        // Pixel sanity: at least some variation in the frame content.
        byte[] pix = td.getPixelData();
        int nonZero = 0;
        int distinct = 0;
        boolean[] seen = new boolean[256];
        for (byte b : pix) {
            int v = b & 0xFF;
            if (v != 0) nonZero++;
            if (!seen[v]) { seen[v] = true; distinct++; }
        }
        assertTrue(nonZero > 0, "frame is entirely index 0 (black)");
        assertTrue(distinct >= 4,
                "frame had only " + distinct + " distinct colors — decoder or"
                + " swscale pixel format likely wrong");

        System.out.printf(
                "decoded frame landed: mode=%d %dx%d distinctColors=%d nonZero=%d%n",
                td.getDisplayMode(), td.getGfxWidth(), td.getGfxHeight(),
                distinct, nonZero);
    }

    @Test
    void consecutive_frames_update_display() throws Exception {
        final int W = 160;
        final int H = 90;
        TerminalDisplay td = new TerminalDisplay();
        byte[] palette = Rgb332Palette.bytes();

        try (VideoDecoder d = VideoDecoder.open(sampleMp4(), W, H)) {
            int frames = 0;
            byte[] prev = null;
            int changedFrames = 0;
            while (frames < 10) {
                VideoDecoder.DecodedFrame f = d.next();
                if (f == null) break;
                byte[] blob = GfxFrameBlob.build(W, H, 1, palette, f.indexed, frames + 1, frames + 1);
                td.setGfxFromBytes(blob);
                if (prev != null) {
                    // Frames are copied out of the decoder, so the display's
                    // internal pixel buffer should reflect the current frame,
                    // not the previous one. Assert they differ from a prior.
                    byte[] cur = td.getPixelData().clone();
                    boolean diff = false;
                    for (int i = 0; i < cur.length; i++) {
                        if (cur[i] != prev[i]) { diff = true; break; }
                    }
                    if (diff) changedFrames++;
                }
                prev = td.getPixelData().clone();
                frames++;
            }
            assertTrue(frames >= 2, "need at least 2 frames to compare");
            assertTrue(changedFrames > 0,
                    "pixel data never changed between frames — display not being updated");
            assertEquals(10, td.getPixelDirtyCounter());
        }
    }
}
