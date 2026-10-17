package com.example.evanscomputermod;

import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Mod settings ({@code config/evanscomputermod-common.toml}). Read through the
 * static getters, which fall back to the defaults before the file is loaded
 * (unit tests, early startup).
 */
public final class EcmConfig {

    public static final ModConfigSpec SPEC;

    private static final ModConfigSpec.IntValue DISPLAY_DEFAULT_HZ;
    private static final ModConfigSpec.IntValue DISPLAY_MAX_HZ;
    private static final ModConfigSpec.IntValue DISPLAY_PUMP_THREADS;
    private static final ModConfigSpec.IntValue DISPLAY_WINDOW_FRAMES;
    private static final ModConfigSpec.IntValue DISPLAY_WINDOW_KB;
    private static final ModConfigSpec.IntValue DISPLAY_QUEUE_FRAMES;
    private static final ModConfigSpec.IntValue DISPLAY_QUEUE_MS;
    private static final ModConfigSpec.BooleanValue DISPLAY_ALLOW_LOSSY;
    private static final ModConfigSpec.IntValue DISPLAY_MAX_STREAM_KBPS;
    private static final ModConfigSpec.IntValue DISPLAY_MAX_PLAYOUT_MS;
    private static final ModConfigSpec.IntValue SPEAKER_RANGE;
    private static final ModConfigSpec.IntValue MIC_RANGE;
    private static final ModConfigSpec.BooleanValue MIC_PLAYER_VOICES;
    /** Per panel tier (ordinal), in {@link TierCaps} component order. */
    private static final ModConfigSpec.IntValue[][] TIER_VALUES =
            new ModConfigSpec.IntValue[com.example.evanscomputermod.computer.display.PanelTier.values().length][];

    /** A tier's server caps (see {@link com.example.evanscomputermod.computer.display.PanelTier#modes()}). */
    public record TierCaps(int textCols, int textRows, int gfxWidth, int gfxHeight, int phoneWidth, int phoneHeight,
                           int screenTileWidth, int screenTileHeight, int screenMaxWidth, int screenMaxHeight,
                           int refreshHz) {}
    private static final ModConfigSpec.IntValue UI_NODES_PER_SURFACE;
    private static final ModConfigSpec.IntValue UI_NODES_PER_APP;
    private static final ModConfigSpec.IntValue UI_RESOURCE_KB_PER_APP;
    private static final ModConfigSpec.IntValue UI_TREE_OPS_KB_PER_SECOND;
    private static final ModConfigSpec.IntValue UI_SURFACES_PER_APP;
    private static final ModConfigSpec.IntValue UI_CONNECTIONS;
    private static final ModConfigSpec.IntValue UI_SHELL_TIMEOUT_MS;
    private static final ModConfigSpec.BooleanValue UI_CLIENT_RENDERING;
    private static final ModConfigSpec.BooleanValue APPS_ALLOW_WITHOUT_PROMPTER;
    private static final ModConfigSpec.IntValue APPS_PROMPT_TIMEOUT_S;
    private static final ModConfigSpec.IntValue APPS_SAVE_DEADLINE_MS;
    private static final ModConfigSpec.IntValue APPS_REFRESH_MINUTES;
    private static final ModConfigSpec.IntValue APPS_REFRESH_BUDGET_MS;
    private static final ModConfigSpec.IntValue APPS_BACKGROUND_CPU_PERCENT;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();

