package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.TerminalScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The terminal and shell as a player uses them, namespace {@code ecm_terminal}:
 * the same definitions {@code /ecm scenario spawn <name>} builds
 * ({@code testing/scenario/TerminalScenarios}).
 */
@GameTestHolder(TerminalTests.NS)
@PrefixGameTestTemplate(false)
public final class TerminalTests {
    static final String NS = "ecm_terminal";
    private static final String STRUCTURE = "gametest_empty";

    /** Line editing at the prompt: moves, kill/yank, word jumps, Tab, history, Ctrl+R, !!; control: Ctrl+C abandons a line. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".terminal_line_editing")
    public static void terminal_line_editing(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("terminal_line_editing"), true);
    }

    /** Shift+PgUp/Up/Home/End scrollback; Ctrl+C stops a program; control: a full-screen program gets Ctrl+C itself. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".terminal_scrollback")
    public static void terminal_scrollback(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("terminal_scrollback"), true);
    }

    /** help with headings and help <topic>, man in the pager with a search, man -k, less; controls: unknown pages. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".terminal_manual")
    public static void terminal_manual(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("terminal_manual"), true);
    }

    /** Redirects and pipes (>, >>, |, <); control: the file isn't there before. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".terminal_pipes")
    public static void terminal_pipes(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("terminal_pipes"), true);
    }

    /** Python and Lua prompts: a typo fixed in the middle, Up, Ctrl+C abandons a line; control: the REPL stays. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repl_line_editing")
    public static void repl_line_editing(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("repl_line_editing"), true);
    }

    /** Ctrl+Z/jobs/bg/fg, $? and variables, --help, a pager at the end of a pipeline, the bell; control: no bell before. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".terminal_jobs_vars")
    public static void terminal_jobs_vars(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("terminal_jobs_vars"), true);
    }

    /** GUI copy goes to the player's clipboard, their clipboard pastes into the Editor, the launcher's Console item returns to the console. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".gui_console_clipboard")
    public static void gui_console_clipboard(GameTestHelper h) {
        TestDriver.scenario(h, NS, TerminalScenarios.ALL.get("gui_console_clipboard"), true);
    }
}
//?}
