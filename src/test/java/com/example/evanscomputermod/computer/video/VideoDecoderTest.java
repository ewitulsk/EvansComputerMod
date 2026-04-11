package com.example.evanscomputermod.computer.video;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Unit tests for {@link VideoDecoder} and {@link Rgb332Palette}. The test
 * uses a real H.264 MP4 at {@code src/test/resources/video/sample.mp4}
 * (~950 KB); the file is loaded via the classloader.
 */
class VideoDecoderTest {

    private static Path sampleMp4() throws Exception {
        URL url = VideoDecoderTest.class.getClassLoader()
                .getResource("video/sample.mp4");
        assertNotNull(url, "sample.mp4 missing from test resources");
        return Paths.get(url.toURI());
    }

    @Test
    void palette_has_256_entries_and_canonical_anchors() {
        byte[] p = Rgb332Palette.bytes();
        assertEquals(768, p.length);
        // Index 0 = black.
        assertEquals(0, p[0] & 0xFF);
        assertEquals(0, p[1] & 0xFF);
        assertEquals(0, p[2] & 0xFF);
        // Index 255 = (R=3,G=7,B=7) -> full white.
        assertEquals(255, p[255 * 3] & 0xFF);
        assertEquals(255, p[255 * 3 + 1] & 0xFF);
        assertEquals(255, p[255 * 3 + 2] & 0xFF);
    }

    @Test
    void palette_red_ramp_is_monotone_non_decreasing() {
        byte[] p = Rgb332Palette.bytes();
        int prev = -1;
        // Index layout: (msb) 2R 3G 3B (lsb). Stepping R with G=B=0
        // corresponds to indices 0, 64, 128, 192.
        int[] rIdx = {0, 64, 128, 192};
        for (int idx : rIdx) {
            int r = p[idx * 3] & 0xFF;
            assertTrue(r >= prev, "red ramp regressed at " + idx);
            prev = r;
        }
    }

    @Test
    void decode_sample_reports_sane_video_info() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(sampleMp4(), 320, 200)) {
            VideoDecoder.VideoInfo info = d.info();
            assertTrue(info.width > 0,  "width must be positive");
            assertTrue(info.height > 0, "height must be positive");
            assertTrue(info.durationMs >= 0, "durationMs must be non-negative");
            System.out.printf("sample info: %dx%d fps=%d/%d frames=%d duration=%dms%n",
                    info.width, info.height, info.fpsNum, info.fpsDen,
                    info.frameCount, info.durationMs);
        }
    }

    @Test
    void decode_sample_first_frames_monotone_pts_and_non_blank() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(sampleMp4(), 320, 200)) {
            List<Long> ptsList = new ArrayList<>();
            int framesDecoded = 0;
            int maxFrames = 30;
            int nonZeroFrames = 0;

            while (framesDecoded < maxFrames) {
                VideoDecoder.DecodedFrame f = d.next();
                if (f == null) break;
                assertEquals(320 * 200, f.indexed.length);
                ptsList.add(f.ptsMs);

                int nonZero = 0;
                for (byte b : f.indexed) if (b != 0) nonZero++;
                if (nonZero > 0) nonZeroFrames++;

                framesDecoded++;
            }

            assertTrue(framesDecoded > 0, "decoder produced no frames");
            assertTrue(nonZeroFrames > 0,
                    "every decoded frame was all-zero — swscale target format may be wrong");

            for (int i = 1; i < ptsList.size(); i++) {
                assertTrue(ptsList.get(i) >= ptsList.get(i - 1),
                        "pts regressed: " + ptsList.get(i - 1) + " -> " + ptsList.get(i));
            }
            System.out.printf("decoded %d frames, pts=[%d..%d] ms, %d non-blank%n",
                    framesDecoded, ptsList.get(0), ptsList.get(ptsList.size() - 1),
                    nonZeroFrames);
        }
    }

    @Test
    void decode_sample_full_run_reaches_eof() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(sampleMp4(), 160, 90)) {
            int count = 0;
            long lastPts = -1;
            while (true) {
                VideoDecoder.DecodedFrame f = d.next();
                if (f == null) break;
                lastPts = f.ptsMs;
                count++;
                // Safety brake in case the sample is huge.
                if (count > 5_000) break;
            }
            assertTrue(count > 0, "no frames decoded in full-run test");
            assertTrue(lastPts >= 0);
            System.out.printf("full-run: %d frames, last pts=%d ms%n", count, lastPts);
            // After EOF, further calls return null cleanly.
            assertNull(d.next());
        }
    }

    @Test
    void decoder_close_is_idempotent() throws Exception {
        VideoDecoder d = VideoDecoder.open(sampleMp4(), 320, 200);
        d.close();
        // Second close must not throw.
        d.close();
    }
}