        b.comment("Graphics displays (the Terminal's screen and Screen clusters).").push("display");
        DISPLAY_DEFAULT_HZ = b
                .comment("Refresh rate a display starts at, in Hz. Programs can ask for up to display.maxRefreshHz.")
                .defineInRange("defaultRefreshHz", 30, 1, 60);
        DISPLAY_MAX_HZ = b
                .comment("Highest refresh rate a program may set, in Hz. Each refresh can send a frame to every viewer,",
                        "so higher rates cost server bandwidth.")
                .defineInRange("maxRefreshHz", 60, 1, 60);
        for (com.example.evanscomputermod.computer.display.PanelTier t
                : com.example.evanscomputermod.computer.display.PanelTier.values()) {
            TierCaps d = t.defaults();
            b.comment("Tier " + t.tier() + " (" + t.id() + ") display panels and Screens. Each value can only be lowered",
                    "from its default (docs/gui/DISPLAY_PIPELINE.md section 6).").push("tier" + t.tier());
            TIER_VALUES[t.ordinal()] = new ModConfigSpec.IntValue[]{
                    b.comment("Terminal text grid: columns.").defineInRange("textCols", d.textCols(), 10, d.textCols()),
                    b.comment("Terminal text grid: rows.").defineInRange("textRows", d.textRows(), 4, d.textRows()),
                    b.comment("Terminal graphics: largest width in pixels.").defineInRange("gfxWidth", d.gfxWidth(), 16, d.gfxWidth()),
                    b.comment("Terminal graphics: largest height in pixels.").defineInRange("gfxHeight", d.gfxHeight(), 16, d.gfxHeight()),
                    b.comment("ePhone (portrait): width in pixels.").defineInRange("phoneWidth", d.phoneWidth(), 16, d.phoneWidth()),
                    b.comment("ePhone (portrait): height in pixels.").defineInRange("phoneHeight", d.phoneHeight(), 16, d.phoneHeight()),
                    b.comment("Screen block: pixels per block, horizontally.").defineInRange("screenTileWidth", d.screenTileWidth(), 4, d.screenTileWidth()),
                    b.comment("Screen block: pixels per block, vertically.").defineInRange("screenTileHeight", d.screenTileHeight(), 4, d.screenTileHeight()),
                    b.comment("Screen wall: largest width in pixels.").defineInRange("screenMaxWidth", d.screenMaxWidth(), 16, d.screenMaxWidth()),
                    b.comment("Screen wall: largest height in pixels.").defineInRange("screenMaxHeight", d.screenMaxHeight(), 16, d.screenMaxHeight()),
                    b.comment("Highest refresh rate in Hz (display.maxRefreshHz still applies).").defineInRange("refreshHz", d.refreshHz(), 1, d.refreshHz()),
            };
            b.pop();
        }
        DISPLAY_PUMP_THREADS = b
                .comment("Threads that diff, encode and send display frames for every computer (off the server thread).",
                        "0 = min(4, cores / 2). docs/gui/DISPLAY_PIPELINE.md, \"Pipeline today\".")
                .defineInRange("pumpThreads", 0, 0, 32);
        DISPLAY_WINDOW_FRAMES = b
                .comment("Frames a viewer may have unacknowledged per display. More frames in flight means the frame rate",
                        "is limited by bandwidth instead of ping.")
                .defineInRange("windowFrames", 8, 1, 64);
        DISPLAY_WINDOW_KB = b
                .comment("Bytes (KiB) a viewer may have unacknowledged per display.")
                .defineInRange("windowKiB", 512, 16, 16384);
        DISPLAY_QUEUE_FRAMES = b
                .comment("Frames a program may queue ahead with gfx_present_at or CommitAt (timed presentation).")
                .defineInRange("queueFrames", 12, 1, 12);
        DISPLAY_QUEUE_MS = b
                .comment("How far ahead of now a queued frame may be, in ms.")
                .defineInRange("queueMs", 500, 50, 500);
        DISPLAY_ALLOW_LOSSY = b
                .comment("Send fast-changing pictures (video, games) as JPEG instead of exact tiles. Text and Tier 1",
                        "panels always stay exact. False turns lossy encoding off everywhere.")
                .define("allowLossy", true);
        DISPLAY_MAX_STREAM_KBPS = b
                .comment("Bandwidth budget per viewer for lossy pictures, in kbit/s, shared by everything one player watches.")
                .defineInRange("maxStreamKbps", 6000, 200, 6000);
        DISPLAY_MAX_PLAYOUT_MS = b
                .comment("Longest playout delay a client may use for timed frames, in ms (it adapts to jitter below this).")
                .defineInRange("maxPlayoutMs", 400, 60, 400);
        b.pop();

        b.comment("Speaker block.").push("speaker");
        SPEAKER_RANGE = b
                .comment("How far a speaker can be heard, in blocks. Audio is only sent to players this close.")
                .defineInRange("range", 48, 8, 128);
        b.pop();

        b.comment("Microphones (the Microphone block, the ePhone's microphone, handheld radios' push-to-talk).",
                "Whether a player's own microphone is used is each player's choice, in the client settings.").push("microphone");
        MIC_RANGE = b
                .comment("How far a microphone hears players and speakers, in blocks (sound falls off with distance).")
                .defineInRange("range", 24, 4, 128);
        MIC_PLAYER_VOICES = b
                .comment("Accept players' voices from their microphones. False: microphones only hear speakers.")
                .define("playerVoices", true);
        b.pop();

