package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.SwitchScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The switching debug scenarios ({@code /ecm scenario spawn}) as 1.21.1
 * GameTests, namespace {@code ecm_switch}. One batch per scenario, so they
 * run one after another instead of competing for CPU.
 */
@GameTestHolder(SwitchTests.NS)
@PrefixGameTestTemplate(false)
public final class SwitchTests {
    static final String NS = "ecm_switch";

    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".basic")
    public static void switch_basic(GameTestHelper h) {
        TestDriver.scenario(h, NS, SwitchScenarios.ALL.get("switch_basic"));
    }

    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".vlans")
    public static void switch_vlans(GameTestHelper h) {
        TestDriver.scenario(h, NS, SwitchScenarios.ALL.get("switch_vlans"));
    }

    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".trunk")
    public static void switch_trunk(GameTestHelper h) {
        TestDriver.scenario(h, NS, SwitchScenarios.ALL.get("switch_trunk"));
    }

    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".stp")
    public static void switch_stp(GameTestHelper h) {
        TestDriver.scenario(h, NS, SwitchScenarios.ALL.get("switch_stp"));
    }

    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".lacp")
    public static void switch_lacp(GameTestHelper h) {
        TestDriver.scenario(h, NS, SwitchScenarios.ALL.get("switch_lacp"));
    }

    private SwitchTests() {}
}
//?}
