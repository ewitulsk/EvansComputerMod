package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.VisualScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The Visual Programmer app, namespace {@code ecm_visual} (docs/gui/ROADMAP.md
 * P6): the same definition {@code /ecm scenario spawn visual_console} builds.
 * {@code visual_app} needs the GUI (the P2 compositor) and joins this
 * namespace once that is merged.
 */
@GameTestHolder(VisualTests.NS)
@PrefixGameTestTemplate(false)
public final class VisualTests {
    static final String NS = "ecm_visual";
    private static final String STRUCTURE = "gametest_empty";

    /** Old .vpl samples generate the old Python and run as a python process; Package writes app.toml. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".console")
    public static void visual_console(GameTestHelper h) {
        TestDriver.scenario(h, NS, VisualScenarios.ALL.get("visual_console"), true);
    }
}
//?}
