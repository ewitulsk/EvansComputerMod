package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.amp.AmplifierBlockEntity;
import com.example.evanscomputermod.radio.amp.ExciterLink;
import com.example.evanscomputermod.radio.amp.RadioAmpContent;
import com.example.evanscomputermod.radio.amp.TunerBlockEntity;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaAnalyzerItem;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.hazard.RadioHazardContent;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.power.BurnerGeneratorBlockEntity;
import com.example.evanscomputermod.radio.power.RadioPowerContent;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.radio.sdr.SdrBlockEntity;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LightningBolt;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.UUID;
import java.util.function.Function;

/**
 * Radio power and hazard scenarios (lanes 4D/5C), registered through
 * {@link RadioScenarios}; the station builder is shared with
 * {@code RadioPowerTests}.
 *
 * <p>{@code ham_station}: a complete HF station on a grass field.
 * <pre>
 *   burner generator (1,1,2) ─FE─ 100 W amplifier (0,1,2) ─ SDR (0,1,3) ─ computer (-1,1,3)
 *   amplifier ─ coax (0,1,1) ─ antenna tuner (0,1,0) ─ lightning arrestor (0,2,0)
 *   ─ coax (0,3..9,0) ─ feed point (0,10,0) ─ antenna wire x -10..10 ─ insulators on posts at x = ±11
 * </pre>
 * The SDR finds the chain and radiates from the dipole; keying it at 5 W
 * drives the amplifier to 100 W, drawing 40 FE/t from the generator only
 * while transmitting; the tuner hides the dipole's mismatch from the
 * amplifier. Controls: lightning on the dipole does nothing (arrestor), and
 * with no FE the amplifier browns out to a 5 W bypass without damage. The
 * scenario is built and run by a player ({@link #hamStation}); {@link #build}
 * and the operating helpers below set the same station up directly for the
 * focused {@code RadioPowerTests}.
 */
public final class PowerScenarios {
    public static final BlockPos FEED = new BlockPos(0, 10, 0);
    public static final BlockPos SDR = new BlockPos(0, 1, 3);
    public static final BlockPos AMP = new BlockPos(0, 1, 2);
    public static final BlockPos TUNER = new BlockPos(0, 1, 0);
    public static final BlockPos ARRESTOR = new BlockPos(0, 2, 0);
    public static final BlockPos GENERATOR = new BlockPos(1, 1, 2);
    public static final BlockPos COMPUTER = new BlockPos(-1, 1, 3);
    public static final BlockPos CHEST = new BlockPos(-2, 1, 0);
    public static final int ARM = 10;
    public static final int SAMPLE_RATE = 48_000;

    /** What to build: wire tier, amplifier (null = plain coax there), tuner, arrestor, generator/posts/field. */
    public record Station(Block wire, @Nullable Block amp, boolean tuner, boolean arrestor, boolean dressing) {}

    private PowerScenarios() {}

    // ------------------------------------------------------------ builder (scenario + tests)

    public static List<BlockPos> footprint(Station s) {
        List<BlockPos> all = new ArrayList<>();
        if (s.dressing) {
            for (int x = -12; x <= 12; x++) for (int z = -2; z <= 5; z++) all.add(new BlockPos(x, 0, z));
            for (int y = 1; y < FEED.getY(); y++) { all.add(new BlockPos(-ARM - 1, y, 0)); all.add(new BlockPos(ARM + 1, y, 0)); }
            all.add(GENERATOR);
            all.add(CHEST);
        }
        for (int i = 1; i <= ARM + 1; i++) { all.add(FEED.west(i)); all.add(FEED.east(i)); }
        all.add(FEED);
        for (int y = 1; y < FEED.getY(); y++) all.add(new BlockPos(0, y, 0));
        all.add(new BlockPos(0, 1, 1));
        all.add(AMP);
        all.add(SDR);
        return all;
    }

