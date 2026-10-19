package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.VimScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Vim at a Terminal as a player uses it, namespace {@code ecm_vim}: the same
 * definitions {@code /ecm scenario spawn <name>} builds
 * ({@code testing/scenario/VimScenarios}).
 */
@GameTestHolder(VimTests.NS)
@PrefixGameTestTemplate(false)
public final class VimTests {
    static final String NS = "ecm_vim";
    private static final String STRUCTURE = "gametest_empty";

    /** Modes, motions, operators with counts, text objects, registers, macros, ., undo/redo, :s, search, marks, Visual; controls: Ctrl+C, E37, no file before :w. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".vim_editing")
    public static void vim_editing(GameTestHelper h) {
        TestDriver.scenario(h, NS, VimScenarios.ALL.get("vim_editing"), true);
    }

    /** Buffers, :split, Ctrl+W, :vsplit, tab pages, :only; control: the new file holds what was typed. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".vim_windows")
    public static void vim_windows(GameTestHelper h) {
        TestDriver.scenario(h, NS, VimScenarios.ALL.get("vim_windows"), true);
    }

    /** ~/.vimrc, vi, filetype and syntax highlighting, :help, :version; controls: :! and Ctrl+Z say they aren't available. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".vim_runtime")
    public static void vim_runtime(GameTestHelper h) {
        TestDriver.scenario(h, NS, VimScenarios.ALL.get("vim_runtime"), true);
    }

    /** A panel swap (160x50 to 80x25) resizes Vim, which redraws; control: the old size before. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".vim_resize")
    public static void vim_resize(GameTestHelper h) {
        TestDriver.scenario(h, NS, VimScenarios.ALL.get("vim_resize"), true);
    }
}
//?}
