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
