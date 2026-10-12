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
    private static final ModConfigSpec.IntValue SPEAKER_RANGE;
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
        b.pop();

        b.comment("Speaker block.").push("speaker");
        SPEAKER_RANGE = b
                .comment("How far a speaker can be heard, in blocks. Audio is only sent to players this close.")
                .defineInRange("range", 48, 8, 128);
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

    public static int uiNodesPerSurface() { return get(UI_NODES_PER_SURFACE, 8192); }

    public static int uiNodesPerApp() { return get(UI_NODES_PER_APP, 32768); }

    public static long uiResourceBytesPerApp() { return get(UI_RESOURCE_KB_PER_APP, 8192) * 1024L; }

    public static long uiTreeOpsBytesPerSecond() { return get(UI_TREE_OPS_KB_PER_SECOND, 256) * 1024L; }

    public static int uiSurfacesPerApp() { return get(UI_SURFACES_PER_APP, 64); }

    public static int uiConnections() { return get(UI_CONNECTIONS, 32); }

    public static int uiShellTimeoutMs() { return get(UI_SHELL_TIMEOUT_MS, 5000); }

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
