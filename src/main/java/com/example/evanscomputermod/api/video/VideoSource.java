package com.example.evanscomputermod.api.video;

import java.io.Closeable;
import java.io.IOException;

/**
 * One open video file, decoded by a {@link VideoBackend}: frames scaled to the
 * size it was opened at, in a display pixel format, and (once
 * {@link #enableAudio} is on) its sound as signed 16-bit little-endian PCM.
 *
 * <p>Contract (what programs rely on through the {@code video_*} imports):
 * {@link #seek} is exact (the next frame is the one showing at the target); the
 * first audio byte after open or a seek is at the stream position (0 or the
 * target), so a player's audio position is {@code start + bytes / bytesPerMs};
 * audio can be read ahead of the picture and the other way round.
 */
public interface VideoSource extends Closeable {

    /** Pixel formats; the same numbers as the displays' ({@code TerminalDisplay.PIXEL_FORMAT_*}). */
    int FORMAT_INDEXED8 = 0;
    int FORMAT_RGBA8888 = 1;
    int FORMAT_RGB565 = 2;

    /** The file's own size, frame rate, frame count and length. */
    final class Info {
        public final int width;
        public final int height;
        public final int fpsNum;
        public final int fpsDen;
        public final long frameCount;
        public final long durationMs;

        public Info(int width, int height, int fpsNum, int fpsDen, long frameCount, long durationMs) {
            this.width = width;
            this.height = height;
            this.fpsNum = fpsNum;
            this.fpsDen = fpsDen;
            this.frameCount = frameCount;
            this.durationMs = durationMs;
        }
    }

    /** One decoded frame. */
    final class Frame {
        /** {@code targetW * targetH * bytesPerPixel} bytes in the output format. */
        public final byte[] pixels;
        /** Presentation time in ms from the start. */
        public final long ptsMs;

        public Frame(byte[] pixels, long ptsMs) {
            this.pixels = pixels;
            this.ptsMs = ptsMs;
        }
    }

    Info info();

    int targetWidth();

    int targetHeight();

    /** {@link #FORMAT_INDEXED8}, {@link #FORMAT_RGBA8888} or {@link #FORMAT_RGB565}. */
    int outputFormat();

    int bytesPerPixel();

    /** Bytes in one output frame. */
    int frameBytes();

    /** The next frame in a fresh array, or null at the end. */
    Frame next() throws IOException;

    /** The next frame into {@code dst} (at least {@link #frameBytes()} long): its pts, or -1 at the end. */
    long nextInto(byte[] dst) throws IOException;

    /** Exact seek to {@code ptsMs}; audio restarts there. */
    void seek(long ptsMs) throws IOException;

    boolean hasAudio();

    /** The audio stream's own {rate, channels}, or {0, 0} without one. */
    int[] sourceAudioFormat();

    /** Decode the sound as s16le at {@code rate} Hz with {@code channels}; false without an audio stream. */
    boolean enableAudio(int rate, int channels) throws IOException;

    boolean audioEnabled();

    /** {@code rate * channels * 2}, 0 without audio. */
    int audioBytesPerSecond();

    /** Up to {@code max} bytes (whole sample frames) of PCM into {@code dst}; 0 at the end of the sound. */
    int readAudio(byte[] dst, int max) throws IOException;

    @Override
    void close();
}
