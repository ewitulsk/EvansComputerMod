package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.ConsoleScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Console programs (crates/ecm-console), namespace {@code ecm_console}: the
 * same definitions {@code /ecm scenario spawn cc_lua} / {@code console_python} build.
 */
@GameTestHolder(ConsoleTests.NS)
@PrefixGameTestTemplate(false)
public final class ConsoleTests {
    static final String NS = "ecm_console";
    private static final String STRUCTURE = "gametest_empty";

    /** A ComputerCraft program in Lua: colours, clocks against the world, one event queue, read(), keys. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cc_lua")
    public static void cc_lua(GameTestHelper h) {
        TestDriver.scenario(h, NS, ConsoleScenarios.ALL.get("cc_lua"), true);
    }

    /** The same console modules from Python: term, computer, events, keys. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".python")
    public static void console_python(GameTestHelper h) {
        TestDriver.scenario(h, NS, ConsoleScenarios.ALL.get("console_python"), true);
    }

    /** Phase 2: a CC program with fs, textutils, a lever (redstone event), parallel blinking a lamp. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cc_world")
    public static void cc_world(GameTestHelper h) {
        TestDriver.scenario(h, NS, ConsoleScenarios.ALL.get("cc_world"), true);
    }

    /** Phase 2 from Python: fs, textutils, the lever and the lamp. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".python_world")
    public static void console_world(GameTestHelper h) {
        TestDriver.scenario(h, NS, ConsoleScenarios.ALL.get("console_world"), true);
    }
}
//?}
