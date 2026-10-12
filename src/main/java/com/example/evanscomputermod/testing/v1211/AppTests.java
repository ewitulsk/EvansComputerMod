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

    /** A serves the sample repository; B installs Net Check from the App Store's Install button and opens it (window). */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".app_store")
    public static void app_store(GameTestHelper h) {
        TestDriver.scenario(h, NS, AppScenarios.ALL.get("app_store"), true);
    }

    private AppTests() {}
}
//?}
