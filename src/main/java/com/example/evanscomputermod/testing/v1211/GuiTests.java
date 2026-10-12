package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.GuiScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The GUI in a world, namespace {@code ecm_gui}: the {@code gui_basics}
 * scenario as spawned by {@code /ecm scenario spawn gui_basics} (a Terminal
 * boots, {@code gui} starts the compositor and the test shell, which tiles
 * Notes, gba and waterfall; safe mode; the console switch; a broken client as
 * the control). Paced to 20 ticks per second because waterfall reads a real
 * SDR, whose sample clock is game time.
 */
@GameTestHolder(GuiTests.NS)
@PrefixGameTestTemplate(false)
public final class GuiTests {
    static final String NS = "ecm_gui";

    @GameTest(template = "gametest_empty", timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".basics")
    public static void gui_basics(GameTestHelper h) {
        TestDriver.scenario(h, NS, GuiScenarios.ALL.get("gui_basics"), true);
    }

    /** Client-side rendering (UI_PROTOCOL.md §14 phase 2a): scenario {@code gui_client_render}. */
    @GameTest(template = "gametest_empty", timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".client_render")
    public static void gui_client_render(GameTestHelper h) {
        TestDriver.scenario(h, NS, GuiScenarios.ALL.get("gui_client_render"), true);
    }
}
//?}
