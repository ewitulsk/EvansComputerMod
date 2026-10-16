package com.example.evanscomputermod.api.video;

import com.example.evanscomputermod.EvansComputerMod;

import java.util.ServiceLoader;

/**
 * The installed {@link VideoBackend}, if any: discovered once with
 * {@link ServiceLoader} (the {@code evanscomputermod-ffmpeg} sidecar registers
 * one), the highest-priority available one with the current API version wins.
 */
public final class VideoBackends {
    private static volatile boolean looked;
    private static volatile VideoBackend selected;

    private VideoBackends() {}

    /** The backend, or null when no video support is installed. */
    public static VideoBackend get() {
        if (!looked) select();
        return selected;
    }

    /** Whether computers can play video on this server. */
    public static boolean available() {
        return get() != null;
    }

    private static synchronized void select() {
        if (looked) return;
        VideoBackend best = null;
        try {
            for (VideoBackend b : ServiceLoader.load(VideoBackend.class, VideoBackends.class.getClassLoader())) {
                if (b.apiVersion() != VideoBackend.CURRENT_API_VERSION) {
                    EvansComputerMod.LOGGER.warn("Video backend '{}' rejected: apiVersion {} != {}", b.name(), b.apiVersion(),
                            VideoBackend.CURRENT_API_VERSION);
                    continue;
                }
                boolean ok;
                try {
                    ok = b.isAvailable();
                } catch (Throwable t) {
                    ok = false;
                }
                if (!ok) {
                    EvansComputerMod.LOGGER.warn("Video backend '{}' is installed but can't decode on this machine", b.name());
                    continue;
                }
                if (best == null || b.priority() > best.priority()) best = b;
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.warn("Looking for a video backend failed: {}", t.toString());
        }
        selected = best;
        looked = true;
        if (best == null) {
            EvansComputerMod.LOGGER.info("No video backend installed (evanscomputermod-ffmpeg): video files can't be played");
        } else {
            EvansComputerMod.LOGGER.info("Video backend: {}", best.name());
        }
    }

    /** Tests: use this backend (null = none), or look again with {@link #reset()}. */
    public static synchronized void setForTesting(VideoBackend backend) {
        selected = backend;
        looked = true;
    }

    public static synchronized void reset() {
        selected = null;
        looked = false;
    }
}
