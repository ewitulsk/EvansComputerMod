package com.example.evanscomputermod.computer.video;

import org.bytedeco.ffmpeg.avcodec.AVCodec;
import org.bytedeco.ffmpeg.avcodec.AVCodecContext;
import org.bytedeco.ffmpeg.avcodec.AVCodecParameters;
import org.bytedeco.ffmpeg.avcodec.AVPacket;
import org.bytedeco.ffmpeg.avformat.AVFormatContext;
import org.bytedeco.ffmpeg.avformat.AVStream;
import org.bytedeco.ffmpeg.avutil.AVFrame;
import org.bytedeco.ffmpeg.avutil.AVRational;
import org.bytedeco.ffmpeg.swscale.SwsContext;
import org.bytedeco.javacpp.BytePointer;
import org.bytedeco.javacpp.IntPointer;
import org.bytedeco.javacpp.PointerPointer;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.Path;
import java.util.Objects;

import static org.bytedeco.ffmpeg.global.avcodec.*;
import static org.bytedeco.ffmpeg.global.avformat.*;
import static org.bytedeco.ffmpeg.global.avutil.*;
import static org.bytedeco.ffmpeg.global.swscale.*;

/**
 * Low-level FFmpeg-based MP4 decoder. Two output pixel formats:
 * <ul>
 *   <li>{@link #FORMAT_INDEXED8}: swscale destination
 *       {@code AV_PIX_FMT_RGB8} (packed 3:3:2). 1 byte per pixel; matches
 *       the {@link Rgb332Palette} install-once palette so no runtime
 *       quantization is needed.</li>
 *   <li>{@link #FORMAT_RGBA8888}: swscale destination
 *       {@code AV_PIX_FMT_RGBA}. 4 bytes per pixel; full color, ready
 *       for direct upload to the screen cluster's RGBA framebuffer.</li>
 * </ul>
 *
 * <p>Uses libavformat + libavcodec + libswscale. Frames are decoded from the
 * file's first video stream, scaled/resampled to {@code targetW × targetH},
 * and converted to the chosen destination format.
 *
 * <p>This class is not thread-safe. A decoder instance is owned by exactly one
 * caller — callers must serialize {@link #next()}, {@link #seek(long)}, and
 * {@link #close()}. Audio streams are ignored.
 */
public final class VideoDecoder implements Closeable {

    /** Destination pixel format: 1 byte/pixel, RGB332 palette indices. */
    public static final int FORMAT_INDEXED8 = 0;
    /** Destination pixel format: 4 bytes/pixel, packed RGBA. */
    public static final int FORMAT_RGBA8888 = 1;

    /** Static descriptor returned by {@link #info()}. */
    public static final class VideoInfo {
        public final int width;
        public final int height;
        public final int fpsNum;
        public final int fpsDen;
        public final long frameCount;
        public final long durationMs;

        public VideoInfo(int width, int height, int fpsNum, int fpsDen,
                          long frameCount, long durationMs) {
            this.width = width;
            this.height = height;
            this.fpsNum = fpsNum;
            this.fpsDen = fpsDen;
            this.frameCount = frameCount;
            this.durationMs = durationMs;
        }
    }

    /** Single decoded frame ready for blit. */
    public static final class DecodedFrame {
        /** {@code targetW * targetH} indexed bytes (one per pixel). */
        public final byte[] indexed;
        /** Presentation timestamp in milliseconds, relative to stream start. */
        public final long ptsMs;

        public DecodedFrame(byte[] indexed, long ptsMs) {
            this.indexed = indexed;
            this.ptsMs = ptsMs;
        }
    }

    private final int targetW;
    private final int targetH;
    private final int outputFormat;
    private final int bytesPerPixel;

    private AVFormatContext formatCtx;
    private AVCodecContext codecCtx;
    private SwsContext swsCtx;
    private AVFrame srcFrame;
    private AVFrame dstFrame;
    private AVPacket packet;

    private int videoStreamIndex = -1;
    private int tbNum;               // video stream time_base numerator
    private int tbDen;               // video stream time_base denominator
    private long totalFrames;
    private long durationMs;
    private int srcWidth;
    private int srcHeight;
    private int fpsNum;
    private int fpsDen;

