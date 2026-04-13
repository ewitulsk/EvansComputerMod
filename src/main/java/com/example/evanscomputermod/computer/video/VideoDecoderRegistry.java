package com.example.evanscomputermod.computer.video;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Per-{@code ComputerInstance} table of open {@link VideoDecoder}s. Handles
 * are small non-negative integers (starting at 1) so they can round-trip
 * through WASM i32 arguments cleanly.
 *
 * <p>Thread-safe: all mutators lock on this instance. Decoders themselves are
 * not thread-safe, but the registry ensures lifecycle operations (open,
 * close, closeAll) are serialized; per-decoder operations (next, seek) are
 * called through bridge methods on the owning ComputerInstance, which is
 * single-producer by convention.
 */
public final class VideoDecoderRegistry {

    private static final Logger LOG = LoggerFactory.getLogger(VideoDecoderRegistry.class);

    private final Map<Integer, VideoDecoder> decoders = new HashMap<>();
    private final AtomicInteger nextHandle = new AtomicInteger(1);

    /**
     * Opens a new decoder and returns its handle. Returns {@code -1} on
     * failure (any {@link IOException}). The exception is logged so bridge
     * callers can trace why an open failed even though the file was present
     * on disk — e.g. container format not recognized, no H.264 stream, etc.
     */
    public synchronized int open(Path mp4Path, int targetW, int targetH) {
        return open(mp4Path, targetW, targetH, VideoDecoder.FORMAT_INDEXED8);
    }

    /**
     * Opens a new decoder with an explicit output pixel format. Same
     * error handling as the indexed-default {@link #open(Path, int, int)}.
     */
    public synchronized int open(Path mp4Path, int targetW, int targetH, int outputFormat) {
        try {
            VideoDecoder d = VideoDecoder.open(mp4Path, targetW, targetH, outputFormat);
            int h = nextHandle.getAndIncrement();
            decoders.put(h, d);
            return h;
        } catch (IOException e) {
            // Unwrap the cause chain so the log line has the actual FFmpeg /
            // JNI error rather than the wrapper message.
            StringBuilder chain = new StringBuilder(e.getMessage());
            Throwable c = e.getCause();
            while (c != null) {
                chain.append(" <- ")
                     .append(c.getClass().getSimpleName())
                     .append(": ")
                     .append(c.getMessage());
                c = c.getCause();
            }
            LOG.info("VideoDecoder.open({}) failed: {}", mp4Path, chain);
            // Also dump the full stack at DEBUG so advanced users can drill in
            // without flipping log config for everyone else.
            LOG.debug("VideoDecoder.open stack trace", e);
            return -1;
        }
    }

    /** Returns the decoder for the given handle, or {@code null}. */
    public synchronized VideoDecoder get(int handle) {
        return decoders.get(handle);
    }

    /** Closes and removes the given decoder. No-op if not found. */
    public synchronized void close(int handle) {
        VideoDecoder d = decoders.remove(handle);
        if (d != null) d.close();
    }

    /** Closes every decoder in the registry. Safe to call multiple times. */
    public synchronized void closeAll() {
        for (VideoDecoder d : decoders.values()) {
            try { d.close(); } catch (Exception ignored) {}
        }
        decoders.clear();
    }

    /** Number of currently-open decoders (debug). */
    public synchronized int size() {
        return decoders.size();
    }
}
