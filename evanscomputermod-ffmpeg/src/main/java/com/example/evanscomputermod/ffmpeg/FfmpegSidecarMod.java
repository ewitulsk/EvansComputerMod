package com.example.evanscomputermod.ffmpeg;

import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Entry point of the FFmpeg sidecar: video support for EvansComputerMod
 * computers (the {@code video} command and the Video Player app). It only
 * needs to be on the server; the main mod finds {@link FfmpegVideoBackend}
 * through ServiceLoader. This is the only place the bundled FFmpeg native
 * libraries are loaded.
 */
@Mod("evanscomputermod_ffmpeg")
public class FfmpegSidecarMod {
    private static final Logger LOGGER = LoggerFactory.getLogger(FfmpegSidecarMod.class);
    /** The bundled bytedeco build (FFmpeg 8.0.1, javacpp 1.5.13); names the native cache folder. */
    static final String VERSION = "8.0.1-1.5.13";

    public FfmpegSidecarMod(IEventBus modEventBus) {
        LOGGER.info("FFmpeg video sidecar loaded: computers can play video files");
    }

    /**
     * Load the native libraries and the struct classes the decoder uses, so a
     * broken build fails here, loudly, instead of as a NoClassDefFoundError on
     * a program's thread. True when decoding works on this machine.
     */
    static boolean probe() {
        // javacpp unpacks the native libraries into ~/.javacpp/cache and re-unpacks a file whose
        // timestamp differs from the jar's. Another JVM on this machine with a different build
        // (another server, an older EvansComputerMod that bundled FFmpeg) keeps its copy loaded,
        // and Windows refuses the overwrite. Our own folder avoids that, unless the server set one.
        if (System.getProperty("org.bytedeco.javacpp.cachedir") == null) {
            System.setProperty("org.bytedeco.javacpp.cachedir", java.nio.file.Path.of(System.getProperty("user.home"),
                    ".javacpp", "cache", "evanscomputermod-ffmpeg-" + VERSION).toString());
        }
        try {
            int version = org.bytedeco.ffmpeg.global.avformat.avformat_version();
            LOGGER.info("FFmpeg libavformat loaded: version {}.{}.{}", version >>> 16, (version >>> 8) & 0xFF, version & 0xFF);
            Class.forName("org.bytedeco.ffmpeg.avformat.AVFormatContext");
            Class.forName("org.bytedeco.ffmpeg.avcodec.AVCodecContext");
            Class.forName("org.bytedeco.ffmpeg.avutil.AVFrame");
            Class.forName("org.bytedeco.ffmpeg.swscale.SwsContext");
            Class.forName("org.bytedeco.ffmpeg.swresample.SwrContext");
            return true;
        } catch (Throwable t) {
            LOGGER.error("FFmpeg native libraries can't be loaded on this machine: video playback is off", t);
            return false;
        }
    }
}
