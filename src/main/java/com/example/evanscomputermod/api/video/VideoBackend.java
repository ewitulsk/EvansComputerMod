package com.example.evanscomputermod.api.video;

import java.io.IOException;
import java.nio.file.Path;

/**
 * Decodes video files for computers (the {@code video_*} child imports, the
 * {@code video} command, the Video Player). The main mod has none: native
 * decoding lives in the optional, server-side {@code evanscomputermod-ffmpeg}
 * sidecar, found with {@link java.util.ServiceLoader} (see {@link VideoBackends}).
 * Without one, computers report video as not installed and the Video Player
 * app isn't offered.
 */
public interface VideoBackend {

    /** Bumped when this interface or {@link VideoSource} changes incompatibly. */
    int CURRENT_API_VERSION = 1;

    String name();

    /** Higher wins when several are installed. */
    int priority();

    int apiVersion();

    /** Whether it can decode on this machine (its native libraries load). */
    boolean isAvailable();

    /** Open {@code file} for frames scaled to {@code targetW x targetH} in {@code format} ({@code VideoSource.FORMAT_*}). */
    VideoSource open(Path file, int targetW, int targetH, int format) throws IOException;
}