        b.comment("GUI compositor limits (docs/gui/UI_PROTOCOL.md section 6). An app that breaks one loses its UI",
                "connection (its windows close); the process keeps running.").push("ui");
        UI_NODES_PER_SURFACE = b.comment("Display-list nodes in one surface.")
                .defineInRange("nodesPerSurface", 8192, 64, 1 << 20);
        UI_NODES_PER_APP = b.comment("Display-list nodes in all of one app's surfaces.")
                .defineInRange("nodesPerApp", 32768, 64, 1 << 22);
        UI_RESOURCE_KB_PER_APP = b.comment("Images, canvases and pixel surfaces of one app, in KiB.")
                .defineInRange("resourceKiBPerApp", 8192, 256, 1 << 20);
        UI_TREE_OPS_KB_PER_SECOND = b.comment("Tree operations one app may send per second, in KiB. Above this,",
                        "commits are merged until the next refresh rather than rejected.")
                .defineInRange("treeOpsKiBPerSecond", 256, 8, 1 << 20);
        UI_SURFACES_PER_APP = b.comment("Surfaces one app may have.")
                .defineInRange("surfacesPerApp", 64, 1, 4096);
        UI_CONNECTIONS = b.comment("UI connections (apps plus the shell) per computer display.")
                .defineInRange("connections", 32, 2, 1024);
        UI_SHELL_TIMEOUT_MS = b.comment("A shell that leaves its messages unread this long is restarted (safe mode meanwhile).")
                .defineInRange("shellTimeoutMs", 5000, 500, 600000);
        UI_CLIENT_RENDERING = b.comment("Send GUI scenes (display-list patches) to players whose client can draw them,",
                        "instead of composited pixels (docs/gui/UI_PROTOCOL.md section 14, phase 2a). Off: every",
                        "player gets tile deltas, as before. Clients without the capability always get tiles.")
                .define("clientRendering", true);
        b.pop();

        b.comment("Installed apps (docs/gui/APPS.md): permissions and background rules.").push("apps");
        APPS_ALLOW_WITHOUT_PROMPTER = b
                .comment("When an app first uses a permission and nothing can ask the player (console mode, no GUI",
                        "running), allow it for that run instead of denying it. Denials are logged and can be",
                        "granted with `pkg perm grant <app> <permission>`.")
                .define("allowWithoutPrompter", false);
        APPS_PROMPT_TIMEOUT_S = b
                .comment("Seconds a permission prompt waits for an answer before the request is denied (not remembered).")
                .defineInRange("promptTimeoutSeconds", 60, 5, 600);
        APPS_SAVE_DEADLINE_MS = b
                .comment("How long apps get to save their state (WillSuspend -> SaveState) at shutdown or suspend, in ms.")
                .defineInRange("saveStateDeadlineMs", 500, 50, 5000);
        APPS_REFRESH_MINUTES = b
                .comment("ePhone: minutes between background refresh wakes of a suspended app.")
                .defineInRange("refreshIntervalMinutes", 5, 1, 120);
        APPS_REFRESH_BUDGET_MS = b
                .comment("ePhone: run time an app gets per refresh wake, in ms (it can end early with BackgroundDone).")
                .defineInRange("refreshBudgetMs", 2000, 100, 30000);
        APPS_BACKGROUND_CPU_PERCENT = b
                .comment("ePhone: CPU cap of an app in a background exception mode (audio, radio, transfer,",
                        "navigation), in percent of one core (duty-cycled).")
                .defineInRange("backgroundCpuPercent", 10, 1, 100);
        b.pop();

