package com.example.evanscomputermod.computer.video;

import org.bytedeco.ffmpeg.avcodec.AVCodec;
import org.bytedeco.ffmpeg.avcodec.AVCodecContext;
import org.bytedeco.ffmpeg.avcodec.AVCodecParameters;
import org.bytedeco.ffmpeg.avcodec.AVPacket;
import org.bytedeco.ffmpeg.avformat.AVFormatContext;
import org.bytedeco.ffmpeg.avformat.AVStream;
import org.bytedeco.ffmpeg.avutil.AVChannelLayout;
import org.bytedeco.ffmpeg.avutil.AVFrame;
import org.bytedeco.ffmpeg.avutil.AVRational;
import org.bytedeco.ffmpeg.swresample.SwrContext;
import org.bytedeco.ffmpeg.swscale.SwsContext;
import org.bytedeco.javacpp.BytePointer;
import org.bytedeco.javacpp.PointerPointer;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Objects;

import static org.bytedeco.ffmpeg.global.avcodec.*;
import static org.bytedeco.ffmpeg.global.avformat.*;
import static org.bytedeco.ffmpeg.global.avutil.*;
import static org.bytedeco.ffmpeg.global.swresample.*;
import static org.bytedeco.ffmpeg.global.swscale.*;

/**
 * FFmpeg-based video file decoder (MP4, MKV, WebM, MOV: whatever libavformat
 * opens with a codec libavcodec has). Three output pixel formats:
 * <ul>
 *   <li>{@link #FORMAT_INDEXED8}: swscale destination
 *       {@code AV_PIX_FMT_RGB8} (packed 3:3:2). 1 byte per pixel; matches
 *       the {@link Rgb332Palette} install-once palette so no runtime
 *       quantization is needed.</li>
 *   <li>{@link #FORMAT_RGBA8888}: swscale destination
 *       {@code AV_PIX_FMT_RGBA}. 4 bytes per pixel; full color.</li>
 *   <li>{@link #FORMAT_RGB565}: swscale destination
 *       {@code AV_PIX_FMT_RGB565LE}. 2 bytes per pixel, the displays' and
 *       UI canvases' rgb565 (little-endian {@code rrrrrggggggbbbbb}).</li>
 * </ul>
 *
 * <p>Frames come from the file's first video stream, scaled to
 * {@code targetW × targetH}. Audio is off until {@link #enableAudio}: then the
 * first audio stream is decoded and resampled to signed 16-bit little-endian
 * PCM at the asked rate and channel count, queued for {@link #readAudio}. The
 * demuxer keeps a queue of video packets so audio can be read ahead of the
 * video (and the other way round) whatever the file's interleaving.
 *
 * <p>Audio stays aligned with the video clock: after {@link #open} and every
 * {@link #seek} the first byte {@link #readAudio} returns is at the stream
 * position (0, or the seek target), padded with silence or trimmed as needed,
 * so a player's audio position is just {@code start + bytes / bytesPerMs}.
 *
 * <p>{@link #seek} is exact: frames before the target are decoded and dropped,
 * so the next frame is the one showing at the target.
 *
 * <p>All public methods are synchronized; a decoder is normally driven by one
 * program thread.
 */
public final class VideoDecoder implements Closeable {

    /** Destination pixel format: 1 byte/pixel, RGB332 palette indices. */
    public static final int FORMAT_INDEXED8 = 0;
    /** Destination pixel format: 4 bytes/pixel, packed RGBA. */
    public static final int FORMAT_RGBA8888 = 1;
    /** Destination pixel format: 2 bytes/pixel, RGB565 little-endian. */
    public static final int FORMAT_RGB565 = 2;

    /** Most audio kept queued (decoded, not yet read): older audio is dropped past this. */
    static final int MAX_AUDIO_QUEUE_MS = 10_000;
    /** Most video packets held while audio is read ahead of the video. */
    static final int MAX_VIDEO_PACKETS = 600;

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
        /** {@code targetW * targetH * bytesPerPixel} bytes in the output format. */
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
    private long startTicks;         // video stream start_time (ticks), 0 if unknown
    private int srcWidth;
    private int srcHeight;
    private int fpsNum;
    private int fpsDen;

    /** {@code true} once the video decoder has been flushed and drained. */
    private boolean eof;
    /** {@code true} once the demuxer reached the end of the file (decoders flushed). */
    private boolean demuxEof;
    /** Video frames before this pts (ms) are dropped (exact seek); -1 = none. */
    private long dropBeforeMs = -1;
    /** Video packets read while looking for audio, oldest first. */
    private final ArrayDeque<AVPacket> videoPackets = new ArrayDeque<>();

