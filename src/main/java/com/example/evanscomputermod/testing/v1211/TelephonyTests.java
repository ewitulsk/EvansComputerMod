package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.scenario.CellularScenarios;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.example.evanscomputermod.testing.scenario.TelephonyScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Telephony (docs/TELEPHONY.md), namespace {@code ecm_telephony} (1.21.1): the
 * {@link TelephonyScenarios} on a player-built carrier (cored with a number block and IMS,
 * imsd, smscd, mmscd, mrfd; a cell site; ePhones), one batch each, each removed once it passes.
 * Structure {@code gametest_cellular} (48x12x36, this namespace's copy).
 *
 * <p>Each one builds the whole carrier first (computers boot, the core and its four IMS servers
 * start, the site registers, a SIM per person is written and fitted, the phones attach and
 * register): about 30-40 s of real time before the telephony checks start, so the scenarios'
 * time limits are above a minute. Sable must be loaded (as for {@code ecm_cellular}).
 */
@GameTestHolder(TelephonyTests.NS)
@PrefixGameTestTemplate(false)
public final class TelephonyTests {
    static final String NS = "ecm_telephony";
    static final String STRUCTURE = "gametest_cellular";

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".register")
    public static void tel_register(GameTestHelper h) {
        run(h, "tel_register");
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".call")
    public static void tel_call(GameTestHelper h) {
        run(h, "tel_call");
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".call_asleep")
    public static void tel_call_asleep(GameTestHelper h) {
        run(h, "tel_call_asleep");
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".sms")
    public static void tel_sms(GameTestHelper h) {
        run(h, "tel_sms");
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".mms_group")
    public static void tel_mms_group(GameTestHelper h) {
        run(h, "tel_mms_group");
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".scripts")
    public static void tel_scripts(GameTestHelper h) {
        run(h, "tel_scripts");
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ivr")
    public static void tel_ivr(GameTestHelper h) {
        run(h, "tel_ivr");
    }

    /** The scenario, real time; a failure report gets the last lines of every computer's screen. */
    private static void run(GameTestHelper h, String name) {
        Scenario s = TelephonyScenarios.ALL.get(name);
        ScenarioRun r = TestDriver.build(h, s, s.name);
        long[] start = {0}, ticks = {0};
        String[] why = {null};
        TestDriver.drive(h, NS, s.name, () -> {
            long now = System.nanoTime();
            if (start[0] == 0) start[0] = now;
            long due = start[0] + ++ticks[0] * 50_000_000L;
            if (due > now) java.util.concurrent.locks.LockSupport.parkNanos(due - now);
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

    private TelephonyTests() {}
}
//?}
