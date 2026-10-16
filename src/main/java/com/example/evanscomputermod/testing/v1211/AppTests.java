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

    /** A server packages Chat and serves it with chatd; a cabled client (DHCP lease) installs it from the App Store, chats, gets a notification. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".chat_app")
    public static void chat_app(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("chat_app"), true);
    }

    /** The server's chat and repository found with disco (none before disco serve); the store lists its Chat unadded; Chat connects to what it found. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".service_discovery")
    public static void service_discovery(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("service_discovery"), true);
    }

    private AppTests() {}
}
//?}
