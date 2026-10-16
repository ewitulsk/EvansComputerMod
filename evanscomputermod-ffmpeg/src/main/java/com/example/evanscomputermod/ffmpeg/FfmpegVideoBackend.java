package com.example.evanscomputermod.ffmpeg;

import com.example.evanscomputermod.api.video.VideoBackend;
import com.example.evanscomputermod.api.video.VideoSource;

import java.io.IOException;
import java.nio.file.Path;

/**
 * The FFmpeg video backend (bytedeco's FFmpeg build: libavformat, libavcodec,
 * libswscale, libswresample), found by the main mod with ServiceLoader when
 * this sidecar is in the server's mods folder.
 */
public final class FfmpegVideoBackend implements VideoBackend {
    private static volatile Boolean available;

    @Override public String name() { return "ffmpeg"; }
    @Override public int priority() { return 100; }
    @Override public int apiVersion() { return VideoBackend.CURRENT_API_VERSION; }

    @Override
    public boolean isAvailable() {
        Boolean a = available;
        if (a == null) available = a = FfmpegSidecarMod.probe();
        return a;
    }

    @Override
    public VideoSource open(Path file, int targetW, int targetH, int format) throws IOException {
        return VideoDecoder.open(file, targetW, targetH, format);
    }
}
