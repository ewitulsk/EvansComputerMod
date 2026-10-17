package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.LuaScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The Lua 5.2 interpreter, namespace {@code ecm_lua}: the same definition
 * {@code /ecm scenario spawn lua_basics} builds.
 */
@GameTestHolder(LuaTests.NS)
@PrefixGameTestTemplate(false)
public final class LuaTests {
    static final String NS = "ecm_lua";
    private static final String STRUCTURE = "gametest_empty";

    /** A Lua script (coroutines, pcall, io, peripherals, a redstone lamp), the REPL, and the error controls. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".basics")
    public static void lua_basics(GameTestHelper h) {
        TestDriver.scenario(h, NS, LuaScenarios.ALL.get("lua_basics"), true);
    }
}
//?}
