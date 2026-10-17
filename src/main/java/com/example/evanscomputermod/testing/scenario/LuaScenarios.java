package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.RedstoneLampBlock;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The Lua 5.2 interpreter ({@code rust/wasm-programs/lua}), spawnable with
 * {@code /ecm scenario spawn lua_basics}; also the {@code ecm_lua} GameTest.
 *
 * <p>{@code lua_basics}: a Terminal with a Redstone Lamp the player puts on
 * top of it. The player types {@code lua_demo.lua} in (from Python) and runs
 * it: coroutines, an error caught by {@code pcall} (Lua's C error handling,
 * setjmp/longjmp over WASM exception handling), a file written with Lua's
 * {@code io} and read back through {@code shell}, the peripheral list, and
 * every redstone side powered, which lights the lamp. Then the REPL: an
 * expression, a multi-line loop, and the lamp switched off from the
 * {@code shell} module. Controls: the script without its argument stops
 * with its own error, and a missing file is reported.
 * <pre>
 *   Terminal at (0,1,0) facing north, Redstone Lamp at (0,2,0) on top of it.
 * </pre>
 */
public final class LuaScenarios {
    public static final String PROGRAM = "lua_demo.lua";
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos LAMP = new BlockPos(0, 2, 0);
    private static final String REPL_PROMPT = "^>$";

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        Scenario s = basics();
        ALL.put(s.name, s);
    }

    private LuaScenarios() {
    }

    private static Scenario basics() {
        var b = Scenario.builder("lua_basics",
                        "Lua 5.2: run a script (coroutines, pcall, io, peripherals, redstone lamp), then the REPL")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(LAMP), null, LuaScenarios::placeLamp, null))
                .note("Setup (done for you): a Redstone Lamp placed on top of the computer")
                .note("Type the Lua demo in (" + PROGRAM + ", from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/" + PROGRAM, "/evanscomputermod/scenarios/" + PROGRAM);
        return b.note("Control: without its count the script stops with its own error")
                .send("pc", "lua " + PROGRAM)
                .expect("pc", "^lua: give a count", "the script's error is reported", "^(LUA|SQUARES) ")
                .expect("pc", PlayerKit.PROMPT, "and the shell prompt is back")
                .note("Control: a file that isn't there")
                .send("pc", "lua nothere.lua")
                .expect("pc", "^lua: cannot open nothere\\.lua", "lua says it can't open it")
                .expect("pc", PlayerKit.PROMPT, "and the shell prompt is back")
                .await(r -> lit(r) ? "the lamp is lit" : null, "control: the lamp starts off", 2_000)
                .note("Run the demo with a count of 5")
                .send("pc", "lua " + PROGRAM + " 5")
                .expect("pc", "^LUA Lua 5\\.2$", "it runs on Lua 5.2", "^lua: ")
                .expect("pc", "^SQUARES 1 4 9 16 25$", "a coroutine yields the squares", "^lua: ")
                .expect("pc", "^CAUGHT false 42$", "pcall catches an error thrown as a table", "^lua: ")
                .expect("pc", "^FILE count=5$", "the file Lua's io wrote reads back", "^lua: ")
                .expect("pc", "^PERIPHERALS \\d+", "the peripheral list", "^lua: ")
                .expect("pc", "^REDSTONE on$", "every side powered", "^lua: ")
                .expect("pc", PlayerKit.PROMPT, "the script ends")
                .await(r -> lit(r) ? null : "the lamp is off", "the lamp on top lights up", 5_000)
                .note("The REPL: an expression, a multi-line loop, the shell module")
                .send("pc", "lua")
                .expect("pc", "^Lua 5\\.2 ", "the REPL's banner")
                .expect("pc", REPL_PROMPT, "the REPL prompt")
                .send("pc", "6 * 7")
                .expect("pc", "^42$", "an expression prints its value")
                .send("pc", "for i = 1, 3 do")
                .expect("pc", "^>>$", "an unfinished statement asks for more")
                .send("pc", "print(i * 10)")
                .send("pc", "end")
                .expect("pc", "^30$", "the loop runs once it is closed")
                .send("pc", "for side = shell.DOWN, shell.RIGHT do shell.set_redstone(side, 0) end")
                .expect("pc", REPL_PROMPT, "redstone off")
                .await(r -> lit(r) ? "the lamp is still lit" : null, "the lamp goes out", 5_000)
                .send("pc", "exit")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    /** The player puts a Redstone Lamp on top of the computer. */
    private static void placeLamp(ScenarioRun run) {
        run.player().placeAgainst(Blocks.REDSTONE_LAMP, run.abs(LAMP), Direction.SOUTH, Direction.DOWN,
                s -> s.is(Blocks.REDSTONE_LAMP));
    }

    private static boolean lit(ScenarioRun run) {
        var st = run.level().getBlockState(run.abs(LAMP));
        return st.is(Blocks.REDSTONE_LAMP) && st.getValue(RedstoneLampBlock.LIT);
    }
}
//?}
