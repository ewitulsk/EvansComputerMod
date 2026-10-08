package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.concurrent.locks.LockSupport;

/**
 * The SDR program scenarios ({@code sdr_lab}, {@code radio0_lab}) as
 * GameTests, namespace {@code ecm_radio}.
 *
 * <p>The SDR's sample clock is the world's game time. The GameTest server
 * ticks unthrottled (many times real time), which would run the radio clock
 * far ahead of the computers' real-time programs, so these tests hold the
 * server to 20 ticks per second while they run, as a normal server does.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioSdrProgramTests {

    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".sdr_lab")
    public static void sdr_lab(GameTestHelper h) {
        realTimeScenario(h, RadioScenarios.ALL.get("sdr_lab"));
    }

    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".radio0_lab")
    public static void radio0_lab(GameTestHelper h) {
        realTimeScenario(h, RadioScenarios.ALL.get("radio0_lab"));
    }

    /** {@link TestDriver#scenario}, with the server paced to 50 ms per tick. */
    static void realTimeScenario(GameTestHelper h, Scenario s) {
        ScenarioRun run = TestDriver.build(h, s, s.name);
        long[] start = {0};
        long[] ticks = {0};
        TestDriver.drive(h, RadioTests.NS, s.name, () -> {
            long now = System.nanoTime();
            if (start[0] == 0) start[0] = now;
            long due = start[0] + ++ticks[0] * 50_000_000L;
            if (due > now) LockSupport.parkNanos(due - now);
            return run.tick() == ScenarioRun.State.PASSED;
        }, () -> run.state() == ScenarioRun.State.FAILED ? run.failure() + "\n" + run.dump() : null);
    }

    private RadioSdrProgramTests() {}
}
//?}