    // --- audio (null / -1 until enableAudio) ---
    private int audioStreamIndex = -1;
    private AVCodecContext audioCtx;
    private SwrContext swr;
    private AVFrame audioFrame;
    private int audioTbNum, audioTbDen;
    private int outRate, outChannels;
    private boolean audioEnabled;
    /** The stream position the next queued byte must start at, until the first chunk after open/seek is queued. */
    private long audioAlignMs = 0;
    private boolean audioAligned;
    private final ArrayDeque<byte[]> audioQueue = new ArrayDeque<>();
    private int audioHeadOffset;
    private long audioQueued;
    /** swresample's output; it allocates the buffer for each converted frame. */
    private AVFrame audioOutFrame;

    /** Reusable output buffer. Size equals {@code targetW * targetH * bytesPerPixel}. */
    private final byte[] outBuffer;

    private VideoDecoder(int targetW, int targetH, int outputFormat) {
        if (targetW <= 0 || targetH <= 0) {
            throw new IllegalArgumentException("target size must be positive");
        }
        if (outputFormat != FORMAT_INDEXED8 && outputFormat != FORMAT_RGBA8888 && outputFormat != FORMAT_RGB565) {
            throw new IllegalArgumentException("unsupported output format: " + outputFormat);
        }
        this.targetW = targetW;
        this.targetH = targetH;
        this.outputFormat = outputFormat;
        this.bytesPerPixel = switch (outputFormat) {
            case FORMAT_RGBA8888 -> 4;
            case FORMAT_RGB565 -> 2;
            default -> 1;
        };
        this.outBuffer = new byte[targetW * targetH * this.bytesPerPixel];
    }

    /**
     * Opens the given video file in indexed8 (RGB332) output mode. Convenience
     * wrapper around {@link #open(Path, int, int, int)} that defaults to
     * {@link #FORMAT_INDEXED8}.
     */
    public static VideoDecoder open(Path mp4Path, int targetW, int targetH) throws IOException {
        return open(mp4Path, targetW, targetH, FORMAT_INDEXED8);
    }

    /**
     * Opens the given video file, sets up its first video stream's decoder,
     * and configures libswscale to output {@code targetW × targetH} frames in
     * the given format. Throws {@link IOException} on any failure; on
     * exception, all native resources allocated so far are released.
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
            throw new IOException("VideoDecoder.open failed (" + root + ")", t);
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
            int type = s.codecpar().codec_type();
            if (type == AVMEDIA_TYPE_VIDEO && videoStream == null) {
                videoStreamIndex = i;
                videoStream = s;
            } else if (type == AVMEDIA_TYPE_AUDIO && audioStreamIndex < 0) {
                audioStreamIndex = i;
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
        startTicks = videoStream.start_time() == AV_NOPTS_VALUE ? 0 : videoStream.start_time();

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

        // Allocate destination frame buffer sized to target. av_frame_get_buffer
        // ties the buffer lifetime to the frame itself, so av_frame_free() later
        // handles the cleanup safely.
        int avDstFormat = switch (outputFormat) {
            case FORMAT_RGBA8888 -> AV_PIX_FMT_RGBA;
            case FORMAT_RGB565 -> AV_PIX_FMT_RGB565LE;
            default -> AV_PIX_FMT_RGB8;
        };
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
    public synchronized VideoInfo info() {
        return new VideoInfo(srcWidth, srcHeight, fpsNum, fpsDen, totalFrames, durationMs);
    }

    /** Target resolution width (pixels). */
    public int targetWidth() { return targetW; }

    /** Target resolution height (pixels). */
    public int targetHeight() { return targetH; }

    /** Output pixel format ({@link #FORMAT_INDEXED8}, {@link #FORMAT_RGBA8888} or {@link #FORMAT_RGB565}). */
    public int outputFormat() { return outputFormat; }

    /** Bytes per output pixel: 1 for indexed8, 2 for rgb565, 4 for rgba8888. */
    public int bytesPerPixel() { return bytesPerPixel; }

    /** Bytes in one output frame. */
    public int frameBytes() { return outBuffer.length; }

    /** Whether the file has an audio stream (whether or not it is enabled). */
    public synchronized boolean hasAudio() { return audioStreamIndex >= 0; }

