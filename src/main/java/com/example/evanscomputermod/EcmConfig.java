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
        b.pop();

        b.comment("Speaker block.").push("speaker");
        SPEAKER_RANGE = b
                .comment("How far a speaker can be heard, in blocks. Audio is only sent to players this close.")
                .defineInRange("range", 48, 8, 128);
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

    public static int speakerRange() {
        return get(SPEAKER_RANGE, 48);
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
