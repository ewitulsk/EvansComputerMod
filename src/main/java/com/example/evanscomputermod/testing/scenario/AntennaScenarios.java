package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaAnalyzerItem;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.entity.ChestBlockEntity;

import java.util.ArrayList;
import java.util.List;

/**
 * Antenna scenarios (Phase 4), registered through {@link RadioScenarios}.
 *
 * <p>{@code ham_dipole}: a 40 m-band half-wave dipole for 7 MHz. Two
 * fence posts 22 blocks apart carry an insulator each; 10 blocks of copper
 * wire run from each insulator to a feed point in the middle, 10 blocks above
 * a grass field. A chest by the west post holds an Antenna Analyzer and an
 * RF Wrench. Right-click the feed point with the analyzer: the antenna
 * resonates near 7.2 MHz with an SWR under 2:1 and a copper-wire-limited
 * rating of about 50 W. Control: cut one arm off the feed point with the
 * wrench and the analyzer reports a mismatched single-arm antenna.
 * <pre>
 *   y=0 grass x -12..12, z -2..2; posts (spruce fence) at x = ±11, y 1..9;
 *   insulators (±11, 10, 0); copper wire x -10..-1 and 1..10 at y 10;
 *   feed point (0, 10, 0), axis x; chest (-11, 1, 2).
 * </pre>
 */
public final class AntennaScenarios {
    static final BlockPos FEED = new BlockPos(0, 10, 0);
    static final int ARM = 10;
    /** Half-wave of a 20 m (tip to tip, centre to centre) wire: c / 40 m. */
    public static final double DESIGN_HZ = 299_792_458.0 / 40;

    private AntennaScenarios() {}

    public static Scenario hamDipole() {
        return Scenario.builder("ham_dipole", "a 7 MHz half-wave copper dipole on two posts, read with the antenna analyzer")
                .decor(new Dipole())
                .note("A 2 x 10 block copper dipole, 10 m up: right-click the feed point (middle) with the Antenna Analyzer")
                .mutate(AntennaScenarios::analyze, "analyzer on the feed point: expect resonance near 7.2 MHz, SWR under 2:1, ~50 W copper-wire limit")
                .note("Control: wrench-cut the east arm at the feed point and read it again - the match is gone")
                .timeLimit(45_000)
                .build();
    }

    /** {@code antenna_tools}: computer A beside the dipole's feed point; computer B beside a bare feed point. */
    static final BlockPos TOOLS_A = FEED.north(), TOOLS_B = new BlockPos(5, 1, 6), BARE_FEED = TOOLS_B.east();
    /** Resonance of the 2 x 10 block dipole as the tools print it: 6.6-7.4 MHz (a few % under c / 40 m). */
    static final String RESONANCE = "(6\\.[6-9]|7\\.[0-4])";

    /**
     * {@code antenna_tools}: the {@code antenna} program reading the
     * {@code antenna} peripheral of a feed point. A sits next to the
     * {@code ham_dipole} feed point: the summary names the resonance, the
     * 6-8 MHz SWR sweep dips under 2:1 near it, and the limit line names the
     * copper wire. Control: B sits next to a feed point with nothing on its
     * arms and reads "No antenna".
     */
    public static Scenario antennaTools() {
        return Scenario.builder("antenna_tools", "the antenna program on a computer next to a dipole's feed point, and next to a bare one")
                .host("A", TOOLS_A, "-")
                .host("B", TOOLS_B, "-")
                .decor(new Dipole())
                .decor(new BareFeed())
                .note("A is next to the 7 MHz dipole's feed point: summary, SWR sweep, impedance, limits")
                .send("A", "antenna")
                .expectOrFail("A", "^Resonant at " + RESONANCE + " MHz", "A reads the dipole's resonance", "^No antenna|^antenna: no antenna")
                .send("A", "antenna swr 6e6 8e6 21")
                .expectOrFail("A", "^antenna: min SWR 1\\.\\d\\d:1 at " + RESONANCE + "\\d\\d MHz; resonant at " + RESONANCE,
                        "the sweep dips under 2:1 at resonance", "^antenna: (no usable|min SWR (>|[2-9]|\\d\\d))")
                .expect("A", "^ *\\d\\.\\d \\|.*@", "the text plot marks the best point")
                .mutate(r -> logScreen(r, "A"), "log A's SWR plot")
                .send("A", "antenna z 7.1M")
                .expect("A", "^7\\.100 MHz: Z = \\d+\\.\\d [+-] j\\d+\\.\\d ohms, SWR \\d+\\.\\d\\d:1", "impedance at 7.1 MHz")
                .send("A", "antenna limits --amp 1k")
                .expect("A", "^Limited to \\d+ W by copper wire at \\(", "the limit names the wire")
                .expect("A", "^transmitter: amp 1\\.0 kW -> will overheat", "a 1 kW amp would overheat the wire")
                .mutate(r -> logScreen(r, "A"), "log A's limits")
                .note("Control: B's feed point has nothing on its arms")
                .send("B", "antenna")
                .expectOrFail("B", "^No antenna", "B reads no antenna", "^Resonant")
                .mutate(r -> logScreen(r, "B"), "log B's screen")
                .timeLimit(60_000)
                .build();
    }