    /** {@code true} once the decoder has been flushed and drained. */
    private boolean eof;
    /** {@code true} once we have sent the final flush packet. */
    private boolean flushed;

    /** Reusable output buffer. Size equals {@code targetW * targetH}. */
    private final byte[] outBuffer;

    private VideoDecoder(int targetW, int targetH, int outputFormat) {
        if (targetW <= 0 || targetH <= 0) {
            throw new IllegalArgumentException("target size must be positive");
        }
        if (outputFormat != FORMAT_INDEXED8 && outputFormat != FORMAT_RGBA8888) {
            throw new IllegalArgumentException("unsupported output format: " + outputFormat);
        }
        this.targetW = targetW;
        this.targetH = targetH;
        this.outputFormat = outputFormat;
        this.bytesPerPixel = (outputFormat == FORMAT_RGBA8888) ? 4 : 1;
        this.outBuffer = new byte[targetW * targetH * this.bytesPerPixel];
    }

    /**
     * Opens the given MP4 file in indexed8 (RGB332) output mode. Convenience
     * wrapper around {@link #open(Path, int, int, int)} that defaults to
     * {@link #FORMAT_INDEXED8}.
     */
    public static VideoDecoder open(Path mp4Path, int targetW, int targetH) throws IOException {
        return open(mp4Path, targetW, targetH, FORMAT_INDEXED8);
    }

    /**
     * Opens the given MP4 file, sets up the first video stream's H.264 decoder,
     * and configures libswscale to output {@code targetW × targetH} frames in
     * either {@code AV_PIX_FMT_RGB8} (FORMAT_INDEXED8) or
     * {@code AV_PIX_FMT_RGBA} (FORMAT_RGBA8888). Throws {@link IOException}
     * on any failure; on exception, all native resources allocated so far
     * are released.
     */
    public static VideoDecoder open(Path mp4Path, int targetW, int targetH, int outputFormat) throws IOException {
        Objects.requireNonNull(mp4Path, "mp4Path");
        VideoDecoder d = new VideoDecoder(targetW, targetH, outputFormat);
        try {
            d.openInternal(mp4Path.toAbsolutePath().toString());
            return d;
        } catch (Throwable t) {
            d.close();
            if (t instanceof IOException io) throw io;
            // Preserve the root cause message so the registry's INFO log line
            // shows something actionable instead of a generic "failed".
            String root = t.getClass().getSimpleName()
                    + (t.getMessage() != null ? ": " + t.getMessage() : "");
            IOException wrapped = new IOException(
                    "VideoDecoder.open failed (" + root + ")", t);
            throw wrapped;
        }
    }

