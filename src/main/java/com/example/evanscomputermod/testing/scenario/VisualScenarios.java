package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;

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
 *   <li>{@code visual_live_catalog} (docs/CC_COMPAT.md): the palette follows
 *       what is attached. The player types {@code tanks.vpl} in, a program
 *       whose block reads an inventory's tanks; it carries that block's
 *       signature, so {@code visual --python} still generates the call with
 *       nothing attached (control). With the editor open, its palette's
 *       Inventory section holds the 6 inventory blocks (control: not the tank
 *       ones); the player places a Create Basin (items and fluids) east of
 *       the computer and, within a couple of seconds and without reopening
 *       the editor, the palette says new blocks arrived and Inventory holds 9.
 *       {@code visual --run tanks.vpl} then runs the block against the Basin
 *       and prints its (empty) tanks. Needs Create (loaded in the test runs).</li>
 * </ul>
 * <pre>
 *   Terminal at (0,1,0) facing north, built and booted by a player.
 * </pre>
 */
public final class VisualScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos BASIN = new BlockPos(1, 1, 0);
    /** Start -> an inventory's tanks -> as text -> print, with the tank block's signature saved in the file. */
    private static final String TANKS_VPL = "{\"version\":1,\"name\":\"tanks\",\"canvas\":{\"offsetX\":0.0,\"offsetY\":0.0,\"zoom\":1.0},"
            + "\"blocks\":[{\"id\":1,\"type\":\"start\",\"x\":0.0,\"y\":0.0,\"inputs\":{}},"
            + "{\"id\":2,\"type\":\"inventory_tanks\",\"x\":0.0,\"y\":80.0,\"inputs\":{}},"
            + "{\"id\":3,\"type\":\"print\",\"x\":0.0,\"y\":180.0,\"inputs\":{}},"
            + "{\"id\":4,\"type\":\"to_str\",\"x\":200.0,\"y\":120.0,\"inputs\":{}}],"
            + "\"connections\":[{\"fromBlock\":1,\"fromPort\":\"flow\",\"toBlock\":2,\"toPort\":\"flow\"},"
            + "{\"fromBlock\":2,\"fromPort\":\"flow\",\"toBlock\":3,\"toPort\":\"flow\"},"
            + "{\"fromBlock\":2,\"fromPort\":\"result\",\"toBlock\":4,\"toPort\":\"value\"},"
            + "{\"fromBlock\":4,\"fromPort\":\"result\",\"toBlock\":3,\"toPort\":\"text\"}],"
            + "\"peripherals\":[[\"inventory\",\"\",[[\"tanks\",\"The fluid in each tank\",[],\"string\"]]]]}";

    static {
        for (Scenario s : List.of(console(), app(), liveCatalog())) ALL.put(s.name, s);
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

    private static Scenario liveCatalog() {
        var b = Scenario.builder("visual_live_catalog",
                        "The Visual Programmer's palette follows what is attached: a Create Basin placed by the computer adds tank blocks")
                .host("pc", PC, "-")
                .asPlayer()
                .realTime()
                .note("Type the program in (tanks.vpl: Start -> an inventory's tanks -> print), from Python");
        PlayerKit.typeFiles(b, Map.of("pc", Map.of("/visual/tanks.vpl", TANKS_VPL)));
        return b.await(r -> net.neoforged.fml.ModList.get().isLoaded("create") ? null : "this scenario needs Create",
                        "Create is loaded (the Basin comes from it)", 1_000)
                .note("Control: with nothing attached the saved signature still makes the call")
                .send("pc", "visual --python visual/tanks.vpl")
                .expect("pc", "peripheral\\.find\\(\"inventory\"\\)\\)\\.tanks\\(\\)", "the tank block is in the program, from its saved signature",
                        "skipped unknown|visual: ")
                .waitMs(1000, "the command exits")
                .send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and shell start", "(?i)unknown command|no GUI device")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> GuiScenarios.ui(r) != null && !GuiScenarios.ui(r).guiVisible() ? null : "GUI still shown",
                        "the console is shown", 5_000)
                .send("pc", "gui launch visual")
                .expect("pc", "^gui: launched visual", "the Visual Programmer starts")
                .until("pc", "gui status", "\"Visual Programmer\" .*\\d+x\\d+", "its window is open")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> GuiScenarios.ui(r) != null && GuiScenarios.ui(r).guiVisible() ? null : "the console is still shown",
                        "the GUI is shown", 5_000)
                .await(r -> GuiScenarios.shows(r, "visual", "+ Inventory (6)"), "the palette's Inventory section has its 6 inventory blocks", 15_000)
                .await(r -> GuiScenarios.shows(r, "visual", "+ Inventory (9)") == null ? "it has the tank blocks already" : null,
                        "control: no tank blocks while no tank is attached", 1_000)
                .note("Place a Create Basin east of the computer, with the editor open")
                .mutate(VisualScenarios::placeBasin, "place a Create Basin east of the computer")
                .await(r -> GuiScenarios.shows(r, "visual", "New peripheral blocks"), "the editor says new blocks arrived", 10_000)
                .await(r -> GuiScenarios.shows(r, "visual", "+ Inventory (9)"), "Inventory now has 9 blocks: tanks, push_fluid and pull_fluid too", 5_000)
                .note("Run the program: its tank block reads the Basin")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> GuiScenarios.ui(r) != null && !GuiScenarios.ui(r).guiVisible() ? null : "GUI still shown",
                        "the console is shown", 5_000)
                .send("pc", "visual --run visual/tanks.vpl")
                .expect("pc", "^\\[\\]$", "the Basin's tanks: none filled", "Traceback|Error")
                .timeLimit(90_000)
                .build();
    }

    /** The player places a Create Basin against the computer's east side. */
    private static void placeBasin(ScenarioRun r) {
        Block basin = BuiltInRegistries.BLOCK.get(ResourceLocation.parse("create:basin"));
        if (basin == Blocks.AIR) throw new IllegalStateException("no create:basin (is Create loaded?)");
        r.player().placeAgainst(basin, r.abs(BASIN), Direction.SOUTH, Direction.WEST, s -> s.is(basin));
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
