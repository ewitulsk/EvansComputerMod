package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.VoiceScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Microphones and voice over radio, namespace {@code ecm_voice}: the same
 * definitions {@code /ecm scenario spawn mic_record|mic_voice|radio_voice} builds.
 */
@GameTestHolder(VoiceTests.NS)
@PrefixGameTestTemplate(false)
public final class VoiceTests {
    static final String NS = "ecm_voice";
    private static final String STRUCTURE = "gametest_empty";

    /** A Microphone records a Speaker's beep (arecord, Python, Lua); quiet when nothing plays. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".record")
    public static void mic_record(GameTestHelper h) {
        TestDriver.scenario(h, NS, VoiceScenarios.ALL.get("mic_record"), true);
    }

    /** A Microphone records a player talking near it, and not from far away. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".voice")
    public static void mic_voice(GameTestHelper h) {
        TestDriver.scenario(h, NS, VoiceScenarios.ALL.get("mic_voice"), true);
    }

    /** Push-to-talk on a Handheld Radio heard on another; a computer talking through radio_station /dev/mic. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".radio")
    public static void radio_voice(GameTestHelper h) {
        TestDriver.scenario(h, NS, VoiceScenarios.ALL.get("radio_voice"), true);
    }
}
//?}