    /** Builds a station; {@code abs} maps layout positions to world positions. */
    public static void build(ServerLevel level, Function<BlockPos, BlockPos> abs, Station s) {
        if (s.dressing) {
            for (int x = -12; x <= 12; x++) for (int z = -2; z <= 5; z++)
                level.setBlock(abs.apply(new BlockPos(x, 0, z)), Blocks.GRASS_BLOCK.defaultBlockState(), 3);
            for (int y = 1; y < FEED.getY(); y++) {
                level.setBlock(abs.apply(new BlockPos(-ARM - 1, y, 0)), Blocks.SPRUCE_FENCE.defaultBlockState(), 3);
                level.setBlock(abs.apply(new BlockPos(ARM + 1, y, 0)), Blocks.SPRUCE_FENCE.defaultBlockState(), 3);
            }
        }
        ConductorBlock.placeConnected(level, abs.apply(FEED.west(ARM + 1)), RadioAntennaContent.INSULATOR.get().defaultBlockState());
        ConductorBlock.placeConnected(level, abs.apply(FEED.east(ARM + 1)), RadioAntennaContent.INSULATOR.get().defaultBlockState());
        ConductorBlock.placeConnected(level, abs.apply(FEED), RadioAntennaContent.FEED_POINT.get().defaultBlockState()
                .setValue(FeedPointBlock.AXIS, Direction.Axis.X));
        for (int i = 1; i <= ARM; i++) {
            ConductorBlock.placeConnected(level, abs.apply(FEED.west(i)), s.wire.defaultBlockState());
            ConductorBlock.placeConnected(level, abs.apply(FEED.east(i)), s.wire.defaultBlockState());
        }
        var coax = RadioAntennaContent.COAX_CABLE.get().defaultBlockState();
        for (int y = FEED.getY() - 1; y >= 3; y--) ConductorBlock.placeConnected(level, abs.apply(new BlockPos(0, y, 0)), coax);
        ConductorBlock.placeConnected(level, abs.apply(ARRESTOR),
                s.arrestor ? RadioAntennaContent.LIGHTNING_ARRESTOR.get().defaultBlockState() : coax);
        if (s.tuner) level.setBlock(abs.apply(TUNER), RadioAmpContent.ANTENNA_TUNER.get().defaultBlockState(), 3);
        else ConductorBlock.placeConnected(level, abs.apply(TUNER), coax);
        ConductorBlock.placeConnected(level, abs.apply(new BlockPos(0, 1, 1)), coax);
        if (s.amp != null) level.setBlock(abs.apply(AMP), s.amp.defaultBlockState(), 3);
        else ConductorBlock.placeConnected(level, abs.apply(AMP), coax);
        level.setBlock(abs.apply(SDR), RadioSdrContent.SDR_STANDARD.get().defaultBlockState().setValue(SdrBlock.FACING, Direction.SOUTH), 3);
        if (s.dressing) {
            level.setBlock(abs.apply(GENERATOR), RadioPowerContent.BURNER_GENERATOR.get().defaultBlockState(), 3);
            if (level.getBlockEntity(abs.apply(GENERATOR)) instanceof BurnerGeneratorBlockEntity g)
                g.fuel().insertItem(0, new ItemStack(Items.COAL, 16), false);
            level.setBlock(abs.apply(CHEST), Blocks.CHEST.defaultBlockState().setValue(ChestBlock.FACING, Direction.SOUTH), 3);
            if (level.getBlockEntity(abs.apply(CHEST)) instanceof ChestBlockEntity c) {
                c.setItem(0, new ItemStack(RadioHazardContent.RF_METER.get()));
                c.setItem(1, new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()));
                c.setItem(2, new ItemStack(Items.COAL, 32));
            }
        }
    }

    // ------------------------------------------------------------ operating helpers

    /** Tunes the SDR to the antenna's resonance and enables transmit at its full 5 W. Returns the frequency. */
    public static double tuneToResonance(ServerLevel level, BlockPos feedAbs, SdrBlockEntity sdr) {
        Antenna a = AntennaManager.get(level, feedAbs);
        double hz = Double.isFinite(a.resonantHz()) ? a.resonantHz() : 7.1e6;
        var radio = sdr.getPeripheral().radio();
        radio.setFrequency(Math.round(hz / 1000.0) * 1000.0);
        radio.setSampleRate(SAMPLE_RATE);
        radio.setTx(true, 37);
        return radio.centerHz();
    }

    /** Transmits {@code seconds} of carrier from the SDR in one write (the airtime is spread over the following ticks). */
    public static long keyDown(SdrBlockEntity sdr, double seconds) {
        int n = (int) Math.round(seconds * SAMPLE_RATE);
        float[] iq = new float[2 * n];
        for (int i = 0; i < n; i++) iq[2 * i] = 1;   // unmodulated carrier, RMS 1 = full power
        return sdr.getPeripheral().radio().write(RadioMediumHooks.medium(), iq, n);
    }

    /** A receive-only endpoint for measuring a station from far away. */
    public static final class Listener implements RadioEndpoint {
        private final UUID id = UUID.randomUUID();
        private final Pose pose;
        private volatile Channel channel;

        public Listener(Pose pose, Channel channel) {
            this.pose = pose;
            this.channel = channel;
        }

        public void tune(Channel c) { channel = c; }
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.ISOTROPIC; }
        @Override public Channel tunedChannel() { return channel; }
        @Override public double maxTxPowerDbm() { return -100; }
        @Override public double sensitivityDbm() { return -200; }
        @Override public boolean listening() { return false; }
        @Override public void onReceive(Reception r) {}

        /** Strongest level heard from {@code from} within {@code micros} of now, dBm (-∞ if nothing). */
        public double heardDbm(RadioMedium medium, RadioEndpoint from, long micros) {
            double[] best = {Double.NEGATIVE_INFINITY};
            long now = medium.nowMicros();
            // Emissions may be scheduled ahead of now (back-to-back after an earlier one): look ahead too.
            medium.forEachHeard(this, channel, now - micros, now + micros, h -> {
                if (h.from().id().equals(from.id())) best[0] = Math.max(best[0], h.rxPowerDbm());
            });
            return best[0];
        }
    }

    // ------------------------------------------------------------ ham_station

    /** Transmit frequency of {@code ham_station}: inside the dipole's 40 m band (the tuner trims the rest of the mismatch). */
    public static final String HAM_FREQ = "7.1M";

    /**
     * {@code ham_station}, built and operated by a player: the antenna, coax,
     * arrestor, tuner, amplifier, SDR and generator are placed by right-clicks
     * (the dipole's feed point last, between the wires), the analyzer and
     * status right-clicks are read from chat, the radio is keyed with
     * {@code tx_tone} on the computer, fuel goes into the generator by
     * right-clicking it with coal, and lightning is {@code /summon}ed.
     * Control 1 (before any fuel): the amplifier has no FE, so it browns out
     * to a 5 W bypass and nothing is drawn. Then coal, 100 W out drawing
     * 40 FE/t only while transmitting. Control 2: lightning on the dipole is
     * grounded by the arrestor (hazards set to "equipment" and lightning
     * damage on with /gamerule, restored afterwards).
     */
    public static Scenario hamStation() {
        long[] fe = {0};
        int[] rules = {-1};
        String[] lightning = {"true"};
        return Scenario.builder("ham_station",
                        "burner generator -> 100 W amplifier -> tuner -> lightning arrestor -> 7 MHz dipole; SDR + computer."
                                + " Controls: no fuel browns out to a 5 W bypass; lightning does nothing (arrestor)")
                .asPlayer()
                .realTime()
                .host("radio", COMPUTER, null)
                .decor(new StationDecor())
                .timeLimit(58_000)
                .note("Built for you: SDR (0,1,3) -> 100 W amp (0,1,2, burner generator beside it, no fuel yet) -> coax -> tuner"
                        + " -> lightning arrestor -> coax up the mast -> feed point of a 2 x 10 block antenna-wire dipole 10 m up."
                        + " The chest has an RF Meter, an Antenna Analyzer and coal.")
                .send("radio", "peripherals")
                .expect("radio", "\\bsdr\\b", "the computer sees the SDR it drives")
                .note("Right-click the feed point with the Antenna Analyzer")
                .await(r -> AntennaScenarios.analyzer(r, FEED), "the analyzer reads the antenna", 30_000)
                .mutate(PowerScenarios::checkAnalyzer, "resonant near 7 MHz, rated above 100 W, feedline with an arrestor")
                .note("Control 1: no fuel in the generator yet. Key 1 s at 5 W and right-click the amplifier while it transmits")
                .mutate(r -> fe[0] = amp(r).feDrawnTotal(), "note the amplifier's FE counter")
                .send("radio", "tx_tone " + HAM_FREQ + " --seconds 1 --power 37")
                .waitMs(600, "mid-transmission")
                .mutate(r -> rightClickAmp(r, "brownout"), "amplifier status: BROWNOUT, 5 W bypass")
                .expect("radio", "^tx_tone: done", "the 1 s transmission ended")
                .mutate(r -> checkBrownout(r, fe[0]), "a 5 W bypass output, nothing drawn, the amplifier intact")
                .note("Fuel: right-click the burner generator with coal; it charges the amplifier")
                .mutate(r -> r.say("§7  generator: " + r.player().use(new ItemStack(Items.COAL, 16), r.abs(GENERATOR), Direction.SOUTH, false)
                        + " " + r.player().useEmpty(r.abs(GENERATOR), Direction.SOUTH, false)), "16 coal in the generator")
                .await(r -> ampCharged(r, 2_000), "the amplifier shows 2000+ FE (right-click it)", 20_000)
                .note("Key down 2 s at 5 W drive: the amplifier raises it to 100 W, drawing 40 FE/t only while transmitting")
                .mutate(r -> fe[0] = amp(r).feDrawnTotal(), "note the amplifier's FE counter")
                .send("radio", "tx_tone " + HAM_FREQ + " --seconds 2 --power 37")
                .waitMs(1_000, "mid-transmission")
                .mutate(r -> rightClickAmp(r, "100 W"), "amplifier status: out 100 W")
                .mutate(r -> r.say("§7  tuner: " + r.player().useEmpty(r.abs(TUNER), Direction.SOUTH, false)), "right-click the tuner")
                .expect("radio", "^tx_tone: done", "the 2 s transmission ended")
                .mutate(r -> checkKeyedDown(r, fe[0]), "~100 W out, ~1600 FE drawn, SWR ~1:1 at the amplifier (tuner)")
                .note("Control 2: lightning strikes the dipole - the arrestor grounds it, nothing is destroyed")
                .mutate(r -> {
                    rules[0] = gamerule(r, "radioHazards", "-1");
                    lightning[0] = r.player().command("/gamerule radioLightningDamage").stream()
                            .map(s -> s.replaceAll(".*: ", "").trim()).findFirst().orElse("true");
                    r.say("§7  " + r.player().command("/gamerule radioHazards 1") + " " + r.player().command("/gamerule radioLightningDamage true"));
                }, "/gamerule radioHazards 1 (equipment) and /gamerule radioLightningDamage true")
                .mutate(PowerScenarios::lightningControl, "/summon lightning_bolt on the east arm; amplifier, tuner and SDR intact")
                .mutate(r -> r.say("§7  " + r.player().command("/gamerule radioHazards " + rules[0]) + " "
                        + r.player().command("/gamerule radioLightningDamage " + lightning[0])), "gamerules put back")
                .build();
    }

    /** Reads an integer gamerule as the player would (/gamerule name), or {@code fallback}. */
    static int gamerule(ScenarioRun r, String rule, String fallback) {
        for (String s : r.player().command("/gamerule " + rule)) {
            java.util.regex.Matcher m = java.util.regex.Pattern.compile("(-?\\d+)\\s*$").matcher(s);
            if (m.find()) return Integer.parseInt(m.group(1));
        }
        return Integer.parseInt(fallback);
    }

    private static SdrBlockEntity sdr(ScenarioRun r) {
        if (!(r.level().getBlockEntity(r.abs(SDR)) instanceof SdrBlockEntity be)) throw new IllegalStateException("no SDR at " + r.abs(SDR));
        return be;
    }

    private static AmplifierBlockEntity amp(ScenarioRun r) {
        if (!(r.level().getBlockEntity(r.abs(AMP)) instanceof AmplifierBlockEntity be)) throw new IllegalStateException("no amplifier at " + r.abs(AMP));
        return be;
    }

    /** Right-click the amplifier with an empty hand; its status lines must mention {@code expect}. */
    private static void rightClickAmp(ScenarioRun r, String expect) {
        List<String> said = r.player().useEmpty(r.abs(AMP), Direction.SOUTH, false);
        EvansComputerMod.LOGGER.info("[ham_station] amplifier right-click: {}", String.join(" | ", said));
        r.say("§7  amplifier: " + String.join(" | ", said));
        boolean ok = expect.equals("brownout") ? said.stream().anyMatch(s -> s.contains("BROWNOUT"))
                : said.stream().anyMatch(s -> s.matches(".*out (9[5-9]|100)(\\.\\d)? W.*"));
        if (!ok) r.fail("amplifier status doesn't show " + expect + ": " + said);
    }

    /** Null once a right-click on the amplifier shows at least {@code fe} FE stored ("n / max FE"). */
    private static String ampCharged(ScenarioRun r, int fe) {
        if (r.level().getGameTime() % 20 != 0) return "waiting";
        List<String> said = r.player().useEmpty(r.abs(AMP), Direction.SOUTH, false);
        for (String s : said) {
            java.util.regex.Matcher m = java.util.regex.Pattern.compile("(\\d+) / (\\d+) FE").matcher(s);
            if (m.find() && Integer.parseInt(m.group(1)) >= fe) {
                r.say("§7  amplifier: " + s);
                return null;
            }
        }
        return "amplifier says " + said;
    }

    private static void checkAnalyzer(ScenarioRun r) {
        double f = AntennaScenarios.resonanceHz(r);
        if (!(f > 6.5e6 && f < 7.5e6)) {
            r.fail("analyzer: resonance " + f + " Hz, expected near 7 MHz: " + AntennaScenarios.READING.get(r));
            return;
        }
        String said = String.join(" | ", r.player().heard());
        if (!said.contains("lightning arrestor fitted")) r.fail("analyzer doesn't see the arrestor on the feedline: " + said);
        java.util.regex.Matcher m = java.util.regex.Pattern.compile("rated (\\d+(?:\\.\\d+)?) (k?)W").matcher(said);
        if (!m.find() || Double.parseDouble(m.group(1)) * (m.group(2).isEmpty() ? 1 : 1000) <= 100)
            r.fail("antenna not rated above 100 W: " + said);
    }

    private static void checkKeyedDown(ScenarioRun r, long feBefore) {
        AmplifierBlockEntity amp = amp(r);
        var b = amp.lastBudget();
        long drawn = amp.feDrawnTotal() - feBefore;
        ExciterLink link = sdr(r).link();
        if (!link.connected() || link.chain().feed() == null || !link.chain().feed().equals(r.abs(FEED)))
            r.fail("SDR did not find the chain to the feed point: " + link.chain());
        else if (link.chain().ampIndex() < 0 || link.chain().tunerIndex() < 0 || !link.chain().hasArrestor())
            r.fail("chain is missing a part: " + link.chain().hops());
        if (b == null || b.amp() == null) { r.fail("the amplifier never keyed up"); return; }
        if (b.amp().outW() < 95 || b.amp().outW() > 100.001) { r.fail("output " + b.amp().outW() + " W, expected 100 W"); return; }
        if (b.swrAtAmp() > 1.05) { r.fail("tuner left SWR " + b.swrAtAmp() + " at the amplifier"); return; }
        // 2 s x 20 ticks x 40 FE/t = 1600 FE.
        if (drawn < 1500 || drawn > 1700) { r.fail("FE drawn " + drawn + ", expected ~1600"); return; }
        if (b.acceptedW() < 80) r.fail("antenna accepts only " + b.acceptedW() + " W");
        EvansComputerMod.LOGGER.info(String.format(Locale.ROOT, "[ham_station] out %.1f W, antenna %.1f W, tuner %.1f W, drawn %d FE",
                b.amp().outW(), b.acceptedW(), b.tunerHeatW(), drawn));
    }

    private static void lightningControl(ScenarioRun r) {
        BlockPos at = r.abs(FEED.east(ARM - 1).above());
        List<String> said = r.player().command(String.format(Locale.ROOT, "/summon minecraft:lightning_bolt %d %d %d", at.getX(), at.getY(), at.getZ()));
        r.say("§7  " + said);
        List<String> status = r.player().useEmpty(r.abs(AMP), Direction.SOUTH, false);
        EvansComputerMod.LOGGER.info("[ham_station] after the strike, amplifier says: {}", status);
        r.say("§7  amplifier: " + String.join(" | ", status));
        ServerLevel level = r.level();
        boolean intact = level.getBlockEntity(r.abs(AMP)) instanceof AmplifierBlockEntity
                && level.getBlockEntity(r.abs(TUNER)) instanceof TunerBlockEntity && level.getBlockEntity(r.abs(SDR)) instanceof SdrBlockEntity;
        if (!intact) r.fail("lightning destroyed the station despite the arrestor: " + status);
        else if (status.stream().noneMatch(w -> w.contains("grounded by the arrestor"))) r.fail("the strike didn't reach the antenna: " + status);
    }

    private static void checkBrownout(ScenarioRun r, long feBefore) {
        AmplifierBlockEntity amp = amp(r);
        var b = amp.lastBudget();
        if (b == null || b.amp() == null) { r.fail("no transmission through the amplifier"); return; }
        if (Math.abs(b.amp().outW() - b.driveW()) > 1e-6 || b.driveW() > 5.02 || b.amp().supply() > 0) { r.fail("brownout output " + b.amp().outW() + " W at supply " + b.amp().supply()); return; }
        if (amp.feDrawnTotal() != feBefore) r.fail("drew " + (amp.feDrawnTotal() - feBefore) + " FE with an empty buffer");
    }

    /** Field, posts and chest as terrain; everything electrical placed by the player. */
    private static final class StationDecor implements Scenario.Decor {
        @Override
        public List<BlockPos> footprint() {
            return PowerScenarios.footprint(new Station(RadioAntennaContent.ANTENNA_WIRE.get(), null, true, true, true));
        }

        @Override
        public void terrain(ScenarioRun run) {
            List<BlockPos> field = new ArrayList<>(), posts = new ArrayList<>();
            for (int x = -12; x <= 12; x++) for (int z = -2; z <= 5; z++) field.add(new BlockPos(x, 0, z));
            for (int y = 1; y < FEED.getY(); y++) { posts.add(new BlockPos(-ARM - 1, y, 0)); posts.add(new BlockPos(ARM + 1, y, 0)); }
            PlayerKit.fill(run, field, Blocks.GRASS_BLOCK.defaultBlockState());
            PlayerKit.fill(run, posts, Blocks.SPRUCE_FENCE.defaultBlockState());
            PlayerKit.chest(run, CHEST, Direction.SOUTH, new ItemStack(RadioHazardContent.RF_METER.get()),
                    new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()), new ItemStack(Items.COAL, 32));
        }

        @Override
        public void build(ScenarioRun run) {
            ScenarioPlayer p = run.player();
            Block wire = RadioAntennaContent.ANTENNA_WIRE.get(), coax = RadioAntennaContent.COAX_CABLE.get();
            p.place(RadioAntennaContent.INSULATOR.get(), run.abs(FEED.west(ARM + 1)), Direction.SOUTH);
            p.place(RadioAntennaContent.INSULATOR.get(), run.abs(FEED.east(ARM + 1)), Direction.SOUTH);
            List<BlockPos> west = new ArrayList<>(), east = new ArrayList<>();
            for (int i = ARM; i >= 1; i--) { west.add(FEED.west(i)); east.add(FEED.east(i)); }
            PlayerKit.placeRun(run, wire, west, Direction.SOUTH);
            PlayerKit.placeRun(run, wire, east, Direction.SOUTH);
            // The mast from the ground up: tuner, arrestor, coax, and the feed point on top, between the wires.
            p.place(RadioAmpContent.ANTENNA_TUNER.get(), run.abs(TUNER), Direction.SOUTH);
            p.placeAgainst(RadioAntennaContent.LIGHTNING_ARRESTOR.get(), run.abs(ARRESTOR), Direction.SOUTH, Direction.DOWN, null);
            for (int y = 3; y < FEED.getY(); y++)
                p.placeAgainst(coax, run.abs(new BlockPos(0, y, 0)), Direction.SOUTH, Direction.DOWN, null);
            p.placeAgainst(RadioAntennaContent.FEED_POINT.get(), run.abs(FEED), Direction.SOUTH, Direction.DOWN,
                    s -> s.getValue(FeedPointBlock.AXIS) == Direction.Axis.X);
            p.place(coax, run.abs(new BlockPos(0, 1, 1)), Direction.SOUTH);
            p.place(RadioAmpContent.AMPLIFIER_100W.get(), run.abs(AMP), Direction.SOUTH);
            p.place(RadioSdrContent.SDR_STANDARD.get(), run.abs(SDR), Direction.SOUTH, s -> s.getValue(SdrBlock.FACING) == Direction.SOUTH);
            p.place(RadioPowerContent.BURNER_GENERATOR.get(), run.abs(GENERATOR), Direction.SOUTH);
        }

        @Override
        public void clear(ScenarioRun run) {
            PlayerKit.emptyChest(run, CHEST);
            if (run.level().getBlockEntity(run.abs(GENERATOR)) instanceof BurnerGeneratorBlockEntity g) g.fuel().extractItem(0, 64, false);
        }
    }
}
//?}