    /** The audio stream's own sample rate and channel count, or {0, 0} without one. */
    public synchronized int[] sourceAudioFormat() {
        if (audioStreamIndex < 0 || formatCtx == null) return new int[] {0, 0};
        AVCodecParameters p = formatCtx.streams(audioStreamIndex).codecpar();
        return new int[] {p.sample_rate(), p.ch_layout().nb_channels()};
    }

    /**
     * Reads packets until the decoder produces the next video frame, rescales
     * it to {@code targetW × targetH}, and copies the pixels into a fresh
     * {@code byte[]}. Returns {@code null} once the stream is fully drained.
     */
    public synchronized DecodedFrame next() throws IOException {
        long pts = decodeNext();
        if (pts < 0) return null;
        byte[] copy = new byte[outBuffer.length];
        System.arraycopy(outBuffer, 0, copy, 0, outBuffer.length);
        return new DecodedFrame(copy, pts);
    }

    /**
     * Like {@link #next()}, but writes the frame into {@code dst} (at least
     * {@link #frameBytes()} long). Returns the frame's pts in ms, or -1 at the
     * end of the stream.
     */
    public synchronized long nextInto(byte[] dst) throws IOException {
        if (dst.length < outBuffer.length) throw new IllegalArgumentException("buffer too small");
        long pts = decodeNext();
        if (pts >= 0) System.arraycopy(outBuffer, 0, dst, 0, outBuffer.length);
        return pts;
    }

    /** Decode the next shown frame into {@link #outBuffer}; its pts, or -1 at the end. */
    private long decodeNext() throws IOException {
        if (eof || codecCtx == null) return -1;
        while (true) {
            int rc = avcodec_receive_frame(codecCtx, srcFrame);
            if (rc == 0) {
                long ptsMs = framePtsMs();
                if (dropBeforeMs >= 0 && ptsMs + halfFrameMs() < dropBeforeMs) {
                    continue; // exact seek: not there yet
                }
                dropBeforeMs = -1;
                scaleFrame();
                return ptsMs;
            }
            if (rc == AVERROR_EOF()) {
                eof = true;
                return -1;
            }
            if (rc != -11 /* EAGAIN */ && rc != AVERROR_EAGAIN()) {
                throw new IOException("avcodec_receive_frame failed: " + ffErr(rc));
            }
            // Need more input: a held video packet first, then the file.
            AVPacket held = videoPackets.pollFirst();
            if (held != null) {
                sendVideo(held);
                av_packet_free(held);
                continue;
            }
            if (demuxEof) {
                eof = true;
                return -1;
            }
            readPacket(true);
        }
    }

    /**
     * Read one packet from the file: audio is decoded into the audio queue;
     * video is sent to the decoder ({@code videoNow}) or held for later. At
     * the end of the file both decoders are flushed.
     */
    private void readPacket(boolean videoNow) throws IOException {
        int rf = av_read_frame(formatCtx, packet);
        if (rf == AVERROR_EOF()) {
            demuxEof = true;
            // Held packets still go first; the flush packet follows them.
            if (videoPackets.isEmpty()) {
                avcodec_send_packet(codecCtx, (AVPacket) null);
            } else {
                videoPackets.addLast(av_packet_alloc()); // empty packet = flush
            }
            if (audioEnabled) {
                avcodec_send_packet(audioCtx, (AVPacket) null);
                drainAudio();
            }
            return;
        }
        if (rf < 0) {
            throw new IOException("av_read_frame failed: " + ffErr(rf));
        }
        try {
            int si = packet.stream_index();
            if (si == videoStreamIndex) {
                if (videoNow && videoPackets.isEmpty()) {
                    sendVideo(packet);
                } else {
                    videoPackets.addLast(av_packet_clone(packet));
                }
            } else if (audioEnabled && si == audioStreamIndex) {
                int sp = avcodec_send_packet(audioCtx, packet);
                if (sp >= 0 || sp == AVERROR_EAGAIN()) drainAudio();
            }
        } finally {
            av_packet_unref(packet);
        }
    }

    private void sendVideo(AVPacket p) throws IOException {
        // An empty held packet (no data, size 0) is the flush that followed the file's end;
        // libavcodec takes it as one.
        int sp = avcodec_send_packet(codecCtx, p);
        if (sp < 0 && sp != AVERROR_EAGAIN() && sp != AVERROR_EOF()) {
            throw new IOException("avcodec_send_packet failed: " + ffErr(sp));
        }
    }

