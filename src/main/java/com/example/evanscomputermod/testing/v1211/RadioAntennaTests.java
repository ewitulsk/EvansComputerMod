package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaAnalyzerItem;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.ConductorBlockEntity;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.testing.scenario.AntennaScenarios;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.List;
import java.util.function.BooleanSupplier;

/**
 * Phase 4 antennas in the world, namespace {@code ecm_radio}: block wires
 * connecting and the wrench cutting, dipoles found and solved from a feed
 * point, detuning by touching metal, insulator splits, the analyzer text and
 * the no-antenna control, plus the {@code ham_dipole} scenario.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioAntennaTests {
    private static final String NS = RadioTests.NS;
    private static final String STRUCTURE = RadioTests.STRUCTURE;
    /** Dipole feed (relative), 12 blocks above the structure floor; arms along x. */
    private static final BlockPos FEED = new BlockPos(20, 12, 20);

    private RadioAntennaTests() {}

    // ------------------------------------------------------------ helpers

    /** Places a block through its BlockItem, as a player's click would. */
    static void place(GameTestHelper h, Player player, Block block, BlockPos rel) {
        BlockPos abs = h.absolutePos(rel);
        ItemStack stack = new ItemStack(block.asItem());
        BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf(abs), Direction.UP, abs, false);
        ((BlockItem) stack.getItem()).place(new BlockPlaceContext(h.getLevel(), player, InteractionHand.MAIN_HAND, stack, hit));
    }

    static void placeConnected(GameTestHelper h, BlockState state, BlockPos rel) {
        ConductorBlock.placeConnected(h.getLevel(), h.absolutePos(rel), state);
    }

    /** A feed point at {@link #FEED} (axis x) with {@code west}/{@code east} copper-wire blocks. */
    static void dipole(GameTestHelper h, int west, int east) {
        placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X), FEED);
        for (int i = 1; i <= west; i++) placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), FEED.west(i));
        for (int i = 1; i <= east; i++) placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), FEED.east(i));
    }

    static boolean connected(GameTestHelper h, BlockPos rel, Direction side) {
        BlockState s = h.getBlockState(rel);
        return s.getBlock() instanceof ConductorBlock && s.getValue(ConductorBlock.property(side));
    }

    static Antenna antenna(GameTestHelper h) {
        return AntennaManager.get(h.getLevel(), h.absolutePos(FEED));
    }

    /** Runs {@code steps} in order, one per tick until each returns true. */
    static void steps(GameTestHelper h, String name, String[] failure, List<BooleanSupplier> steps) {
        int[] i = {0};
        long start = System.currentTimeMillis();
        TestDriver.drive(h, NS, name, () -> {
            if (System.currentTimeMillis() - start > 50_000) {
                failure[0] = "timed out at step " + (i[0] + 1);
                return false;
            }
            try {
                while (i[0] < steps.size() && failure[0] == null && steps.get(i[0]).getAsBoolean()) i[0]++;
            } catch (RuntimeException e) {
                failure[0] = "step " + (i[0] + 1) + ": " + e;
            }
            return i[0] >= steps.size() && failure[0] == null;
        }, () -> failure[0]);
    }

    static void check(boolean ok, String[] failure, String why) {
        if (!ok && failure[0] == null) failure[0] = why;
    }

    // ------------------------------------------------------------ tests

    /**
     * Wires placed like a player places them join their neighbours on both
     * sides; the wrench on an arm cuts that side (and the neighbour's arm
     * goes too); a second click restores it. Bare wire joins an insulator but
     * not coax (control).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wires")
    public static void wires_autoconnect_and_wrench_cuts(GameTestHelper h) {
        String[] failure = {null};
        Player player = h.makeMockPlayer(GameType.SURVIVAL);
        BlockPos a = new BlockPos(5, 3, 5), b = a.east(), c = b.east();
        place(h, player, RadioAntennaContent.COPPER_WIRE.get(), a);
        place(h, player, RadioAntennaContent.COPPER_WIRE.get(), b);
        place(h, player, RadioAntennaContent.COPPER_WIRE.get(), c);
        place(h, player, RadioAntennaContent.INSULATOR.get(), c.east());
        place(h, player, RadioAntennaContent.COAX_CABLE.get(), a.west());
        check(connected(h, a, Direction.EAST) && connected(h, b, Direction.WEST) && connected(h, b, Direction.EAST)
                && connected(h, c, Direction.WEST), failure, "three wires in a row did not join");
        check(connected(h, c, Direction.EAST), failure, "wire did not join the insulator");
        check(!connected(h, a, Direction.WEST) && !connected(h, a.west(), Direction.EAST), failure, "control: bare wire joined coax");
        check(!connected(h, a, Direction.UP), failure, "arm towards air");

        // Wrench on b's east arm (hit 0.35 east of its centre).
        ItemStack wrench = new ItemStack(RadioAntennaContent.RF_WRENCH.get());
        player.setItemInHand(InteractionHand.MAIN_HAND, wrench);
        BlockPos babs = h.absolutePos(b);
        BlockHitResult armHit = new BlockHitResult(Vec3.atCenterOf(babs).add(0.35, 0.03, 0), Direction.UP, babs, false);
        h.getBlockState(b).useItemOn(wrench, h.getLevel(), player, InteractionHand.MAIN_HAND, armHit);
        check(!connected(h, b, Direction.EAST) && !connected(h, c, Direction.WEST), failure, "wrench didn't cut b-c");
        check(connected(h, a, Direction.EAST) && connected(h, b, Direction.WEST), failure, "wrench cut the wrong side");
        check(h.getLevel().getBlockEntity(babs) instanceof ConductorBlockEntity be && be.isCut(Direction.EAST), failure,
                "cut not stored in the block entity");
        // A neighbour update must not re-join a cut side.
        h.setBlock(c.above(), Blocks.STONE);
        check(!connected(h, c, Direction.WEST), failure, "cut side re-joined after a neighbour update");
        h.getBlockState(b).useItemOn(wrench, h.getLevel(), player, InteractionHand.MAIN_HAND, armHit);
        check(connected(h, b, Direction.EAST) && connected(h, c, Direction.WEST), failure, "second click didn't restore b-c");
        TestDriver.drive(h, NS, "wires_autoconnect_and_wrench_cuts", () -> failure[0] == null, () -> failure[0]);
    }

    /**
     * A copper half-wave dipole (2 x 10 blocks + feed point, 20 m centre to
     * centre) 12 blocks up is found from its feed point and solved: resonant a
     * few percent under c/40 m = 7.49 MHz, SWR under 2:1 there, copper wire
     * the weakest link at tens of watts.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".dipole")
    public static void dipole_resonates_near_design_frequency(GameTestHelper h) {
        String[] failure = {null};
        dipole(h, 10, 10);
        steps(h, "dipole_resonates_near_design_frequency", failure, List.of(
                () -> antenna(h).solved(),
                () -> {
                    Antenna a = antenna(h);
                    double f = a.resonantHz(), design = AntennaScenarios.DESIGN_HZ;
                    check(a.graph() != null && a.graph().blockCount == 21, failure, "graph blocks " + (a.graph() == null ? -1 : a.graph().blockCount));
                    check(f > 0.88 * design && f < design, failure, "resonance " + f + " vs half-wave " + design);
                    check(a.swrAt(f) < 2, failure, "SWR at resonance " + a.swrAt(f));
                    check(a.swrAt(f * 1.2) > 3, failure, "no mismatch 20% off resonance: " + a.swrAt(f * 1.2));
                    check(a.powerLimitW() > 20 && a.powerLimitW() < 120 && a.weakestLink().equals("copper wire"), failure,
                            "limit " + a.powerLimitW() + " W by " + a.weakestLink());
                    check(a.pattern(f).peakGainDbi() > 2, failure, "pattern peak " + a.pattern(f).peakGainDbi());
                    return true;
                }));
    }

    /** An iron block touching the end of an arm joins the antenna and pulls the resonance down. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".detune")
    public static void touching_iron_detunes_dipole(GameTestHelper h) {
        String[] failure = {null};
        double[] clean = {0};
        dipole(h, 10, 10);
        steps(h, "touching_iron_detunes_dipole", failure, List.of(
                () -> antenna(h).solved(),
                () -> {
                    clean[0] = antenna(h).resonantHz();
                    h.setBlock(FEED.east(11), Blocks.IRON_BLOCK);
                    return true;
                },
                () -> antenna(h).solved() && antenna(h).graph().blockCount == 22,
                () -> {
                    double f = antenna(h).resonantHz();
                    check(f < clean[0] * 0.98, failure, "iron block didn't detune: " + clean[0] + " -> " + f);
                    return true;
                }));
    }

    /**
     * An insulator in place of the 5th block of the east arm ends that arm
     * there: the outer five blocks drop out of the antenna and it resonates
     * higher; the insulated end is recorded for the voltage rating.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".insulator")
    public static void insulator_gap_splits_antenna(GameTestHelper h) {
        String[] failure = {null};
        dipole(h, 10, 10);
        placeConnected(h, RadioAntennaContent.INSULATOR.get().defaultBlockState(), FEED.east(5));
        steps(h, "insulator_gap_splits_antenna", failure, List.of(
                () -> antenna(h).solved(),
                () -> {
                    Antenna a = antenna(h);
                    check(a.graph().blockCount == 1 + 10 + 4, failure, "graph blocks " + a.graph().blockCount);
                    check(a.graph().insulatedPoints.size() == 1, failure, "insulated ends " + a.graph().insulatedPoints.size());
                    check(a.resonantHz() > 1.15 * AntennaScenarios.DESIGN_HZ, failure, "short side should resonate higher: " + a.resonantHz());
                    return true;
                }));
    }

    /**
     * The analyzer's text for a dipole names resonance, 2:1 band and the
     * rating; the control, a feed point with no wire, reports "No antenna".
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".analyzer")
    public static void analyzer_reports_dipole_and_no_antenna(GameTestHelper h) {
        String[] failure = {null};
        dipole(h, 10, 10);
        BlockPos lonely = new BlockPos(5, 6, 5);
        placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState(), lonely);
        steps(h, "analyzer_reports_dipole_and_no_antenna", failure, List.of(
                () -> antenna(h).solved(),
                () -> {
                    List<String> lines = AntennaAnalyzerItem.reportLines(h.getLevel(), antenna(h));
                    com.example.evanscomputermod.EvansComputerMod.LOGGER.info("[ecm_radio] analyzer: {}", lines);
                    check(lines.get(0).matches("Resonant at \\d+\\.\\d MHz · 2:1 SWR band \\d+\\.\\d–\\d+\\.\\d MHz · rated \\d+ W \\(copper wire\\) / .+ \\(.+\\)"),
                            failure, "analyzer line: " + lines.get(0));
                    check(lines.size() >= 2 && lines.get(1).contains("ground: average soil"), failure, "details: " + lines);
                    Antenna none = AntennaManager.get(h.getLevel(), h.absolutePos(lonely));
                    List<String> control = AntennaAnalyzerItem.reportLines(h.getLevel(), none);
                    check(none.report().status() == AntennaReport.Status.NO_ANTENNA && control.get(0).startsWith("No antenna"),
                            failure, "control: " + control);
                    check(none.powerLimitW() == 0 && Double.isInfinite(none.swrAt(7e6)), failure, "control has a rating");
                    return true;
                }));
    }

    /**
     * Fine Wire on a feed point's lugs (no block wire: a compact feed) makes
     * a VHF dipole: two 1 m runs plus the 1/8 m gap resonate a few percent
     * under c / (2 x 2.125 m) = 70.5 MHz, rated about 5 W (fine wire).
     * The runs are spawned as wire entities on the lug terminals (what the
     * Fine Wire item creates); routing itself is covered by ecm_sensor.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".finewire")
    public static void fine_wire_vhf_dipole(GameTestHelper h) {
        String[] failure = {null};
        BlockPos rel = new BlockPos(20, 8, 20);
        placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X), rel);
        BlockPos feed = h.absolutePos(rel);
        var level = h.getLevel();
        for (int t = 0; t < 2; t++) {
            var wire = com.example.evanscomputermod.sensor.wire.BlockWireEntity.create(level,
                    new com.example.evanscomputermod.sensor.wire.BlockWireEndpoint(feed, t),
                    new ItemStack(com.example.evanscomputermod.sensor.SensorContent.SENSOR_WIRE.get(), 8),
                    List.of(com.example.evanscomputermod.sensor.wire.BlockWireEntity.Point.x(t == 0 ? -1f : 1f)));
            level.addFreshEntity(wire);
        }
        double design = 299_792_458.0 / (2 * 2.125);
        steps(h, "fine_wire_vhf_dipole", failure, List.of(
                () -> AntennaManager.get(level, feed).solved(),
                () -> {
                    Antenna a = AntennaManager.get(level, feed);
                    com.example.evanscomputermod.EvansComputerMod.LOGGER.info("[ecm_radio] fine wire: {} | {}", a.summary(), a.details());
                    check(a.graph().hasFineWire() && a.graph().edges.size() == 2, failure, "fine-wire edges " + a.graph().edges.size());
                    check(a.resonantHz() > 0.85 * design && a.resonantHz() < design, failure, "resonance " + a.resonantHz() + " vs " + design);
                    check(a.swrAt(a.resonantHz()) < 2, failure, "SWR " + a.swrAt(a.resonantHz()));
                    check(a.wireLimitW() > 2 && a.wireLimitW() < 10 && a.weakestLink().equals("fine wire"), failure,
                            "fine wire limit " + a.wireLimitW() + " by " + a.weakestLink());
                    return true;
                }));
    }

    /**
     * Sable assembles a small dipole (with a wrench cut in one arm) and its
     * floor into a structure: in the plot the feed point finds the same
     * antenna (same blocks, cut kept, same resonance), ground is the
     * structure's own floor, and the pose maps back to where it was built.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".sable")
    public static void antenna_survives_sable_assembly(GameTestHelper h) {
        String[] failure = {null};
        var level = h.getLevel();
        BlockPos feedRel = new BlockPos(20, 4, 20);
        for (int x = 16; x <= 24; x++) h.setBlock(new BlockPos(x, 2, 20), Blocks.STONE);
        placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X), feedRel);
        for (int i = 1; i <= 3; i++) placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feedRel.west(i));
        for (int i = 1; i <= 4; i++) placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feedRel.east(i));
        BlockPos cutAt = h.absolutePos(feedRel.east(3));
        ((ConductorBlock) RadioAntennaContent.COPPER_WIRE.get()).toggleCut(level, cutAt, Direction.EAST);
        BlockPos oldFeed = h.absolutePos(feedRel);
        double[] before = {0};
        String[] groundBefore = {""};
        BlockPos[] moved = {null};
        dev.ryanhcode.sable.sublevel.ServerSubLevel[] sub = {null};
        steps(h, "antenna_survives_sable_assembly", failure, List.of(
                () -> AntennaManager.get(level, oldFeed).solved(),
                () -> {
                    Antenna a = AntennaManager.get(level, oldFeed);
                    check(a.graph().blockCount == 7, failure, "cut arm block counted before the move: " + a.graph().blockCount);
                    before[0] = a.resonantHz();
                    groundBefore[0] = a.graph().groundName;
                    List<BlockPos> blocks = new java.util.ArrayList<>();
                    BlockPos min = h.absolutePos(new BlockPos(16, 2, 20)), max = h.absolutePos(new BlockPos(24, 4, 20));
                    for (BlockPos p : BlockPos.betweenClosed(min, max)) if (!level.getBlockState(p).isAir()) blocks.add(p.immutable());
                    sub[0] = dev.ryanhcode.sable.api.SubLevelAssemblyHelper.assembleBlocks(level, oldFeed, blocks,
                            new dev.ryanhcode.sable.companion.math.BoundingBox3i(min, max));
                    check(!(level.getBlockState(oldFeed).getBlock() instanceof FeedPointBlock), failure, "feed point did not move");
                    return true;
                },
                () -> {
                    var plot = sub[0].getPlot();
                    for (int cx = plot.getChunkMin().x; cx <= plot.getChunkMax().x && moved[0] == null; cx++)
                        for (int cz = plot.getChunkMin().z; cz <= plot.getChunkMax().z && moved[0] == null; cz++)
                            for (BlockPos p : level.getChunk(cx, cz).getBlockEntities().keySet())
                                if (level.getBlockState(p).getBlock() instanceof FeedPointBlock) moved[0] = p.immutable();
                    check(moved[0] != null, failure, "no feed point in the structure's plot");
                    return true;
                },
                () -> AntennaManager.get(level, moved[0]).solved(),
                () -> {
                    Antenna a = AntennaManager.get(level, moved[0]);
                    com.example.evanscomputermod.EvansComputerMod.LOGGER.info("[ecm_radio] on structure: {} | {}", a.summary(), a.details());
                    check(a.graph().blockCount == 7, failure, "wrench cut lost in the move: " + a.graph().blockCount + " blocks");
                    check(Math.abs(a.resonantHz() - before[0]) < 0.01 * before[0], failure, "resonance changed: " + before[0] + " -> " + a.resonantHz());
                    check(a.graph().groundName.equals(groundBefore[0]), failure, "ground " + groundBefore[0] + " -> " + a.graph().groundName);
                    check(level.getBlockEntity(moved[0].east(3)) instanceof ConductorBlockEntity be && be.isCut(Direction.EAST), failure,
                            "cut mask not carried by the block entity");
                    var pose = a.pose(level);
                    double d = Math.sqrt(Math.pow(pose.x() - (oldFeed.getX() + 0.5), 2) + Math.pow(pose.y() - (oldFeed.getY() + 0.5), 2)
                            + Math.pow(pose.z() - (oldFeed.getZ() + 0.5), 2));
                    check(d < 1.0, failure, "pose " + pose + " is " + d + " m from where the feed was built");
                    return true;
                }));
    }

    /** The ham_dipole scenario, as spawned by {@code /ecm scenario spawn ham_dipole}. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ham_dipole")
    public static void ham_dipole(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("ham_dipole"));
    }
}
//?}
