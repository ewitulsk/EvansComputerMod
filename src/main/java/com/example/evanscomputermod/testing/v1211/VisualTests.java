package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.VisualScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The Visual Programmer app, namespace {@code ecm_visual} (docs/gui/ROADMAP.md
 * P6): the same definitions {@code /ecm scenario spawn visual_console} and
 * {@code visual_app} build.
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

    /** In the GUI: the editor opens the counter program, and the packaged counter app gets a window. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".gui")
    public static void visual_app(GameTestHelper h) {
        TestDriver.scenario(h, NS, VisualScenarios.ALL.get("visual_app"), true);
    }

    /** The palette follows what is attached: a Create Basin adds the tank blocks; the saved program runs against it. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".live_catalog")
    public static void visual_live_catalog(GameTestHelper h) {
        TestDriver.scenario(h, NS, VisualScenarios.ALL.get("visual_live_catalog"), true);
    }
}
//?}