        SPEC = b.build();
    }

    private EcmConfig() {}

    private static int get(ModConfigSpec.IntValue v, int fallback) {
        try {
            return SPEC.isLoaded() ? v.get() : fallback;
        } catch (RuntimeException e) {
            return fallback;
        }
    }

    public static int displayDefaultRefreshHz() {
        return Math.min(get(DISPLAY_DEFAULT_HZ, 30), displayMaxRefreshHz());
    }

    public static int displayMaxRefreshHz() {
        return get(DISPLAY_MAX_HZ, 60);
    }

    /** Display pump threads (0 in the file = min(4, cores / 2)). */
    public static int displayPumpThreads() {
        int n = get(DISPLAY_PUMP_THREADS, 0);
        return n > 0 ? n : com.example.evanscomputermod.computer.pipeline.DisplayPump.defaultThreads();
    }

    /** Test override of the window and lossy settings ({@code ecm.display.*} system properties), else the config. */
    private static int prop(String name, int fallback) {
        String v = System.getProperty("ecm.display." + name);
        if (v == null) return fallback;
        try {
            return Integer.parseInt(v.trim());
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    public static int displayWindowFrames() {
        return prop("windowFrames", get(DISPLAY_WINDOW_FRAMES, 8));
    }

    public static long displayWindowBytes() {
        return prop("windowKiB", get(DISPLAY_WINDOW_KB, 512)) * 1024L;
    }

    public static int displayQueueFrames() {
        return get(DISPLAY_QUEUE_FRAMES, 12);
    }

    public static int displayQueueMs() {
        return get(DISPLAY_QUEUE_MS, 500);
    }

    private static volatile Boolean lossyOverride;

    public static boolean displayAllowLossy() {
        Boolean o = lossyOverride;
        if (o != null) return o;
        if ("false".equals(System.getProperty("ecm.display.allowLossy"))) return false;
        try {
            return !SPEC.isLoaded() || DISPLAY_ALLOW_LOSSY.get();
        } catch (RuntimeException e) {
            return true;
        }
    }

    /**
     * Set {@code display.allowLossy} in the server's config, as an operator
     * does by editing the config file (the value is saved to it). Used by the
     * {@code video_smooth_under_lag} scenario's control. Before the config is
     * loaded (unit tests) it is kept in memory only.
     */
    public static void setDisplayAllowLossy(boolean on) {
        try {
            if (SPEC.isLoaded()) {
                DISPLAY_ALLOW_LOSSY.set(on);
                lossyOverride = null;
                return;
            }
        } catch (RuntimeException ignored) {
            // fall through
        }
        lossyOverride = on;
    }

    public static int displayMaxStreamKbps() {
        return get(DISPLAY_MAX_STREAM_KBPS, 6000);
    }

    public static int displayMaxPlayoutMs() {
        return get(DISPLAY_MAX_PLAYOUT_MS, 400);
    }

    /** The server's caps for a panel tier (the defaults before the config is loaded). */
    public static TierCaps tierCaps(com.example.evanscomputermod.computer.display.PanelTier tier) {
        TierCaps d = tier.defaults();
        ModConfigSpec.IntValue[] v = TIER_VALUES[tier.ordinal()];
        if (v == null) return d;
        return new TierCaps(get(v[0], d.textCols()), get(v[1], d.textRows()), get(v[2], d.gfxWidth()),
                get(v[3], d.gfxHeight()), get(v[4], d.phoneWidth()), get(v[5], d.phoneHeight()),
                get(v[6], d.screenTileWidth()), get(v[7], d.screenTileHeight()), get(v[8], d.screenMaxWidth()),
                get(v[9], d.screenMaxHeight()), get(v[10], d.refreshHz()));
    }

    public static int speakerRange() {
        return get(SPEAKER_RANGE, 48);
    }

    public static int micRange() {
        return get(MIC_RANGE, 24);
    }

    public static boolean micPlayerVoices() {
        try {
            return !SPEC.isLoaded() || MIC_PLAYER_VOICES.get();
        } catch (RuntimeException e) {
            return true;
        }
    }

    public static int uiNodesPerSurface() { return get(UI_NODES_PER_SURFACE, 8192); }

    public static int uiNodesPerApp() { return get(UI_NODES_PER_APP, 32768); }

    public static long uiResourceBytesPerApp() { return get(UI_RESOURCE_KB_PER_APP, 8192) * 1024L; }

    public static long uiTreeOpsBytesPerSecond() { return get(UI_TREE_OPS_KB_PER_SECOND, 256) * 1024L; }

    public static int uiSurfacesPerApp() { return get(UI_SURFACES_PER_APP, 64); }

    public static int uiConnections() { return get(UI_CONNECTIONS, 32); }

    public static int uiShellTimeoutMs() { return get(UI_SHELL_TIMEOUT_MS, 5000); }

    /** Tests and the {@code ecm.ui.clientRendering} system property can force client rendering off. */
    public static boolean uiClientRendering() {
        if ("false".equals(System.getProperty("ecm.ui.clientRendering"))) return false;
        try {
            return SPEC.isLoaded() ? UI_CLIENT_RENDERING.get() : true;
        } catch (RuntimeException e) {
            return true;
        }
    }

    /**
     * Set {@code ui.clientRendering} in the server's config, as an operator
     * does by editing {@code serverconfig/evanscomputermod-server.toml} (the
     * value is saved to that file). Used by the {@code gui_client_render}
     * scenario's control.
     */
    public static void setUiClientRendering(boolean on) {
        UI_CLIENT_RENDERING.set(on);
    }

    public static boolean appsAllowWithoutPrompter() {
        try {
            return SPEC.isLoaded() ? APPS_ALLOW_WITHOUT_PROMPTER.get() : false;
        } catch (RuntimeException e) {
            return false;
        }
    }

    public static int appsPromptTimeoutSeconds() {
        return get(APPS_PROMPT_TIMEOUT_S, 60);
    }

    public static int appsSaveStateDeadlineMs() {
        return get(APPS_SAVE_DEADLINE_MS, 500);
    }

    public static int appsRefreshIntervalMinutes() {
        return get(APPS_REFRESH_MINUTES, 5);
    }

    public static int appsRefreshBudgetMs() {
        return get(APPS_REFRESH_BUDGET_MS, 2000);
    }

    public static int appsBackgroundCpuPercent() {
        return get(APPS_BACKGROUND_CPU_PERCENT, 10);
    }
}
