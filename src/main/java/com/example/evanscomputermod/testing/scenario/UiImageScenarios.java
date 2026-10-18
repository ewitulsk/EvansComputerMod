package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ui.Compositor;
import net.minecraft.core.BlockPos;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Large image resources in an app window on a Terminal (docs/gui/UI_PROTOCOL.md
 * §6), as a player sees them; also the {@code ecm_apps} GameTest {@code gui_large_image}.
 *
 * <p>{@code gui_large_image}: the player starts the GUI, opens the Terminal app and runs
 * {@code apptest ui-image 160 page}: a window like Music's Now Playing, a 160x160 cover
 * (100 KiB of RGBA) on a scrolling page that redraws every loop turn, with a "Clicks N"
 * button. The player clicks the button again and again for several seconds: every click
 * must come back as a redraw.
 */
public final class UiImageScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    private UiImageScenarios() {}

    private static Compositor comp(ScenarioRun r) {
        return GuiScenarios.comp(r);
    }

    private static String clickCount(ScenarioRun r, int n) {
        Compositor c = comp(r);
        if (c == null) return "no GUI";
        int[] at = c.findText("Image Test", "Clicks " + n);
        if (at == null) return "the button doesn't show 'Clicks " + n + "'";
        GuiScenarios.click(r, at);
        return null;
    }

    private static String showsCount(ScenarioRun r, int n) {
        Compositor c = comp(r);
        if (c == null) return "no GUI";
        return c.findText("Image Test", "Clicks " + n) != null ? null : "the window doesn't show 'Clicks " + n + "' (GUI: "
                + GuiScenarios.ui(r).status() + ")";
    }

    private static Scenario.Builder clicks(Scenario.Builder b, int side, int n) {
        GuiScenarios.terminalCommand(b, "apptest ui-image " + side + " page");
        b.await(r -> showsCount(r, 0), "the " + side + "x" + side + " image window opens", 20_000);
        for (int i = 0; i < n; i++) {
            int k = i;
            b.await(r -> clickCount(r, k), "click 'Clicks " + k + "'", 3_000)
                    .await(r -> showsCount(r, k + 1), "the app answers: 'Clicks " + (k + 1) + "'", 3_000)
                    .waitMs(400, "a pause between clicks");
        }
        return b;
    }

    static Scenario largeImage() {
        var b = Scenario.builder("gui_large_image",
                        "an app window with a 160x160 image keeps answering clicks for seconds")
                .asPlayer()
                .realTime()
                .timeLimit(120_000);
        b.host("pc", new BlockPos(0, 1, 0), "-");
        b.send("pc", "gui")
                .expect("pc", "^gui: started", "the GUI starts with the desktop", "(?i)unknown command|no GUI device")
                .await(r -> comp(r) != null && comp(r).shellConnected() ? null : "no desktop yet", "the desktop is connected", 15_000);
        GuiScenarios.launch(b, "Terminal", "terminal");
        b.note("A 160x160 cover on a page that redraws every turn; click its button for several seconds");
        clicks(b, 160, 14);
        return b.build();
    }

    static {
        for (Scenario s : List.of(largeImage())) ALL.put(s.name, s);
    }
}
//?}
