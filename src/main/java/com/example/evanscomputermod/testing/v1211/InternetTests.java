package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RouterScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The Internet Gateway's link to the real network, namespace
 * {@code ecm_internet} (1.21.1): {@code router_internet}. Needs the
 * evanscomputermod-internet sidecar in the run (scripts/Test.ps1 -Internet);
 * without it the scenario stops at its first step and says so.
 */
@GameTestHolder(InternetTests.NS)
@PrefixGameTestTemplate(false)
public final class InternetTests {
    static final String NS = "ecm_internet";

    /** A client cabled to an Internet Gateway curls a web page on this machine; control: a LAN address is refused. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".router_internet")
    public static void router_internet(GameTestHelper h) {
        TestDriver.scenario(h, NS, RouterScenarios.ALL.get("router_internet"));
    }

    private InternetTests() {}
}
//?}
