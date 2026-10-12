package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.PhoneScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * ePhone hardware, namespace {@code ecm_phone} (docs/gui/EPHONE.md,
 * ROADMAP P5). Each test is the scenario {@code /ecm scenario spawn} builds
 * (PhoneScenarios), one per batch so they run one after another.
 */
@GameTestHolder(PhoneTests.NS)
@PrefixGameTestTemplate(false)
public final class PhoneTests {
    static final String NS = "ecm_phone";
    private static final String STRUCTURE = "gametest_phone";

    /** Held = awake, hotbar = asleep (frozen), out of the hotbar = off and back in the same state; hotkey slot 1 reaches the computer. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".basics")
    public static void phone_basics(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_basics"), true);
    }

    /** The battery drains while held, the dock charges it, an empty battery powers the phone off. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".battery")
    public static void phone_battery(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_battery"), true);
    }

    /** Server visibility GLOW sends another player no display packets; FULL does (control); the two toggles. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".visibility")
    public static void phone_visibility(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_visibility"), true);
    }

    /** Two players holding duplicates, a dimension change, dropping the phone. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".duplicates")
    public static void phone_duplicates(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_duplicates"), true);
    }

    /** Wi-Fi stays associated walking away from an Access Point until the link runs out (behind a stone wall). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_walk")
    public static void phone_wifi_walk(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_wifi_walk"), true);
    }
}
//?}
