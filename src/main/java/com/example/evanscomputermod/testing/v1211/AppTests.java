package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.AppScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * App platform (GUI roadmap P4), namespace {@code ecm_apps} (1.21.1): the
 * {@link AppScenarios}, player-built and typed, one batch each.
 */
@GameTestHolder(AppTests.NS)
@PrefixGameTestTemplate(false)
public final class AppTests {
    static final String NS = "ecm_apps";

    /** The Volume Mixer: two programs on one Speaker, one turned down, one muted; master mute silences both. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".volume_mixer")
    public static void volume_mixer(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.MixerScenarios.ALL.get("volume_mixer"), true);
    }

    /** A serves a repository; B installs the app over the cable with pkg; network denied until granted. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".app_repo")
    public static void app_repo(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("app_repo"), true);
    }

    /** A serves the sample repository; B installs Net Check with the store's Install button, opens it; the desktop's dialog: Deny, then Allow. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".app_store")
    public static void app_store(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("app_store"), true);
    }

    /** Double-clicking a .py in Files opens the Text Editor app, which can read only that file. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".files_open_with")
    public static void files_open_with(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("files_open_with"), true);
    }

    /** A server runs discordd and serves the Discord app; a cabled client installs it, adds the backend, talks in #general, gets a notification. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".discord_app")
    public static void discord_app(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("discord_app"), true);
    }

    /** The server's Discord backend and repository found with disco (none before disco serve); the store lists Discord unadded; Discord connects to what it found. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".service_discovery")
    public static void service_discovery(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("service_discovery"), true);
    }

    /** The video command plays a video on the Terminal with the Speaker; Files opens it in the Video Player on the desktop. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".video_desktop")
    public static void video_desktop(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.VideoScenarios.ALL.get("video_desktop"), true);
    }

    private AppTests() {}
}
//?}
