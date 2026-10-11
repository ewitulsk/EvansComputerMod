package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.function.BooleanSupplier;

/**
 * Screen clusters, namespace {@code ecm_screen}.
 */
@GameTestHolder(ScreenTests.NS)
@PrefixGameTestTemplate(false)
public final class ScreenTests {
    static final String NS = "ecm_screen";
    private static final String STRUCTURE = "gametest_empty";
    private static final long WALL_LIMIT_MS = 60_000;

    private static final BlockPos TERMINAL = new BlockPos(6, 2, 3);
    /** West of the north-facing terminal, facing west: the terminal is behind it. */
    private static final BlockPos SCREEN = TERMINAL.west();

    /**
     * A program that owns the Screen at its own resolution (gba, 240x160)
     * keeps it through a block update next to the terminal. The rescan that
     * update causes used to see the display's size differ from the cluster's,
     * replace the display and power the screen off: the Screen went black a
     * moment into a game, as soon as the speaker's cone moved.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".owned")
    public static void program_keeps_screen_through_neighbor_update(GameTestHelper h) {
        TerminalDisplay[] owned = new TerminalDisplay[1];
        long[] at = new long[1];
        int[] dirty = new int[1];
        run(h, "program_keeps_screen_through_neighbor_update", List.of(
                () -> {
                    h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                            .setValue(TerminalBlock.FACING, Direction.NORTH));
                    h.setBlock(SCREEN, ModBlocks.SCREEN_BLOCK.get().defaultBlockState()
                            .setValue(ScreenBlock.FACING, Direction.WEST));
                    return true;
                },
                () -> {
                    if (terminal(h) == null) return false;
                    terminal(h).initializeWasm(); // what opening the GUI does
                    return true;
                },
                () -> terminal(h).hasScreenCluster()
                        && screen(h).contains("Welcome to Terminal OS"),
                () -> {
                    Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(terminal(h));
                    try {
                        Files.createDirectories(dir);
                        Files.copy(repoRoot().resolve("rust/wasm-programs/gba/tests/roms/arm.gba"),
                                dir.resolve("arm.gba"), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                    } catch (java.io.IOException ex) {
                        throw new AssertionError("can't copy the test ROM: " + ex);
                    }
                    terminal(h).onStringInput("gba arm.gba screen --no-sound\n");
                    return true;
                },
                () -> {
                    TerminalDisplay sd = terminal(h).getScreenDisplay();
                    if (sd == null || sd.getGfxWidth() != 240 || sd.getGfxHeight() != 160 || !anchorActive(h)) {
                        return false;
                    }
                    owned[0] = sd;
                    return true;
                },
                () -> {
                    // A block update beside the terminal, like a speaker's cone changing state.
                    h.setBlock(TERMINAL.above(), Blocks.STONE.defaultBlockState());
                    h.setBlock(TERMINAL.above(), Blocks.AIR.defaultBlockState());
                    at[0] = System.currentTimeMillis();
                    return true;
                },
                () -> System.currentTimeMillis() - at[0] > 1000,
                () -> {
                    TerminalDisplay sd = terminal(h).getScreenDisplay();
                    check(sd == owned[0], "the screen's display was replaced");
                    check(sd.getGfxWidth() == 240 && sd.getGfxHeight() == 160,
                            "the screen is " + sd.getGfxWidth() + "x" + sd.getGfxHeight());
                    check(anchorActive(h), "the screen was powered off");
                    dirty[0] = sd.getPixelDirtyCounter();
                    return true;
                },
                // gba is still drawing to it.
                () -> terminal(h).getScreenDisplay().getPixelDirtyCounter() != dirty[0]));
    }

    // ------------------------------------------------------------ helpers

    private static TerminalBlockEntity terminal(GameTestHelper h) {
        return h.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity t ? t : null;
    }

    private static String screen(GameTestHelper h) {
        TerminalBlockEntity t = terminal(h);
        return t == null ? "" : ScenarioRun.screen(t.getDisplay());
    }

    private static boolean anchorActive(GameTestHelper h) {
        return h.getBlockEntity(SCREEN) instanceof ScreenBlockEntity s && s.isActive();
    }

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        if (p == null) throw new AssertionError("repository root not found from " + Path.of("").toAbsolutePath());
        return p;
    }

    private static void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }

    /** Each step runs every tick until it returns true. */
    private static void run(GameTestHelper h, String name, List<BooleanSupplier> steps) {
        int[] step = {0};
        long[] started = {0};
        String[] failure = {null};
        TestDriver.drive(h, NS, name, () -> {
            if (started[0] == 0) started[0] = System.currentTimeMillis();
            if (System.currentTimeMillis() - started[0] > WALL_LIMIT_MS) {
                failure[0] = "timed out at step " + (step[0] + 1) + "/" + steps.size() + "\n" + screen(h);
                return false;
            }
            try {
                while (step[0] < steps.size() && steps.get(step[0]).getAsBoolean()) step[0]++;
            } catch (AssertionError | RuntimeException ex) {
                failure[0] = "step " + (step[0] + 1) + ": " + ex.getMessage();
                return false;
            }
            return step[0] >= steps.size();
        }, () -> failure[0]);
    }
}
//?}
