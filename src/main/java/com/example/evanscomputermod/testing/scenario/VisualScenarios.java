package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The Visual Programmer app (docs/gui/VISUAL_PROGRAMMER.md, ROADMAP P6),
 * spawnable with {@code /ecm scenario spawn <name>}; {@code visual_console}
 * is also the {@code ecm_visual} GameTest.
 *
 * <ul>
 *   <li>{@code visual_console}: a Terminal. {@code visual --install-samples}
 *       writes old-format {@code .vpl} programs and the designer-built counter
 *       app into {@code /visual}; an old graph generates the old generator's
 *       Python ({@code if / elif / else}); {@code visual --run} starts
 *       {@code python} as a normal process (a command session, nothing typed
 *       into this terminal) and shows its output; {@code visual --package}
 *       writes the counter app's folder with {@code app.toml}. Controls: a GUI
 *       app doesn't run headless; a wrong count fails at once.</li>
 *   <li>{@code visual_app}: starts the GUI, opens the editor on the counter
 *       program and runs the packaged counter app as a window.</li>
 * </ul>
 * <pre>
 *   Terminal at (0,1,0) facing north, built and booted by a player.
 * </pre>
 */
public final class VisualScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    private static final BlockPos PC = new BlockPos(0, 1, 0);

    static {
        for (Scenario s : List.of(console(), app())) ALL.put(s.name, s);
    }

    private VisualScenarios() {
    }

    private static Scenario console() {
        return Scenario.builder("visual_console",
                        "Visual Programmer from the console: old .vpl samples generate and run as a python process; package the counter app")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Install the samples: old .vpl graphs and the designer-built counter app")
                .send("pc", "visual --install-samples")
                .expect("pc", "^visual: installed 8 sample programs", "8 sample programs in /visual")
                .waitMs(1000, "the command exits")
                .note("An old graph generates the same Python as the old Java generator")
                .send("pc", "visual --python visual/branches.vpl")
                .expect("pc", "^elif len\\(_block2_result\\) > 3:$", "the if / elif / else chain")
                .waitMs(1000, "the command exits")
                .note("Run starts python as a normal process; its output comes back here")
                .send("pc", "visual --run visual/hello.vpl")
                .expect("pc", "^Hello, world!$", "hello.vpl prints Hello, world!", "^(?:Traceback|visual: )")
                .expect("pc", "^\\[finished\\]$", "and the run ends")
                .waitMs(1000, "the command exits")
                .send("pc", "visual --run visual/counter_loop.vpl")
                .expect("pc", "^5$", "counter_loop.vpl counts to 5", "^(?:[0-46-9]|Traceback)")
                .expect("pc", "^\\[finished\\]$", "and the run ends")
                .waitMs(1000, "the command exits")
                .note("Package the designer-built counter app")
                .send("pc", "visual --package visual/counter.vpl")
                .expect("pc", "^visual: packaged /visual/apps/dev\\.visual\\.counter \\(permissions: none\\)",
                        "the app folder is written, no permissions needed")
                .waitMs(1000, "the command exits")
                .send("pc", "cat visual/apps/dev.visual.counter/app.toml")
                .expect("pc", "^entry\\s+= \"main\\.py\"", "app.toml starts main.py")
                .waitMs(1000, "the command exits")
                .note("Control: a GUI app doesn't run headless")
                .send("pc", "visual --run visual/counter.vpl")
                .expect("pc", "is a GUI app", "visual refuses to run the counter app in the console")
                .timeLimit(60_000)
                .build();
    }

    private static Scenario app() {
        return Scenario.builder("visual_app",
                        "Visual Programmer in the GUI: the editor opens the counter program; the packaged counter app runs")
                .host("pc", PC, "-")
                .asPlayer()
                .realTime()
                .send("pc", "visual --install-samples")
                .expect("pc", "^visual: installed 8 sample programs", "8 sample programs in /visual")
                .waitMs(1000, "the command exits")
                .send("pc", "visual")
                .expect("pc", "Start the GUI with `gui`", "in console mode the editor says how to start the GUI")
                .waitMs(1000, "the command exits")
                .send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and shell start", "(?i)unknown command|no GUI device")
                .note("Ctrl+Alt+Backspace back to the console (the GUI keeps running) to type the next commands")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> GuiScenarios.ui(r) != null && !GuiScenarios.ui(r).guiVisible() ? null : "GUI still shown",
                        "the console is shown", 5_000)
                .send("pc", "gui launch visual visual/counter.vpl")
                .expect("pc", "^gui: launched visual", "the Visual Programmer starts")
                .until("pc", "gui status", "\"Visual Programmer\" .*\\d+x\\d+", "its window is open")
                .note("Package the counter app and run it as a GUI app: a window titled Counter")
                .send("pc", "visual --package visual/counter.vpl")
                .expect("pc", "^visual: packaged /visual/apps/dev\\.visual\\.counter", "the app folder is written")
                .waitMs(1000, "the command exits")
                .send("pc", "gui launch python /visual/apps/dev.visual.counter/main.py")
                .expect("pc", "^gui: launched python", "the counter app starts")
                .until("pc", "gui status", "\"Counter\" .*\\d+x\\d+", "the Counter window is open")
                .note("By hand (manual mode): click Add one twice; the label says Count: 2")
                .timeLimit(60_000)
                .build();
    }
}
//?}