    private long framePtsMs() {
        long pts = srcFrame.pts();
        if (pts == AV_NOPTS_VALUE) pts = srcFrame.best_effort_timestamp();
        if (pts == AV_NOPTS_VALUE) return 0;
        return Math.max(0, ((pts - startTicks) * 1000L * tbNum) / Math.max(1, tbDen));
    }

    private long halfFrameMs() {
        return fpsNum > 0 && fpsDen > 0 ? (500L * fpsDen) / fpsNum : 0;
    }

    private void scaleFrame() throws IOException {
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
    }

    // --- audio ---

    /**
     * Decode the first audio stream from here on, resampled to {@code rate} Hz
     * signed 16-bit little-endian with {@code channels} (1 or 2) interleaved.
     * Returns false when the file has no audio stream or it can't be decoded.
     * Calling it again with another format restarts the queue in that format.
     */
    public synchronized boolean enableAudio(int rate, int channels) throws IOException {
        if (audioStreamIndex < 0 || formatCtx == null) return false;
        if (rate < 8000 || rate > 48000 || channels < 1 || channels > 2) throw new IllegalArgumentException("audio format");
        if (audioEnabled && rate == outRate && channels == outChannels) return true;
        closeAudio();
        AVStream s = formatCtx.streams(audioStreamIndex);
        AVCodec codec = avcodec_find_decoder(s.codecpar().codec_id());
        if (codec == null) return false;
        audioCtx = avcodec_alloc_context3(codec);
        if (audioCtx == null || avcodec_parameters_to_context(audioCtx, s.codecpar()) < 0
                || avcodec_open2(audioCtx, codec, (PointerPointer<?>) null) < 0) {
            closeAudio();
            return false;
        }
        AVChannelLayout outLayout = new AVChannelLayout();
        av_channel_layout_default(outLayout, channels);
        swr = new SwrContext(null);
        int rc = swr_alloc_set_opts2(swr, outLayout, AV_SAMPLE_FMT_S16, rate,
                audioCtx.ch_layout(), audioCtx.sample_fmt(), audioCtx.sample_rate(), 0, null);
        av_channel_layout_uninit(outLayout);
        if (rc < 0 || swr.isNull() || swr_init(swr) < 0) {
            closeAudio();
            throw new IOException("swr setup failed: " + ffErr(rc));
        }
        audioFrame = av_frame_alloc();
        audioOutFrame = av_frame_alloc();
        AVRational tb = s.time_base();
        audioTbNum = tb.num();
        audioTbDen = tb.den();
        outRate = rate;
        outChannels = channels;
        audioEnabled = true;
        audioAligned = false;
        // Audio starts where the video is now: 0 at open, the seek target after a seek.
        if (dropBeforeMs >= 0) audioAlignMs = dropBeforeMs;
        return true;
    }

    /** Whether {@link #enableAudio} is on. */
    public synchronized boolean audioEnabled() { return audioEnabled; }

    /** Output audio bytes per second ({@code rate × channels × 2}), 0 without audio. */
    public synchronized int audioBytesPerSecond() { return audioEnabled ? outRate * outChannels * 2 : 0; }

    /**
     * Take up to {@code max} bytes (whole sample frames) of queued PCM into
     * {@code dst}, reading ahead in the file while the queue is shorter than
     * asked (video packets met on the way are held for {@link #next}).
     * Returns the bytes written: 0 at the end of the audio, or before
     * {@link #enableAudio}.
     */
    public synchronized int readAudio(byte[] dst, int max) throws IOException {
        if (!audioEnabled) return 0;
        int frame = outChannels * 2;
        int want = Math.min(max, dst.length) / frame * frame;
        while (audioQueued < want && !demuxEof && videoPackets.size() < MAX_VIDEO_PACKETS) {
            readPacket(false);
        }
        int n = (int) Math.min(want, audioQueued);
        int off = 0;
        while (off < n) {
            byte[] head = audioQueue.peekFirst();
            int take = Math.min(n - off, head.length - audioHeadOffset);
            System.arraycopy(head, audioHeadOffset, dst, off, take);
            off += take;
            audioHeadOffset += take;
            if (audioHeadOffset == head.length) {
                audioQueue.pollFirst();
                audioHeadOffset = 0;
            }
        }
        audioQueued -= n;
        return n;
    }

    /** Bytes of audio decoded and waiting. */
    public synchronized long audioQueuedBytes() { return audioQueued; }

