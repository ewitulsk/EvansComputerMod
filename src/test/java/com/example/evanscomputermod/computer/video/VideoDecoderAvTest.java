package com.example.evanscomputermod.computer.video;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The decoder paths the Video Player and the {@code video} command use, on
 * {@code video/sample_av.mp4}: a 6 s, 320x180, 15 fps test pattern
 * (ffmpeg {@code testsrc2}) with a 440 Hz AAC tone at 22050 Hz mono.
 * RGB565 frames into a caller's buffer, exact seeks, sound resampled and
 * aligned with the picture (also when it is read ahead of the video), and
 * the per-program decoder registry. Control: {@code sample.mp4} has no sound.
 */
class VideoDecoderAvTest {

    private static Path resource(String name) throws Exception {
        URL url = VideoDecoderAvTest.class.getClassLoader().getResource("video/" + name);
        assertNotNull(url, name + " missing from test resources");
        return Paths.get(url.toURI());
    }

    @Test
    void rgb565_frames_go_into_the_callers_buffer() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(resource("sample_av.mp4"), 160, 90, VideoDecoder.FORMAT_RGB565)) {
            assertEquals(2, d.bytesPerPixel());
            assertEquals(160 * 90 * 2, d.frameBytes());
            VideoDecoder.VideoInfo i = d.info();
            assertEquals(320, i.width);
            assertEquals(180, i.height);
            assertEquals(15, i.fpsNum / Math.max(1, i.fpsDen));
            assertTrue(Math.abs(i.durationMs - 6000) <= 100, "duration " + i.durationMs);

            byte[] buf = new byte[d.frameBytes()];
            long first = d.nextInto(buf);
            assertEquals(0, first, "the first frame is at 0");
            // testsrc2 is colourful: many distinct RGB565 values.
            java.util.Set<Integer> colours = new java.util.HashSet<>();
            for (int p = 0; p < buf.length; p += 2) colours.add((buf[p] & 0xFF) | (buf[p + 1] & 0xFF) << 8);
            assertTrue(colours.size() > 20, "a colourful frame: " + colours.size() + " colours");
            byte[] copy = buf.clone();
            long second = d.nextInto(buf);
            assertTrue(second > first && second <= 70, "15 fps: the next frame about 67 ms later, got " + second);
            assertFalse(Arrays.equals(copy, buf), "the pattern moves between frames");
            assertThrows(IllegalArgumentException.class, () -> d.nextInto(new byte[10]), "a short buffer is refused");

            int frames = 2;
            while (d.nextInto(buf) >= 0) frames++;
            assertTrue(frames >= 88 && frames <= 92, "6 s at 15 fps: " + frames + " frames");
            assertEquals(-1, d.nextInto(buf), "after the end: -1 again");
        }
    }

    @Test
    void seeks_are_exact() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(resource("sample_av.mp4"), 64, 36, VideoDecoder.FORMAT_RGB565)) {
            byte[] buf = new byte[d.frameBytes()];
            for (long target : new long[]{3000, 1234, 5900, 0}) {
                d.seek(target);
                long pts = d.nextInto(buf);
                assertTrue(pts >= target - 34 && pts <= target + 67,
                        "seek to " + target + " lands on the frame showing then, got " + pts);
            }
            // Past the end: nothing more.
            d.seek(60_000);
            assertEquals(-1, d.nextInto(buf));
            // And back.
            d.seek(0);
            assertEquals(0, d.nextInto(buf));
        }
    }

    @Test
    void sound_is_resampled_aligned_and_readable_ahead_of_the_picture() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(resource("sample_av.mp4"), 64, 36, VideoDecoder.FORMAT_RGB565)) {
            assertTrue(d.hasAudio());
            assertArrayEquals(new int[]{22050, 1}, d.sourceAudioFormat());
            assertEquals(0, d.readAudio(new byte[100], 100), "off until enabled");
            assertTrue(d.enableAudio(22050, 1));
            assertEquals(44100, d.audioBytesPerSecond());

            // Two seconds of sound before any picture: the demuxer reads ahead and holds the video.
            byte[] pcm = new byte[44100 * 2];
            int got = d.readAudio(pcm, pcm.length);
            assertEquals(pcm.length, got, "two seconds of sound read ahead");
            double rms = rms(pcm, got);
            assertTrue(rms > 1000, "the tone is there: rms " + rms);
            assertEquals(440, zeroCrossingHz(pcm, got, 22050), 15, "a 440 Hz tone");
            // The picture still starts at 0.
            byte[] buf = new byte[d.frameBytes()];
            assertEquals(0, d.nextInto(buf), "the held video packets come first");

            // The rest of the sound: about 6 s in all.
            long total = got;
            byte[] chunk = new byte[8192];
            int n;
            while ((n = d.readAudio(chunk, chunk.length)) > 0) total += n;
            double secs = total / 44100.0;
            assertTrue(secs > 5.7 && secs < 6.3, "about 6 s of sound: " + secs);

            // After a seek the sound starts at the target: about 2 s left from 4 s.
            d.seek(4000);
            long after = 0;
            while ((n = d.readAudio(chunk, chunk.length)) > 0) after += n;
            double left = after / 44100.0;
            assertTrue(left > 1.8 && left < 2.2, "2 s of sound after a seek to 4 s: " + left);
            long pts = d.nextInto(buf);
            assertTrue(pts >= 3966 && pts <= 4067, "and the picture there too: " + pts);

            // Stereo at 48 kHz: the same tone, twice the channels.
            assertTrue(d.enableAudio(48000, 2));
            d.seek(0);
            byte[] st = new byte[48000 * 4];
            assertEquals(st.length, d.readAudio(st, st.length));
            assertTrue(rms(st, st.length) > 1000);
        }
    }

    @Test
    void a_video_without_sound_says_so() throws Exception {
        try (VideoDecoder d = VideoDecoder.open(resource("sample.mp4"), 64, 64, VideoDecoder.FORMAT_RGBA8888)) {
            assertFalse(d.hasAudio());
            assertArrayEquals(new int[]{0, 0}, d.sourceAudioFormat());
            assertFalse(d.enableAudio(22050, 1));
            assertEquals(64 * 64 * 4, d.frameBytes());
            assertNotNull(d.next());
        }
    }

    @Test
    void the_registry_keeps_decoders_to_their_program() throws Exception {
        VideoDecoderRegistry r = new VideoDecoderRegistry();
        try {
            int a = r.open(resource("sample_av.mp4"), 32, 18, VideoDecoder.FORMAT_RGB565, 7);
            int b = r.open(resource("sample_av.mp4"), 32, 18, VideoDecoder.FORMAT_RGB565, 8);
            assertTrue(a > 0 && b > 0 && a != b);
            assertNotNull(r.get(a, 7));
            assertNull(r.get(a, 8), "another program's handle is refused");
            assertNotNull(r.get(a, 0), "the host itself may use any");
            assertEquals(1, r.closeOwnedBy(7), "program 7 exited");
            assertNull(r.get(a, 7));
            assertNotNull(r.get(b, 8));
            // At most MAX_OPEN at once.
            for (int i = r.size(); i < VideoDecoderRegistry.MAX_OPEN; i++) {
                assertTrue(r.open(resource("sample_av.mp4"), 16, 16, VideoDecoder.FORMAT_INDEXED8, 9) > 0);
            }
            assertEquals(-1, r.open(resource("sample_av.mp4"), 16, 16, VideoDecoder.FORMAT_INDEXED8, 9), "one too many");
            assertEquals(-1, r.open(Path.of("does-not-exist.mp4"), 16, 16, VideoDecoder.FORMAT_INDEXED8, 9));
        } finally {
            r.closeAll();
        }
        assertEquals(0, r.size());
    }

    private static double rms(byte[] pcm, int n) {
        double sum = 0;
        int samples = n / 2;
        for (int i = 0; i < samples; i++) {
            short s = (short) ((pcm[i * 2] & 0xFF) | pcm[i * 2 + 1] << 8);
            sum += (double) s * s;
        }
        return Math.sqrt(sum / Math.max(1, samples));
    }

    private static double zeroCrossingHz(byte[] pcm, int n, int rate) {
        int samples = n / 2;
        int crossings = 0;
        short prev = 0;
        for (int i = 0; i < samples; i++) {
            short s = (short) ((pcm[i * 2] & 0xFF) | pcm[i * 2 + 1] << 8);
            if (i > 0 && (prev < 0) != (s < 0)) crossings++;
            prev = s;
        }
        return crossings / 2.0 / (samples / (double) rate);
    }
}