    /** Writes what {@code node} printed for its last command to the log (the test's receipt). */
    static void logScreen(ScenarioRun run, String node) {
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info("[antenna_tools] {} screen:\n{}", node, run.latestOutput(node));
    }

    /** A feed point with nothing on its arms (axis z; B touches a coax side). */
    private static final class BareFeed implements Scenario.Decor {
        @Override
        public List<BlockPos> footprint() {
            return List.of(BARE_FEED);
        }

        @Override
        public void build(ScenarioRun run) {
            ConductorBlock.placeConnected(run.level(), run.abs(BARE_FEED), RadioAntennaContent.FEED_POINT.get().defaultBlockState()
                    .setValue(FeedPointBlock.AXIS, Direction.Axis.Z));
        }
    }

    /** The analyzer's reading, checked; fails the run when the dipole isn't a dipole. */
    static void analyze(ScenarioRun run) {
        BlockPos feed = run.abs(FEED);
        Antenna a = AntennaManager.solveNow(run.level(), feed, 30_000);
        List<String> lines = AntennaAnalyzerItem.reportLines(run.level(), a);
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info("[ham_dipole] analyzer: {}", String.join(" | ", lines));
        if (!a.solved()) { run.fail("antenna not solved: " + a.summary()); return; }
        double f = a.resonantHz();
        if (!(f > 0.88 * DESIGN_HZ && f < DESIGN_HZ)) { run.fail("resonance " + f + " Hz, expected just under " + DESIGN_HZ); return; }
        if (!(a.swrAt(f) < 2)) { run.fail("SWR at resonance " + a.swrAt(f)); return; }
        if (!lines.get(0).startsWith("Resonant at ")) run.fail("analyzer text: " + lines.get(0));
    }

    private static final class Dipole implements Scenario.Decor {
        final List<BlockPos> ground = new ArrayList<>(), posts = new ArrayList<>(), wire = new ArrayList<>();
        final BlockPos chest = new BlockPos(-11, 1, 2);

        Dipole() {
            for (int x = -12; x <= 12; x++) for (int z = -2; z <= 2; z++) ground.add(new BlockPos(x, 0, z));
            for (int y = 1; y < FEED.getY(); y++) { posts.add(new BlockPos(-ARM - 1, y, 0)); posts.add(new BlockPos(ARM + 1, y, 0)); }
            for (int i = 1; i <= ARM; i++) { wire.add(FEED.west(i)); wire.add(FEED.east(i)); }
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(ground);
            all.addAll(posts);
            all.addAll(wire);
            all.add(FEED);
            all.add(FEED.west(ARM + 1));
            all.add(FEED.east(ARM + 1));
            all.add(chest);
            return all;
        }

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            for (BlockPos p : ground) level.setBlock(run.abs(p), Blocks.GRASS_BLOCK.defaultBlockState(), 3);
            for (BlockPos p : posts) level.setBlock(run.abs(p), Blocks.SPRUCE_FENCE.defaultBlockState(), 3);
            ConductorBlock.placeConnected(level, run.abs(FEED.west(ARM + 1)), RadioAntennaContent.INSULATOR.get().defaultBlockState());
            ConductorBlock.placeConnected(level, run.abs(FEED.east(ARM + 1)), RadioAntennaContent.INSULATOR.get().defaultBlockState());
            ConductorBlock.placeConnected(level, run.abs(FEED), RadioAntennaContent.FEED_POINT.get().defaultBlockState()
                    .setValue(FeedPointBlock.AXIS, Direction.Axis.X));
            for (BlockPos p : wire) ConductorBlock.placeConnected(level, run.abs(p), RadioAntennaContent.COPPER_WIRE.get().defaultBlockState());
            level.setBlock(run.abs(chest), Blocks.CHEST.defaultBlockState().setValue(ChestBlock.FACING, Direction.NORTH), 3);
            if (level.getBlockEntity(run.abs(chest)) instanceof ChestBlockEntity c) {
                c.setItem(0, new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()));
                c.setItem(1, new ItemStack(RadioAntennaContent.RF_WRENCH.get()));
            }
        }

        @Override
        public void clear(ScenarioRun run) {
            if (run.level().getBlockEntity(run.abs(chest)) instanceof ChestBlockEntity c) c.clearContent();
        }
    }
}
//?}