    private void openInternal(String path) throws IOException {
        formatCtx = new AVFormatContext(null);
        int rc = avformat_open_input(formatCtx, path, null, null);
        if (rc < 0) {
            formatCtx = null;
            throw new IOException("avformat_open_input(" + path + ") failed: " + ffErr(rc));
        }

        if (avformat_find_stream_info(formatCtx, (PointerPointer<?>) null) < 0) {
            throw new IOException("avformat_find_stream_info failed");
        }

        videoStreamIndex = -1;
        int nb = formatCtx.nb_streams();
        AVStream videoStream = null;
        for (int i = 0; i < nb; i++) {
            AVStream s = formatCtx.streams(i);
            if (s.codecpar().codec_type() == AVMEDIA_TYPE_VIDEO) {
                videoStreamIndex = i;
                videoStream = s;
                break;
            }
        }
        if (videoStream == null) {
            throw new IOException("no video stream found");
        }

        AVCodecParameters codecpar = videoStream.codecpar();
        AVCodec codec = avcodec_find_decoder(codecpar.codec_id());
        if (codec == null) {
            throw new IOException("no decoder for codec_id=" + codecpar.codec_id());
        }
        codecCtx = avcodec_alloc_context3(codec);
        if (codecCtx == null) {
            throw new IOException("avcodec_alloc_context3 failed");
        }
        if (avcodec_parameters_to_context(codecCtx, codecpar) < 0) {
            throw new IOException("avcodec_parameters_to_context failed");
        }
        if (avcodec_open2(codecCtx, codec, (PointerPointer<?>) null) < 0) {
            throw new IOException("avcodec_open2 failed");
        }

        srcWidth = codecCtx.width();
        srcHeight = codecCtx.height();
        if (srcWidth <= 0 || srcHeight <= 0) {
            throw new IOException("source has invalid dimensions "
                    + srcWidth + "x" + srcHeight);
        }

        // Average frame rate (may be 0/0 for variable-fps streams).
        AVRational fr = videoStream.avg_frame_rate();
        fpsNum = fr.num();
        fpsDen = fr.den();
        if (fpsDen == 0) {
            // Fall back to r_frame_rate.
            AVRational rf = videoStream.r_frame_rate();
            fpsNum = rf.num();
            fpsDen = rf.den();
        }

        // Time base for pts→ms conversion (copy primitives only — do not
        // retain any AVRational Java wrapper because that's a native handle).
        AVRational tb = videoStream.time_base();
        tbNum = tb.num();
        tbDen = tb.den();

        totalFrames = videoStream.nb_frames();
        long streamDurTicks = videoStream.duration();
        if (streamDurTicks > 0 && tb.den() > 0) {
            durationMs = (streamDurTicks * 1000L * tb.num()) / tb.den();
        } else if (formatCtx.duration() > 0) {
            durationMs = (formatCtx.duration() * 1000L) / AV_TIME_BASE;
        } else {
            durationMs = 0;
        }

        srcFrame = av_frame_alloc();
        dstFrame = av_frame_alloc();
        packet = av_packet_alloc();
        if (srcFrame == null || dstFrame == null || packet == null) {
            throw new IOException("av_frame_alloc / av_packet_alloc failed");
        }

        // Allocate destination frame buffer sized to target. The pixel
        // format depends on outputFormat: RGB8 for indexed8 (1 bpp,
        // 3:3:2 packed) or RGBA for rgba8888 (4 bpp, packed RGBA).
        // av_frame_get_buffer ties the buffer lifetime to the frame itself,
        // so av_frame_free() later handles the cleanup safely.
        int avDstFormat = (outputFormat == FORMAT_RGBA8888) ? AV_PIX_FMT_RGBA : AV_PIX_FMT_RGB8;
        dstFrame.format(avDstFormat);
        dstFrame.width(targetW);
        dstFrame.height(targetH);
        if (av_frame_get_buffer(dstFrame, 1) < 0) {
            throw new IOException("av_frame_get_buffer failed");
        }

        // Build swscale context. Source params come from the decoder.
        swsCtx = sws_getContext(
                srcWidth, srcHeight, codecCtx.pix_fmt(),
                targetW, targetH, avDstFormat,
                SWS_BILINEAR,
                null, null, (double[]) null);
        if (swsCtx == null) {
            throw new IOException("sws_getContext failed");
        }
    }

    /** Returns the decoder's static metadata. */
    public VideoInfo info() {
        return new VideoInfo(srcWidth, srcHeight, fpsNum, fpsDen, totalFrames, durationMs);
    }

    /** Target resolution width (pixels). */
    public int targetWidth() { return targetW; }

    /** Target resolution height (pixels). */
    public int targetHeight() { return targetH; }

    /** Output pixel format ({@link #FORMAT_INDEXED8} or {@link #FORMAT_RGBA8888}). */
    public int outputFormat() { return outputFormat; }

    /** Bytes per output pixel: 1 for indexed8, 4 for rgba8888. */
    public int bytesPerPixel() { return bytesPerPixel; }

