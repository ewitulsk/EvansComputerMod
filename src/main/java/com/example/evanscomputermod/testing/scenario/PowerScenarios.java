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
 * with no FE the amplifier browns out to a 5 W bypass without damage.
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

    public static Scenario hamStation() {
        Station s = new Station(RadioAntennaContent.ANTENNA_WIRE.get(), RadioAmpContent.AMPLIFIER_100W.get(), true, true, true);
        Long[] fe = {0L};
        long[] baseline = {0};
        return Scenario.builder("ham_station",
                        "burner generator -> 100 W amplifier -> tuner -> lightning arrestor -> 7 MHz dipole; SDR + computer."
                                + " Controls: lightning does nothing (arrestor); no FE browns out to a 5 W bypass")
                .host("radio", COMPUTER, null)
                .decor(new StationDecor(s))
                .timeLimit(55_000)
                .note("SDR (0,1,3) -> 100 W amp (0,1,2, fed by the burner generator) -> coax -> tuner -> lightning arrestor"
                        + " -> coax up the mast -> feed point of a 2 x 10 block antenna-wire dipole 10 m up")
                .send("radio", "peripherals")
                .expect("radio", "\\bsdr\\b", "the computer sees the SDR it drives")
                .waitMs(2_000, "let the generator charge the amplifier")
                .mutate(r -> analyzeAndTune(r), "antenna analyzer on the feed point; tune the SDR to resonance, transmit on (5 W exciter)")
                .mutate(r -> {
                    fe[0] = amp(r).feDrawnTotal();
                    keyDown(sdr(r), 2.0);
                }, "key down 2 s: the amplifier raises 5 W to 100 W and draws 40 FE/t only while transmitting")
                .waitMs(3_000, "the 2 s transmission runs (right-click the amplifier and tuner for their status)")
                .mutate(r -> checkKeyedDown(r, fe[0]), "expect ~100 W out, ~1600 FE drawn, SWR ~1:1 at the amplifier (tuner), antenna rated above 100 W")
                .note("Control 1: lightning strikes the dipole - the arrestor grounds it, nothing is destroyed")
                .mutate(PowerScenarios::lightningControl, "strike the east arm; expect amplifier, tuner and SDR intact")
                .note("Control 2: no fuel, no FE - the amplifier browns out to a 5 W bypass, no damage")
                .mutate(r -> {
                    if (r.level().getBlockEntity(r.abs(GENERATOR)) instanceof BurnerGeneratorBlockEntity g) g.fuel().extractItem(0, 64, false);
                    r.level().removeBlock(r.abs(GENERATOR), false);
                    amp(r).energy().setEnergy(0);
                    baseline[0] = amp(r).feDrawnTotal();
                }, "remove the generator and drain the amplifier")
                .waitMs(500, "the amplifier notices it has no supply")
                .mutate(r -> keyDown(sdr(r), 0.5), "key down 0.5 s with no FE")
                .waitMs(1_500, "the transmission runs")
                .mutate(r -> checkBrownout(r, baseline[0]), "expect a 5 W bypass output, nothing drawn, the amplifier intact")
                .build();
    }

    private static SdrBlockEntity sdr(ScenarioRun r) {
        if (!(r.level().getBlockEntity(r.abs(SDR)) instanceof SdrBlockEntity be)) throw new IllegalStateException("no SDR at " + r.abs(SDR));
        return be;
    }

    private static AmplifierBlockEntity amp(ScenarioRun r) {
        if (!(r.level().getBlockEntity(r.abs(AMP)) instanceof AmplifierBlockEntity be)) throw new IllegalStateException("no amplifier at " + r.abs(AMP));
        return be;
    }

    private static void analyzeAndTune(ScenarioRun r) {
        Antenna a = AntennaManager.solveNow(r.level(), r.abs(FEED), 30_000);
        List<String> lines = AntennaAnalyzerItem.reportLines(r.level(), a);
        EvansComputerMod.LOGGER.info("[ham_station] analyzer: {}", String.join(" | ", lines));
        if (!a.solved()) { r.fail("antenna not solved: " + a.summary()); return; }
        if (!(a.powerLimitW() > 100)) { r.fail("antenna rated only " + a.powerLimitW() + " W for a 100 W station"); return; }
        double hz = tuneToResonance(r.level(), r.abs(FEED), sdr(r));
        ExciterLink link = sdr(r).link();
        if (!link.connected() || link.chain().feed() == null || !link.chain().feed().equals(r.abs(FEED)))
            r.fail("SDR did not find the chain to the feed point: " + link.chain());
        else if (link.chain().ampIndex() < 0 || link.chain().tunerIndex() < 0 || !link.chain().hasArrestor())
            r.fail("chain is missing a part: " + link.chain().hops());
        else if (amp(r).energy().getEnergyStored() < 100) r.fail("generator didn't charge the amplifier: " + amp(r).energy().getEnergyStored() + " FE");
        EvansComputerMod.LOGGER.info("[ham_station] tuned to {} Hz, amplifier {} FE", (long) hz, amp(r).energy().getEnergyStored());
    }

    private static void checkKeyedDown(ScenarioRun r, long feBefore) {
        AmplifierBlockEntity amp = amp(r);
        var b = amp.lastBudget();
        long drawn = amp.feDrawnTotal() - feBefore;
        List<String> status = amp.statusLines(sdr(r).link().warnings());
        EvansComputerMod.LOGGER.info("[ham_station] amplifier: {}", String.join(" | ", status));
        if (r.level().getBlockEntity(r.abs(TUNER)) instanceof TunerBlockEntity t)
            EvansComputerMod.LOGGER.info("[ham_station] {}", t.statusLine());
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
        ServerLevel level = r.level();
        LightningBolt bolt = EntityType.LIGHTNING_BOLT.create(level);
        if (bolt == null) { r.fail("no lightning bolt"); return; }
        bolt.moveTo(Vec3.atBottomCenterOf(r.abs(FEED.east(ARM - 1).above())));
        // Hazards at equipment level with lightning damage on, for this station only, so the control means something.
        com.example.evanscomputermod.radio.hazard.RadioGameRules.override(level, r.abs(FEED), 30,
                com.example.evanscomputermod.radio.RadioConfig.HazardLevel.EQUIPMENT, true);
        level.addFreshEntity(bolt);   // LightningHazard handles the strike as the bolt joins the level
        com.example.evanscomputermod.radio.hazard.RadioGameRules.clearOverride(r.abs(FEED));
        List<String> warnings = sdr(r).link().warnings();
        EvansComputerMod.LOGGER.info("[ham_station] after the strike: {}", warnings);
        boolean intact = level.getBlockEntity(r.abs(AMP)) instanceof AmplifierBlockEntity
                && level.getBlockEntity(r.abs(TUNER)) instanceof TunerBlockEntity && level.getBlockEntity(r.abs(SDR)) instanceof SdrBlockEntity;
        if (!intact) r.fail("lightning destroyed the station despite the arrestor: " + warnings);
        else if (warnings.stream().noneMatch(w -> w.contains("grounded by the arrestor"))) r.fail("the strike didn't reach the antenna: " + warnings);
    }

    private static void checkBrownout(ScenarioRun r, long feBefore) {
        AmplifierBlockEntity amp = amp(r);
        var b = amp.lastBudget();
        EvansComputerMod.LOGGER.info("[ham_station] no FE: {}", String.join(" | ", amp.statusLines(null)));
        if (b == null || b.amp() == null) { r.fail("no transmission through the amplifier"); return; }
        if (Math.abs(b.amp().outW() - b.driveW()) > 1e-6 || b.driveW() > 5.02 || b.amp().supply() > 0) { r.fail("brownout output " + b.amp().outW() + " W at supply " + b.amp().supply()); return; }
        if (amp.feDrawnTotal() != feBefore) r.fail("drew " + (amp.feDrawnTotal() - feBefore) + " FE with an empty buffer");
    }

    private static final class StationDecor implements Scenario.Decor {
        final Station station;

        StationDecor(Station s) {
            station = s;
        }

        @Override
        public List<BlockPos> footprint() {
            return PowerScenarios.footprint(station);
        }

        @Override
        public void build(ScenarioRun run) {
            PowerScenarios.build(run.level(), run::abs, station);
        }

        @Override
        public void clear(ScenarioRun run) {
            if (run.level().getBlockEntity(run.abs(CHEST)) instanceof ChestBlockEntity c) c.clearContent();
            if (run.level().getBlockEntity(run.abs(GENERATOR)) instanceof BurnerGeneratorBlockEntity g) g.fuel().extractItem(0, 64, false);
        }
    }
}
//?}
