package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;


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

    /** {@link TestDriver#scenario}: the scenarios are {@code realTime}, so the server is paced to 50 ms per tick. */
    static void realTimeScenario(GameTestHelper h, Scenario s) {
        if (!s.realTime) throw new IllegalStateException(s.name + " should be a realTime scenario");
        TestDriver.scenario(h, RadioTests.NS, s);
    }

    /** {@code radio_station}: the station plays the playlist, a listener hears it on 11.6 MHz AM (handheld receiver), nothing at 11.67 MHz. */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".radio_station")
    public static void radio_station(GameTestHelper h) {
        // Removed once it passes: the station would otherwise keep transmitting on HF for the rest of the run.
        TestDriver.scenario(h, RadioTests.NS, RadioScenarios.ALL.get("radio_station"), true);
    }

    private RadioSdrProgramTests() {}
}
//?}
