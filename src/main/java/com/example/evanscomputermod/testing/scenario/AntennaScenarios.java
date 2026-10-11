package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Antenna scenarios (Phase 4), registered through {@link RadioScenarios}.
 *
 * <p>{@code ham_dipole}: a 40 m-band half-wave dipole for 7 MHz, built by a
 * player. Two fence posts 22 blocks apart; an insulator on each; 10 blocks of
 * copper wire run from each insulator to a feed point in the middle, 10
 * blocks above a grass field (the feed point lines up with the wires it is
 * clicked between). A chest by the west post holds an Antenna Analyzer and an
 * RF Wrench. The player right-clicks the feed point with the analyzer: the
 * antenna resonates near 7.2 MHz with a 2:1 SWR band. Control: the player cuts
 * the east arm off the feed point with the wrench and the analyzer no longer
 * reports a matched 7 MHz dipole.
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
    static final BlockPos CHEST = new BlockPos(-11, 1, 2);

    private AntennaScenarios() {}

    public static Scenario hamDipole() {
        return Scenario.builder("ham_dipole", "a 7 MHz half-wave copper dipole on two posts, read with the antenna analyzer;"
                        + " control: the RF wrench cuts one arm and the match is gone")
                .asPlayer()
                .decor(new Dipole())
                .note("Built for you: posts, insulators, 2 x 10 blocks of copper wire, the feed point clicked in between, 10 m up."
                        + " The chest holds the Antenna Analyzer and RF Wrench.")
                .note("Right-click the feed point (middle) with the Antenna Analyzer")
                .await(r -> analyzer(r, FEED), "the analyzer reads the solved antenna", 30_000)
                .mutate(AntennaScenarios::checkDipole, "analyzer: resonant just under 7.5 MHz with a 2:1 SWR band")
                .note("Control: right-click the feed point's east side with the RF Wrench (cuts the east arm), then read it again")
                .mutate(r -> wrench(r, FEED, Direction.EAST), "east arm cut at the feed point")
                .await(r -> analyzer(r, FEED), "the analyzer reads the cut antenna", 30_000)
                .mutate(AntennaScenarios::checkCut, "analyzer: no longer a matched 7 MHz dipole")
                .note("Wrench the same side again to reconnect it")
                .mutate(r -> wrench(r, FEED, Direction.EAST), "east arm reconnected")
                .timeLimit(55_000)
                .build();
    }

    // ------------------------------------------------------------ the analyzer, as a player reads it

    /** The last solved analyzer summary per run (what the player read). */
    static final java.util.Map<ScenarioRun, String> READING = java.util.Collections.synchronizedMap(new java.util.WeakHashMap<>());
    static final java.util.Map<ScenarioRun, long[]> CLICKED = java.util.Collections.synchronizedMap(new java.util.WeakHashMap<>());

    /**
     * Right-click {@code feedRel} with the analyzer (again every 3 s, as a
     * player would while it says "solving") until a solved summary arrives in
     * chat; null then, with the summary kept for the checks.
     */
    static String analyzer(ScenarioRun r, BlockPos feedRel) {
        long now = System.currentTimeMillis();
        long[] at = CLICKED.computeIfAbsent(r, k -> new long[] {0});
        if (now - at[0] > 3_000) {
            at[0] = now;
            r.player().use(new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()), r.abs(feedRel), Direction.NORTH, false);
        }
        for (String line : r.player().heard()) {
            if (line.contains("solving") || line.startsWith("≈")) continue;
            if (line.startsWith("Resonant at") || line.startsWith("No resonance") || line.startsWith("No antenna")
                    || line.startsWith("Antenna can't")) {
                READING.put(r, line);
                at[0] = 0;
                r.say("§7  analyzer: " + String.join(" | ", r.player().heard()));
                com.example.evanscomputermod.EvansComputerMod.LOGGER.info("[{}] analyzer: {}", r.scenario().name,
                        String.join(" | ", r.player().heard()));
                return null;
            }
        }
        return "analyzer said: " + r.player().heard();
    }

    static final Pattern RESONANT = Pattern.compile("^Resonant at (\\d+(?:\\.\\d+)?) (k|M)Hz");

    /** The resonance in the last reading, Hz (NaN if none). */
    static double resonanceHz(ScenarioRun r) {
        String s = READING.getOrDefault(r, "");
        Matcher m = RESONANT.matcher(s);
        if (!m.find()) return Double.NaN;
        return Double.parseDouble(m.group(1)) * (m.group(2).equals("k") ? 1e3 : 1e6);
    }

    /** The 2:1 SWR band in the last reading as {low, high} Hz, or null. */
    static double[] swrBand(ScenarioRun r) {
        Matcher m = Pattern.compile("2:1 SWR band (\\d+(?:\\.\\d+)?)\\s*(?:(k|M)Hz)?\\s*[–-]\\s*(\\d+(?:\\.\\d+)?) (k|M)Hz")
                .matcher(READING.getOrDefault(r, ""));
        if (!m.find()) return null;
        double hiUnit = m.group(4).equals("k") ? 1e3 : 1e6;
        double loUnit = m.group(2) == null ? hiUnit : m.group(2).equals("k") ? 1e3 : 1e6;
        return new double[] {Double.parseDouble(m.group(1)) * loUnit, Double.parseDouble(m.group(3)) * hiUnit};
    }

    static void checkDipole(ScenarioRun r) {
        double f = resonanceHz(r);
        if (!(f > 0.88 * DESIGN_HZ && f < DESIGN_HZ)) {
            r.fail("analyzer: resonance " + f + " Hz, expected just under " + DESIGN_HZ + " (" + READING.get(r) + ")");
            return;
        }
        double[] band = swrBand(r);
        if (band == null || !(band[0] <= f && f <= band[1])) r.fail("analyzer: no 2:1 SWR band around resonance: " + READING.get(r));
    }

    static void checkCut(ScenarioRun r) {
        double f = resonanceHz(r);
        double[] band = swrBand(r);
        if (f > 0.88 * DESIGN_HZ && f < DESIGN_HZ && band != null)
            r.fail("control: the cut antenna still reads as a matched 7 MHz dipole: " + READING.get(r));
    }

    /** Right-click {@code side} of the conductor at {@code rel} with the RF Wrench (cuts or restores that side). */
    static void wrench(ScenarioRun r, BlockPos rel, Direction side) {
        var said = r.player().use(new ItemStack(RadioAntennaContent.RF_WRENCH.get()), r.abs(rel), side, false);
        r.say("§7  wrench: " + said);
        if (said.isEmpty()) r.fail("the wrench did nothing on " + side + " of " + r.abs(rel));
    }

    // ------------------------------------------------------------ antenna_tools

    /** {@code antenna_tools}: computer A beside the dipole's feed point; computer B beside a bare feed point. */
    static final BlockPos TOOLS_A = FEED.north(), TOOLS_B = new BlockPos(5, 1, 6), BARE_FEED = TOOLS_B.east();
    /** Resonance of the 2 x 10 block dipole as the tools print it: 6.6-7.4 MHz (a few % under c / 40 m). */
    static final String RESONANCE = "(6\\.[6-9]|7\\.[0-4])";

    /**
     * {@code antenna_tools}: the {@code antenna} program reading the
     * {@code antenna} peripheral of a feed point. A sits next to the
     * {@code ham_dipole} feed point: the summary names the resonance, the
     * 6-8 MHz SWR sweep dips under 2:1 near it, and the limit line names the
     * copper wire. Control: B sits next to a feed point placed on its own
     * on the grass, with nothing on its arms, and reads "No antenna".
     */
    public static Scenario antennaTools() {
        return Scenario.builder("antenna_tools", "the antenna program on a computer next to a dipole's feed point, and next to a bare one")
                .asPlayer()
                .host("A", TOOLS_A, "-")
                .host("B", TOOLS_B, "-")
                .decor(new Dipole())
                .decor(PlayerKit.decor(List.of(BARE_FEED), null,
                        r -> r.player().place(RadioAntennaContent.FEED_POINT.get(), r.abs(BARE_FEED), Direction.SOUTH), null))
                .note("Built for you: the ham_dipole antenna with computer A against the feed point's north (coax) side;"
                        + " computer B next to a feed point standing alone on the grass.")
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
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info("{}", screenLog(run.scenario().name, node, run.latestOutput(node)));
    }

    /** The log line {@link #logScreen} writes: {@code [<scenario>] <node> screen:} then the output. */
    public static String screenLog(String scenario, String node, String output) {
        return "[" + scenario + "] " + node + " screen:\n" + output;
    }

    /** Grass and fence posts (terrain); the player places insulators, wires inward from them, and the feed point last. */
    private static final class Dipole implements Scenario.Decor {
        final List<BlockPos> ground = new ArrayList<>(), posts = new ArrayList<>(), west = new ArrayList<>(), east = new ArrayList<>();

        Dipole() {
            for (int x = -12; x <= 12; x++) for (int z = -2; z <= 2; z++) ground.add(new BlockPos(x, 0, z));
            for (int y = 1; y < FEED.getY(); y++) { posts.add(new BlockPos(-ARM - 1, y, 0)); posts.add(new BlockPos(ARM + 1, y, 0)); }
            for (int i = ARM; i >= 1; i--) { west.add(FEED.west(i)); east.add(FEED.east(i)); }
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(ground);
            all.addAll(posts);
            all.addAll(west);
            all.addAll(east);
            all.add(FEED);
            all.add(FEED.west(ARM + 1));
            all.add(FEED.east(ARM + 1));
            all.add(CHEST);
            return all;
        }

        @Override
        public void terrain(ScenarioRun run) {
            PlayerKit.fill(run, ground, Blocks.GRASS_BLOCK.defaultBlockState());
            PlayerKit.fill(run, posts, Blocks.SPRUCE_FENCE.defaultBlockState());
            PlayerKit.chest(run, CHEST, Direction.NORTH, new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()),
                    new ItemStack(RadioAntennaContent.RF_WRENCH.get()));
        }

        @Override
        public void build(ScenarioRun run) {
            Block insulator = RadioAntennaContent.INSULATOR.get(), wire = RadioAntennaContent.COPPER_WIRE.get();
            run.player().place(insulator, run.abs(FEED.west(ARM + 1)), Direction.NORTH);
            run.player().place(insulator, run.abs(FEED.east(ARM + 1)), Direction.NORTH);
            List<BlockPos> w = new ArrayList<>(List.of(FEED.west(ARM + 1)));
            w.addAll(west);
            List<BlockPos> e = new ArrayList<>(List.of(FEED.east(ARM + 1)));
            e.addAll(east);
            PlayerKit.placeRun(run, wire, w.subList(1, w.size()), Direction.NORTH);
            PlayerKit.placeRun(run, wire, e.subList(1, e.size()), Direction.NORTH);
            run.player().placeAgainst(RadioAntennaContent.FEED_POINT.get(), run.abs(FEED), Direction.NORTH, Direction.WEST,
                    s -> s.getValue(FeedPointBlock.AXIS) == Direction.Axis.X);
        }

        @Override
        public void clear(ScenarioRun run) {
            PlayerKit.emptyChest(run, CHEST);
        }
    }

    static String fmtMhz(double hz) {
        return String.format(Locale.ROOT, "%.3f MHz", hz / 1e6);
    }
}
//?}
