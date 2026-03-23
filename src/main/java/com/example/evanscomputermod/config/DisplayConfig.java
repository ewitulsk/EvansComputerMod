package com.example.evanscomputermod.config;

import net.neoforged.neoforge.common.ModConfigSpec;
import org.apache.commons.lang3.tuple.Pair;

/**
 * Configuration for the display/framebuffer system.
 * Server config controls limits enforced on the server.
 * Common config controls client-side rendering settings.
 */
public class DisplayConfig {

    // ==================== Server Config ====================

    public static class Server {
        // Display size limits
        public final ModConfigSpec.IntValue maxDisplayWidth;
        public final ModConfigSpec.IntValue maxDisplayHeight;
        public final ModConfigSpec.IntValue pixelsPerBlock;
        public final ModConfigSpec.IntValue maxTotalPixels;

        // Network / Performance
        public final ModConfigSpec.IntValue tileSize;
        public final ModConfigSpec.IntValue maxFlushRateTicks;
        public final ModConfigSpec.IntValue maxDirtyTilesPerPacket;
        public final ModConfigSpec.IntValue maxBytesPerTickPerPlayer;
        public final ModConfigSpec.BooleanValue fullPacketCompression;

        // Server resource limits
        public final ModConfigSpec.IntValue maxDisplaysPerPlayer;
        public final ModConfigSpec.IntValue maxDisplaysTotal;
        public final ModConfigSpec.IntValue maxFramebufferMemoryMB;

        // WASM / Framebuffer API limits
        public final ModConfigSpec.IntValue maxWriteRegionSize;
        public final ModConfigSpec.BooleanValue allowFbBlitText;

        Server(ModConfigSpec.Builder builder) {
            builder.comment("Display Block Configuration")
                    .push("display_size");

            maxDisplayWidth = builder
                    .comment("Maximum display width in blocks")
                    .defineInRange("maxDisplayWidth", 8, 1, 32);

            maxDisplayHeight = builder
                    .comment("Maximum display height in blocks")
                    .defineInRange("maxDisplayHeight", 8, 1, 32);

            pixelsPerBlock = builder
                    .comment("Pixel resolution per block (both axes). A 3x2 display at 128 = 384x256")
                    .defineInRange("pixelsPerBlock", 128, 32, 512);

            maxTotalPixels = builder
                    .comment("Hard cap on total pixels per display. Prevents excessive memory allocation")
                    .defineInRange("maxTotalPixels", 1048576, 65536, 4194304);

            builder.pop();
            builder.comment("Network and Performance Settings")
                    .push("network");

            tileSize = builder
                    .comment("Dirty-tile size in pixels. Must be power of 2. Smaller = finer granularity but more overhead")
                    .defineInRange("tileSize", 16, 8, 64);

            maxFlushRateTicks = builder
                    .comment("Minimum ticks between framebuffer flushes (2 = 10 FPS, 1 = 20 FPS)")
                    .defineInRange("maxFlushRateTicks", 2, 1, 20);

            maxDirtyTilesPerPacket = builder
                    .comment("Maximum tiles in a single update packet. Excess tiles queue to next tick")
                    .defineInRange("maxDirtyTilesPerPacket", 256, 16, 1024);

            maxBytesPerTickPerPlayer = builder
                    .comment("Bandwidth cap: max framebuffer bytes sent to one player per tick (bytes)")
                    .defineInRange("maxBytesPerTickPerPlayer", 131072, 32768, 1048576);

            fullPacketCompression = builder
                    .comment("Whether to deflate-compress full framebuffer packets")
                    .define("fullPacketCompression", true);

            builder.pop();
            builder.comment("Server Resource Limits")
                    .push("resources");

            maxDisplaysPerPlayer = builder
                    .comment("Maximum active displays one player can own")
                    .defineInRange("maxDisplaysPerPlayer", 16, 1, 128);

            maxDisplaysTotal = builder
                    .comment("Maximum active displays server-wide")
                    .defineInRange("maxDisplaysTotal", 128, 1, 1024);

            maxFramebufferMemoryMB = builder
                    .comment("Total RAM budget for all framebuffers combined (MB)")
                    .defineInRange("maxFramebufferMemoryMB", 256, 32, 2048);

            builder.pop();
            builder.comment("WASM / Framebuffer API Limits")
                    .push("wasm_api");

            maxWriteRegionSize = builder
                    .comment("Max pixels per single fb_write_region call")
                    .defineInRange("maxWriteRegionSize", 65536, 4096, 1048576);

            allowFbBlitText = builder
                    .comment("Whether fb_blit_text is enabled")
                    .define("allowFbBlitText", true);

            builder.pop();
        }
    }

    // ==================== Common Config ====================

    public static class Common {
        public final ModConfigSpec.IntValue displayRenderDistance;
        public final ModConfigSpec.IntValue maxClientTextures;

        Common(ModConfigSpec.Builder builder) {
            builder.comment("Client-Side Display Settings")
                    .push("client");

            displayRenderDistance = builder
                    .comment("Max block distance at which displays render their content. Beyond this, a static placeholder is shown")
                    .defineInRange("displayRenderDistance", 64, 16, 256);

            maxClientTextures = builder
                    .comment("Max simultaneous DynamicTextures on the client. Displays beyond this show placeholder")
                    .defineInRange("maxClientTextures", 32, 4, 128);

            builder.pop();
        }
    }

    // ==================== Spec Instances ====================

    public static final ModConfigSpec SERVER_SPEC;
    public static final Server SERVER;

    public static final ModConfigSpec COMMON_SPEC;
    public static final Common COMMON;

    static {
        Pair<Server, ModConfigSpec> serverPair = new ModConfigSpec.Builder().configure(Server::new);
        SERVER = serverPair.getLeft();
        SERVER_SPEC = serverPair.getRight();

        Pair<Common, ModConfigSpec> commonPair = new ModConfigSpec.Builder().configure(Common::new);
        COMMON = commonPair.getLeft();
        COMMON_SPEC = commonPair.getRight();
    }
}
