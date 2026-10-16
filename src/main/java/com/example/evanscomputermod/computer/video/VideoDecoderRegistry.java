package com.example.evanscomputermod.computer.video;

import com.example.evanscomputermod.api.video.VideoBackend;
import com.example.evanscomputermod.api.video.VideoBackends;
import com.example.evanscomputermod.api.video.VideoSource;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Per-{@code ComputerInstance} table of open {@link VideoSource}s (opened by the installed {@link VideoBackend}). Handles
 * are small non-negative integers (starting at 1) so they can round-trip
 * through WASM i32 arguments cleanly.
 *
 * <p>Thread-safe: all mutators lock on this instance. Decoders themselves are
 * not thread-safe, but the registry ensures lifecycle operations (open,
 * close, closeAll) are serialized; per-decoder operations (next, seek) are
 * called through bridge methods on the owning ComputerInstance, which is
 * single-producer by convention.
 *
 * <p>Each decoder remembers the program that opened it ({@code ownerPid}, 0 =
 * not a program): {@link #get(int, int)} refuses another program's handle and
 * {@link #closeOwnedBy} frees what a program left open when it exits. At most
 * {@link #MAX_OPEN} decoders are open per computer.
 */
public final class VideoDecoderRegistry {

    private static final Logger LOG = LoggerFactory.getLogger(VideoDecoderRegistry.class);

    /** Most decoders open at once on one computer. */
    public static final int MAX_OPEN = 8;

    private final Map<Integer, VideoSource> decoders = new HashMap<>();
    private final Map<Integer, Integer> owners = new HashMap<>();
    private final AtomicInteger nextHandle = new AtomicInteger(1);

    /**
     * Opens a new decoder and returns its handle. Returns {@code -1} on
     * failure (any {@link IOException}). The exception is logged so bridge
     * callers can trace why an open failed even though the file was present
     * on disk — e.g. container format not recognized, no H.264 stream, etc.
     */
    public synchronized int open(Path mp4Path, int targetW, int targetH) {
        return open(mp4Path, targetW, targetH, VideoSource.FORMAT_INDEXED8);
    }

    /**
     * Opens a new decoder with an explicit output pixel format. Same
     * error handling as the indexed-default {@link #open(Path, int, int)}.
     */
    public synchronized int open(Path mp4Path, int targetW, int targetH, int outputFormat) {
        return open(mp4Path, targetW, targetH, outputFormat, 0);
    }

    /** Open a decoder for program {@code ownerPid}; -1 on failure or when {@link #MAX_OPEN} are open. */
    public synchronized int open(Path mp4Path, int targetW, int targetH, int outputFormat, int ownerPid) {
        if (decoders.size() >= MAX_OPEN) {
            LOG.info("VideoDecoder.open({}) refused: {} decoders already open", mp4Path, decoders.size());
            return -1;
        }
        VideoBackend backend = VideoBackends.get();
        if (backend == null) {
            LOG.info("VideoDecoder.open({}) refused: no video backend installed (evanscomputermod-ffmpeg)", mp4Path);
            return -1;
        }
        try {
            VideoSource d = backend.open(mp4Path, targetW, targetH, outputFormat);
            int h = nextHandle.getAndIncrement();
            decoders.put(h, d);
            owners.put(h, ownerPid);
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
    public synchronized VideoSource get(int handle) {
        return decoders.get(handle);
    }

    /** The decoder for {@code handle} if program {@code pid} opened it (pid 0 opens and uses any), else null. */
    public synchronized VideoSource get(int handle, int pid) {
        Integer owner = owners.get(handle);
        if (owner == null || (pid != 0 && owner != 0 && owner != pid)) return null;
        return decoders.get(handle);
    }

    /** Closes and removes the given decoder. No-op if not found. */
    public synchronized void close(int handle) {
        VideoSource d = decoders.remove(handle);
        owners.remove(handle);
        if (d != null) d.close();
    }

    /** Close every decoder program {@code pid} opened (it exited). Returns how many. */
    public synchronized int closeOwnedBy(int pid) {
        int n = 0;
        var it = owners.entrySet().iterator();
        while (it.hasNext()) {
            var e = it.next();
            if (e.getValue() != pid) continue;
            VideoSource d = decoders.remove(e.getKey());
            it.remove();
            if (d != null) {
                try { d.close(); } catch (Exception ignored) {}
                n++;
            }
        }
        return n;
    }

    /** Closes every decoder in the registry. Safe to call multiple times. */
    public synchronized void closeAll() {
        for (VideoSource d : decoders.values()) {
            try { d.close(); } catch (Exception ignored) {}
        }
        decoders.clear();
        owners.clear();
    }

    /** Number of currently-open decoders (debug). */
    public synchronized int size() {
        return decoders.size();
    }
}