    /**
     * Reads packets until the decoder produces the next video frame, rescales
     * it to {@code targetW × targetH}, and copies the indexed bytes into a
     * fresh {@code byte[]}. Returns {@code null} once the stream is fully
     * drained.
     */
    public DecodedFrame next() throws IOException {
        if (eof) return null;

        while (true) {
            int rc = avcodec_receive_frame(codecCtx, srcFrame);
            if (rc == 0) {
                return buildFrame();
            }
            if (rc == AVERROR_EOF()) {
                eof = true;
                return null;
            }
            if (rc != -11 /* EAGAIN */ && rc != AVERROR_EAGAIN()) {
                throw new IOException("avcodec_receive_frame failed: " + ffErr(rc));
            }

            // Need more input.
            if (flushed) {
                // Already asked the decoder to drain; nothing more is coming.
                eof = true;
                return null;
            }

            int rf = av_read_frame(formatCtx, packet);
            if (rf == AVERROR_EOF()) {
                avcodec_send_packet(codecCtx, (AVPacket) null);
                flushed = true;
                continue;
            }
            if (rf < 0) {
                throw new IOException("av_read_frame failed: " + ffErr(rf));
            }
            if (packet.stream_index() == videoStreamIndex) {
                int sp = avcodec_send_packet(codecCtx, packet);
                if (sp < 0 && sp != AVERROR_EAGAIN() && sp != AVERROR_EOF()) {
                    av_packet_unref(packet);
                    throw new IOException("avcodec_send_packet failed: " + ffErr(sp));
                }
            }
            av_packet_unref(packet);
        }
    }

    private DecodedFrame buildFrame() throws IOException {
        int slices = sws_scale(swsCtx,
                srcFrame.data(), srcFrame.linesize(), 0, srcHeight,
                dstFrame.data(), dstFrame.linesize());
        if (slices <= 0) {
            throw new IOException("sws_scale failed: " + slices);
        }

        BytePointer data0 = dstFrame.data(0);
        int stride = dstFrame.linesize(0);
        int rowBytes = targetW * bytesPerPixel;
        if (stride == rowBytes) {
            data0.position(0).get(outBuffer, 0, rowBytes * targetH);
        } else {
            // Copy row-by-row when swscale has inserted padding.
            for (int y = 0; y < targetH; y++) {
                data0.position((long) y * stride).get(outBuffer, y * rowBytes, rowBytes);
            }
        }
        data0.position(0);

        long pts = srcFrame.pts();
        if (pts == AV_NOPTS_VALUE) pts = srcFrame.best_effort_timestamp();
        long ptsMs;
        if (pts == AV_NOPTS_VALUE) {
            ptsMs = 0;
        } else {
            ptsMs = (pts * 1000L * tbNum) / Math.max(1, tbDen);
        }

        // Return a defensive copy so the caller can hold on to the frame after
        // we overwrite outBuffer on the next next() call.
        byte[] copy = new byte[outBuffer.length];
        System.arraycopy(outBuffer, 0, copy, 0, outBuffer.length);
        return new DecodedFrame(copy, ptsMs);
    }

    /**
     * Seeks the stream as close as possible to the given pts in milliseconds,
     * then flushes the decoder so the next {@link #next()} call returns a
     * frame at or after the requested time.
     */
    public void seek(long ptsMs) throws IOException {
        if (ptsMs < 0) ptsMs = 0;
        long ticks = (ptsMs * Math.max(1, tbDen)) / (1000L * Math.max(1, tbNum));
        int rc = av_seek_frame(formatCtx, videoStreamIndex, ticks, AVSEEK_FLAG_BACKWARD);
        if (rc < 0) throw new IOException("av_seek_frame failed: " + ffErr(rc));
        avcodec_flush_buffers(codecCtx);
        eof = false;
        flushed = false;
    }

    @Override
    public void close() {
        if (swsCtx != null) {
            sws_freeContext(swsCtx);
            swsCtx = null;
        }
        if (dstFrame != null) {
            av_frame_free(dstFrame);
            dstFrame = null;
        }
        if (srcFrame != null) {
            av_frame_free(srcFrame);
            srcFrame = null;
        }
        if (packet != null) {
            av_packet_free(packet);
            packet = null;
        }
        if (codecCtx != null) {
            avcodec_free_context(codecCtx);
            codecCtx = null;
        }
        if (formatCtx != null) {
            avformat_close_input(formatCtx);
            formatCtx = null;
        }
    }

    private static String ffErr(int code) {
        BytePointer buf = new BytePointer(128);
        av_strerror(code, buf, 128);
        String s = buf.getString();
        buf.deallocate();
        return code + " (" + s + ")";
    }
}