    private void drainAudio() throws IOException {
        while (true) {
            int rc = avcodec_receive_frame(audioCtx, audioFrame);
            if (rc != 0) return; // EAGAIN, EOF or an error: nothing more now
            av_frame_unref(audioOutFrame);
            audioOutFrame.format(AV_SAMPLE_FMT_S16);
            audioOutFrame.sample_rate(outRate);
            av_channel_layout_default(audioOutFrame.ch_layout(), outChannels);
            if (swr_convert_frame(swr, audioOutFrame, audioFrame) < 0) continue;
            int got = audioOutFrame.nb_samples();
            if (got <= 0) continue;
            byte[] pcm = new byte[got * outChannels * 2];
            audioOutFrame.data(0).position(0).get(pcm);
            queueAudio(pcm, audioFramePtsMs());
        }
    }

    private long audioFramePtsMs() {
        long pts = audioFrame.pts();
        if (pts == AV_NOPTS_VALUE) pts = audioFrame.best_effort_timestamp();
        if (pts == AV_NOPTS_VALUE) return -1;
        long startMs = (startTicks * 1000L * tbNum) / Math.max(1, tbDen);
        return (pts * 1000L * audioTbNum) / Math.max(1, audioTbDen) - startMs;
    }

    /** Queue a converted chunk; the first after open/seek is padded or trimmed to start at {@link #audioAlignMs}. */
    private void queueAudio(byte[] pcm, long ptsMs) {
        int frameBytes = outChannels * 2;
        if (!audioAligned) {
            if (ptsMs < 0) ptsMs = audioAlignMs;
            long offsetFrames = (ptsMs - audioAlignMs) * outRate / 1000;
            if (offsetFrames < 0) {
                long trim = -offsetFrames * frameBytes;
                if (trim >= pcm.length) return; // all before the start
                byte[] rest = new byte[pcm.length - (int) trim];
                System.arraycopy(pcm, (int) trim, rest, 0, rest.length);
                pcm = rest;
            } else if (offsetFrames > 0) {
                long pad = Math.min(offsetFrames, (long) outRate * 5) * frameBytes; // at most 5 s of silence
                audioQueue.addLast(new byte[(int) pad]);
                audioQueued += pad;
            }
            audioAligned = true;
        }
        audioQueue.addLast(pcm);
        audioQueued += pcm.length;
        long max = (long) MAX_AUDIO_QUEUE_MS * outRate / 1000 * frameBytes;
        while (audioQueued > max && audioQueue.size() > 1) {
            byte[] old = audioQueue.pollFirst();
            audioQueued -= old.length - audioHeadOffset;
            audioHeadOffset = 0;
        }
    }

    private void clearAudioQueue() {
        audioQueue.clear();
        audioHeadOffset = 0;
        audioQueued = 0;
    }

    private void closeAudio() {
        audioEnabled = false;
        clearAudioQueue();
        if (swr != null) {
            swr_free(swr);
            swr = null;
        }
        if (audioFrame != null) {
            av_frame_free(audioFrame);
            audioFrame = null;
        }
        if (audioCtx != null) {
            avcodec_free_context(audioCtx);
            audioCtx = null;
        }
        if (audioOutFrame != null) {
            av_frame_free(audioOutFrame);
            audioOutFrame = null;
        }
    }

    // --- seeking and teardown ---

    /**
     * Seeks to the given position in milliseconds: the next {@link #next()}
     * returns the frame showing at that time, and audio restarts there.
     */
    public synchronized void seek(long ptsMs) throws IOException {
        if (formatCtx == null) throw new IOException("closed");
        if (ptsMs < 0) ptsMs = 0;
        long ticks = startTicks + (ptsMs * Math.max(1, tbDen)) / (1000L * Math.max(1, tbNum));
        int rc = av_seek_frame(formatCtx, videoStreamIndex, ticks, AVSEEK_FLAG_BACKWARD);
        if (rc < 0) throw new IOException("av_seek_frame failed: " + ffErr(rc));
        avcodec_flush_buffers(codecCtx);
        freeHeldPackets();
        eof = false;
        demuxEof = false;
        dropBeforeMs = ptsMs;
        if (audioEnabled) {
            avcodec_flush_buffers(audioCtx);
            clearAudioQueue();
            audioAlignMs = ptsMs;
            audioAligned = false;
        }
    }

    private void freeHeldPackets() {
        AVPacket p;
        while ((p = videoPackets.pollFirst()) != null) av_packet_free(p);
    }

    @Override
    public synchronized void close() {
        closeAudio();
        freeHeldPackets();
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
