package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.scenario.CellularScenarios;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Cellular (GUI roadmap P7), namespace {@code ecm_cellular} (1.21.1): the four
 * {@link CellularScenarios}, each one batch, each removed once it passes (towers
 * keep beaconing otherwise). Structure {@code gametest_cellular} (48x12x36);
 * {@code cell_lte_far} also builds a station 300 blocks east, kept loaded with
 * /forceload. Sable must be loaded (the handover cart is a Sable ship).
 */
@GameTestHolder(CellularTests.NS)
@PrefixGameTestTemplate(false)
public final class CellularTests {
    static final String NS = "ecm_cellular";
    static final String STRUCTURE = "gametest_cellular";

    /** LTE to a tower 300 blocks away: DHCP lease and pings from the server on its backhaul; no SIM = no carrier. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".lte_far")
    public static void lte_far_tower_dhcp_and_ping(GameTestHelper h) {
        run(h, "cell_lte_far");
    }

    /** mmWave attaches in line of sight and hears nothing behind an earth ridge; LTE gets through it. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".mmwave_hill")
    public static void mmwave_los_works_hill_blocks(GameTestHelper h) {
        run(h, "cell_mmwave_hill");
    }

    /** Same-carrier handover on a moving cart keeps an SSH (TCP) session. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".handover")
    public static void handover_keeps_tcp(GameTestHelper h) {
        run(h, "cell_handover");
    }

    /** A SIM of another carrier never attaches; the right SIM does. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wrong_carrier")
    public static void wrong_carrier_sim_does_not_attach(GameTestHelper h) {
        run(h, "cell_wrong_carrier");
    }

    /**
     * Cellular v2 phase 3, network side: a player builds a core (cored, a Card Reader/Writer, a
     * server on its LAN) and a cell site (celld-site, Advanced SDR, LTE panel on a mast); a wrong
     * site credential is refused, the right one registers over the sealed backhaul, a SIM is
     * written, a framed SDR hears the beacons, the Carrier Console shows the site.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cell_site_core")
    public static void cell_site_core(GameTestHelper h) {
        run(h, "cell_site_core");
    }

    /**
     * Cellular v2 phase 3, the whole network: cored (with its card writer) and celld-site on player-built computers; an
     * ePhone with an LTE Modem and the SIM cored wrote attaches through celld and pings a server on the core's LAN.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cell_phone_attach")
    public static void cell_phone_attach(GameTestHelper h) {
        run(h, com.example.evanscomputermod.testing.scenario.CellPhoneScenarios.ALL.get("cell_phone_attach"));
    }

    /** {@link TestDriver#scenario} with the towers' state added to a failure report. */
    private static void run(GameTestHelper h, String name) {
        run(h, RadioScenarios.ALL.get(name));
    }

    private static void run(GameTestHelper h, Scenario s) {
        ScenarioRun r = TestDriver.build(h, s, s.name);
        long[] start = {0}, ticks = {0};
        String[] why = {null};
        TestDriver.drive(h, NS, s.name, () -> {
            if (s.realTime) {
                long now = System.nanoTime();
                if (start[0] == 0) start[0] = now;
                long due = start[0] + ++ticks[0] * 50_000_000L;
                if (due > now) java.util.concurrent.locks.LockSupport.parkNanos(due - now);
            }
            if (r.tick() != ScenarioRun.State.PASSED) return false;
            r.clear();
            return true;
        }, () -> {
            if (r.state() != ScenarioRun.State.FAILED) return null;
            if (why[0] == null) {
                why[0] = r.failure() + "\n" + CellularScenarios.diagnoseRun(r) + "\n" + r.dump();
                EvansComputerMod.LOGGER.error("[{}] {}", s.name, why[0]);
                r.clear();
            }
            return why[0];
        });
    }

    private CellularTests() {}
}
//?}
