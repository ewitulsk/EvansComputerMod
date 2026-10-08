package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.amp.AmplifierBlockEntity;
import com.example.evanscomputermod.radio.amp.ChainBudget;
import com.example.evanscomputermod.radio.amp.RadioAmpContent;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.hazard.RadioGameRules;
import com.example.evanscomputermod.radio.hazard.RadioHazardContent;
import com.example.evanscomputermod.radio.hazard.ThermalModel;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.sdr.SdrBlockEntity;
import com.example.evanscomputermod.testing.scenario.PowerScenarios;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LightningBolt;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.BooleanSupplier;

/**
 * Lanes 4D/5C in the world, namespace {@code ecm_radio}: an SDR driving an
 * amplifier into a dipole through coax (far-away level with and without FE),
 * FE drawn only while transmitting, a thin copper dipole melting at 1 kW after
 * the thermal model's exact time while heavy cable survives (control), hazard
 * level off (warning only), a cancelled HazardEvent, lightning with and
 * without an arrestor, and the {@code ham_station} scenario.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioPowerTests {
    private static final String NS = RadioTests.NS;
    private static final String STRUCTURE = RadioTests.STRUCTURE;
    /** Station layout origin inside the structure. */
    private static final BlockPos ORIGIN = new BlockPos(20, 1, 17);

    private static final List<HazardEvent> EVENTS = new CopyOnWriteArrayList<>();
    private static volatile BlockPos cancelNear;
    private static volatile boolean listening;

    private RadioPowerTests() {}

    // ------------------------------------------------------------ helpers

    private static void listen() {
        if (listening) return;
        listening = true;
        NeoForge.EVENT_BUS.addListener((HazardEvent e) -> {
            EVENTS.add(e);
            BlockPos c = cancelNear;
            if (c != null && e.pos().closerThan(c, 16)) e.setCanceled(true);
        });
    }

    private static BlockPos abs(GameTestHelper h, BlockPos layout) {
        return h.absolutePos(ORIGIN.offset(layout));
    }

    /** Sets the hazard level (and lightning toggle) for this test's station only: tests share one world. */
    private static void hazards(GameTestHelper h, int level, boolean lightning) {
        RadioGameRules.override(h.getLevel(), abs(h, BlockPos.ZERO), 20, RadioConfig.HazardLevel.values()[level], lightning);
    }

    private static void clearHazards(GameTestHelper h) {
        RadioGameRules.clearOverride(abs(h, BlockPos.ZERO));
    }

    private static void build(GameTestHelper h, PowerScenarios.Station s) {
        PowerScenarios.build(h.getLevel(), p -> abs(h, p), s);
    }

    private static SdrBlockEntity sdr(GameTestHelper h) {
        return (SdrBlockEntity) h.getLevel().getBlockEntity(abs(h, PowerScenarios.SDR));
    }

    private static AmplifierBlockEntity amp(GameTestHelper h) {
        return h.getLevel().getBlockEntity(abs(h, PowerScenarios.AMP)) instanceof AmplifierBlockEntity a ? a : null;
    }

    private static Antenna antenna(GameTestHelper h) {
        return AntennaManager.get(h.getLevel(), abs(h, PowerScenarios.FEED));
    }

    /** The station is built, solved and the SDR has found its chain to the feed point. */
    private static boolean ready(GameTestHelper h) {
        SdrBlockEntity s = sdr(h);
        return s != null && antenna(h).solved() && s.link().connected() && s.link().antenna() != null && s.link().antenna().solved() && abs(h, PowerScenarios.FEED).equals(s.link().chain().feed());
    }

    private static void check(boolean ok, String[] failure, String why) {
        if (!ok && failure[0] == null) failure[0] = why;
    }

    private static int armBlocksLeft(GameTestHelper h, Block wire) {
        int n = 0;
        for (int i = 1; i <= PowerScenarios.ARM; i++) {
            if (h.getLevel().getBlockState(abs(h, PowerScenarios.FEED.west(i))).is(wire)) n++;
            if (h.getLevel().getBlockState(abs(h, PowerScenarios.FEED.east(i))).is(wire)) n++;
        }
        return n;
    }

    // ------------------------------------------------------------ amplifier

    /**
     * SDR (5 W) → 100 W amplifier → coax → antenna-wire dipole, heard by a
     * receiver 200 blocks overhead (broadside, in loaded chunks). With no FE the amplifier browns out to a
     * bypass (the exciter's 5 W, nothing damaged, nothing drawn); powered, the
     * same receiver hears it about 13 dB louder. The SDR radiates from the
     * dipole's feed point with the dipole's pattern, not its whip.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".amp_level")
    public static void amplifier_raises_far_level_and_browns_out(GameTestHelper h) {
        String[] failure = {null};
        hazards(h, 0, false);
        build(h, new PowerScenarios.Station(RadioAntennaContent.ANTENNA_WIRE.get(), RadioAmpContent.AMPLIFIER_100W.get(), false, true, false));
        RadioMedium medium = RadioMediumHooks.medium();
        PowerScenarios.Listener[] rx = {null};
        double[] hz = {0}, bypass = {0};
        int[] waited = {0};
        RadioAntennaTests.steps(h, "amplifier_raises_far_level_and_browns_out", failure, List.of(
                () -> ready(h),
                () -> {
                    hz[0] = PowerScenarios.tuneToResonance(h.getLevel(), abs(h, PowerScenarios.FEED), sdr(h));
                    BlockPos f = abs(h, PowerScenarios.FEED);
                    rx[0] = new PowerScenarios.Listener(Pose.at(h.getLevel().dimension().location().toString(), f.getX() + 0.5, f.getY() + 200.5, f.getZ() + 0.5),
                            sdr(h).getPeripheral().radio().channel());
                    medium.register(rx[0]);
                    Pose p = sdr(h).endpoint().pose();
                    check(Math.abs(p.x() - (f.getX() + 0.5)) < 0.01 && Math.abs(p.y() - (f.getY() + 0.5)) < 0.01, failure,
                            "SDR radiates from " + p + ", not the feed point " + f);
                    double peak = sdr(h).endpoint().antenna().peakGainDbi();
                    check(peak > 3, failure, "endpoint pattern peak " + peak + " dBi: still the whip?");
                    return true;
                },
                () -> !Double.isNaN(medium.pathGainDb(sdr(h).endpoint(), rx[0], hz[0])) || ++waited[0] > 200,
                () -> {
                    AmplifierBlockEntity a = amp(h);
                    check(a != null && a.energy().getEnergyStored() == 0 && a.supply() == 0, failure, "amplifier should start unpowered");
                    PowerScenarios.keyDown(sdr(h), 0.2);
                    bypass[0] = rx[0].heardDbm(medium, sdr(h).endpoint(), 1_000_000);
                    ChainBudget b = a.lastBudget();
                    check(b == null || b.amp() == null || Math.abs(b.amp().outW() - b.driveW()) < 1e-6, failure, "no-FE output " + (b == null ? null : b.amp()));
                    return true;
                },
                () -> {
                    AmplifierBlockEntity a = amp(h);
                    ChainBudget b = sdr(h).link().lastBudget();
                    check(b != null && b.amp() != null && Math.abs(b.forwardW() - b.driveW()) < 1e-6 && b.amp().supply() == 0, failure,
                            "brownout should bypass the 5 W exciter: " + (b == null ? null : b.amp()));
                    check(a.feDrawnTotal() == 0 && !a.isRemoved(), failure, "brownout drew FE or damaged the amplifier");
                    a.energy().setEnergy(a.energy().getMaxEnergyStored());
                    return true;
                },
                () -> amp(h).supply() >= 1,
                () -> {
                    PowerScenarios.keyDown(sdr(h), 0.2);
                    double powered = rx[0].heardDbm(medium, sdr(h).endpoint(), 1_000_000);
                    ChainBudget b = sdr(h).link().lastBudget();
                    EvansComputerMod.LOGGER.info(String.format(Locale.ROOT,
                            "[ecm_radio] amp level: bypass %.1f dBm, powered %.1f dBm 200 m overhead; out %.1f W, antenna %.1f W, SWR %.2f; path gain %.1f dB, tx pattern %.1f dBi peak, feed loss %.2f dB",
                            bypass[0], powered, b.forwardW(), b.acceptedW(), b.swrAtAmp(), medium.pathGainDb(sdr(h).endpoint(), rx[0], hz[0]),
                            sdr(h).endpoint().antenna().peakGainDbi(), sdr(h).endpoint().antenna().feedLossDb()));
                    check(Double.isFinite(bypass[0]) && Double.isFinite(powered), failure, "receiver heard nothing: " + bypass[0] + " / " + powered);
                    check(Math.abs(b.amp().outW() - 100) < 1, failure, "amplifier output " + b.amp().outW() + " W");
                    check(powered - bypass[0] > 12 && powered - bypass[0] < 14, failure,
                            "amplified level only " + (powered - bypass[0]) + " dB above the bypass (expect 13 dB)");
                    medium.unregister(rx[0]);
                    return true;
                }));
    }

    /**
     * FE flows only while transmitting: a full 100 W amplifier keeps its
     * charge while idle, draws 2 x 100 W / 5 W per FE/t = 40 FE per tick of
     * airtime (800 FE for a 1 s transmission), then stops.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".amp_fe")
    public static void amplifier_draws_fe_only_while_transmitting(GameTestHelper h) {
        String[] failure = {null};
        hazards(h, 0, false);
        build(h, new PowerScenarios.Station(RadioAntennaContent.ANTENNA_WIRE.get(), RadioAmpContent.AMPLIFIER_100W.get(), false, true, false));
        long[] mark = {0};
        int[] t = {0};
        RadioAntennaTests.steps(h, "amplifier_draws_fe_only_while_transmitting", failure, List.of(
                () -> ready(h),
                () -> {
                    amp(h).energy().setEnergy(amp(h).energy().getMaxEnergyStored());
                    PowerScenarios.tuneToResonance(h.getLevel(), abs(h, PowerScenarios.FEED), sdr(h));
                    return true;
                },
                () -> ++t[0] >= 20,
                () -> {
                    AmplifierBlockEntity a = amp(h);
                    check(a.feDrawnTotal() == 0 && a.energy().getEnergyStored() == a.energy().getMaxEnergyStored(), failure,
                            "idle amplifier drew " + a.feDrawnTotal() + " FE");
                    PowerScenarios.keyDown(sdr(h), 1.0);
                    t[0] = 0;
                    return true;
                },
                () -> ++t[0] >= 30,
                () -> {
                    AmplifierBlockEntity a = amp(h);
                    long drawn = a.feDrawnTotal();
                    EvansComputerMod.LOGGER.info("[ecm_radio] 1 s at 100 W drew {} FE; {}", drawn, a.statusLines(null));
                    check(drawn >= 790 && drawn <= 810, failure, "1 s at 100 W drew " + drawn + " FE, expected 800");
                    check(a.bursts() == 1, failure, "bursts " + a.bursts());
                    mark[0] = drawn;
                    t[0] = 0;
                    return true;
                },
                () -> ++t[0] >= 20,
                () -> {
                    check(amp(h).feDrawnTotal() == mark[0], failure, "kept drawing after the transmission: " + (amp(h).feDrawnTotal() - mark[0]));
                    return true;
                }));
    }

    // ------------------------------------------------------------ thermal hazards

    /** Keys a 1 kW station for one tick per test tick until {@code until} holds; records the ticks and the load. */
    private static final class Keyer {
        int ticks;
        double load = Double.NaN;
        long expected = -1;

        boolean tick(GameTestHelper h) {
            PowerScenarios.keyDown(sdr(h), 0.05);
            ticks++;
            ChainBudget b = sdr(h).link().lastBudget();
            if (expected < 0 && b != null && b.acceptedW() > 0) {
                load = b.acceptedW() / antenna(h).wireLimitW();
                expected = ThermalModel.Part.WIRE.ticksToFailure(0, load);
                EvansComputerMod.LOGGER.info(String.format(Locale.ROOT,
                        "[ecm_radio] 1 kW station: out %.0f W, antenna takes %.0f W, wire limit %.1f W -> load %.2f, %s to fail (%d ticks); %s",
                        b.forwardW(), b.acceptedW(), antenna(h).wireLimitW(), load,
                        ThermalModel.describeSeconds(ThermalModel.Part.WIRE.timeToFailure(0, load)), expected, antenna(h).summary()));
            }
            return true;
        }
    }

    private static List<BooleanSupplier> kilowatt(GameTestHelper h, Block wire, int hazardLevel) {
        return List.of(
                () -> ready(h),
                () -> {
                    hazards(h, hazardLevel, false);
                    amp(h).energy().setEnergy(amp(h).energy().getMaxEnergyStored());
                    PowerScenarios.tuneToResonance(h.getLevel(), abs(h, PowerScenarios.FEED), sdr(h));
                    return true;
                },
                () -> amp(h).supply() >= 1);
    }

    /**
     * A thin copper-wire dipole fed 1 kW (hazard level equipment) melts after
     * exactly the thermal model's time for its load: the hottest segment (next
     * to the feed) goes, drops melted scrap, and a HazardEvent MELT fires.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".melt")
    public static void thin_copper_dipole_melts_at_1kw(GameTestHelper h) {
        String[] failure = {null};
        listen();
        Block wire = RadioAntennaContent.COPPER_WIRE.get();
        build(h, new PowerScenarios.Station(wire, RadioAmpContent.AMPLIFIER_1KW.get(), false, true, false));
        Keyer k = new Keyer();
        int[] eventsBefore = {0};
        List<BooleanSupplier> steps = new ArrayList<>(kilowatt(h, wire, 1));
        steps.add(() -> {
            eventsBefore[0] = EVENTS.size();
            return true;
        });
        steps.add(() -> {
            if (armBlocksLeft(h, wire) == 2 * PowerScenarios.ARM) {
                k.tick(h);
                if (k.ticks > 1200) failure[0] = "no melt after " + k.ticks + " ticks (expected " + k.expected + ")";
                return false;
            }
            return true;
        });
        steps.add(() -> {
            EvansComputerMod.LOGGER.info("[ecm_radio] copper melted after {} ticks, model {} ticks; {}", k.ticks, k.expected,
                    sdr(h).link().hazards().lastEvent());
            check(k.load > 1 && k.expected > 5, failure, "load " + k.load + " expected " + k.expected);
            check(k.ticks >= k.expected - 1 && k.ticks <= k.expected + 3, failure,
                    "melted after " + k.ticks + " ticks, thermal model says " + k.expected);
            check(armBlocksLeft(h, wire) == 2 * PowerScenarios.ARM - 1, failure, "more than one block melted: " + armBlocksLeft(h, wire));
            BlockPos feed = abs(h, PowerScenarios.FEED);
            boolean nextToFeed = !h.getLevel().getBlockState(feed.west()).is(wire) || !h.getLevel().getBlockState(feed.east()).is(wire);
            check(nextToFeed, failure, "the melted segment isn't the hottest one next to the feed");
            var scrap = h.getLevel().getEntitiesOfClass(ItemEntity.class, new AABB(feed).inflate(4),
                    e -> e.getItem().is(RadioHazardContent.MELTED_SCRAP.get()));
            check(!scrap.isEmpty(), failure, "no melted scrap dropped");
            check(EVENTS.subList(eventsBefore[0], EVENTS.size()).stream().anyMatch(e -> e.kind() == HazardEvent.Kind.MELT
                    && e.pos().closerThan(feed, 3)), failure, "no HazardEvent MELT");
            return true;
        });
        RadioAntennaTests.steps(h, "thin_copper_dipole_melts_at_1kw", failure, steps);
    }

    /** Control: a heavy-cable dipole (rated ~2 kW) at the same 1 kW for 3 s never reaches the failure point. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".nomelt")
    public static void heavy_cable_dipole_survives_1kw(GameTestHelper h) {
        String[] failure = {null};
        Block wire = RadioAntennaContent.HEAVY_CABLE.get();
        build(h, new PowerScenarios.Station(wire, RadioAmpContent.AMPLIFIER_1KW.get(), false, true, false));
        Keyer k = new Keyer();
        List<BooleanSupplier> steps = new ArrayList<>(kilowatt(h, wire, 1));
        steps.add(() -> {
            k.tick(h);
            return k.ticks >= 60;
        });
        steps.add(() -> {
            double theta = sdr(h).link().hazards().wireTheta();
            EvansComputerMod.LOGGER.info("[ecm_radio] heavy cable at 1 kW: load {}, theta {} after {} ticks", k.load, theta, k.ticks);
            check(k.load < 1, failure, "heavy cable load " + k.load + " at 1 kW");
            check(armBlocksLeft(h, wire) == 2 * PowerScenarios.ARM, failure, "heavy cable melted");
            check(theta < 1 && theta > 0, failure, "heavy cable theta " + theta);
            return true;
        });
        RadioAntennaTests.steps(h, "heavy_cable_dipole_survives_1kw", failure, steps);
    }

    /** Hazard level off: the copper dipole at 1 kW passes its failure point but only warns; nothing breaks. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".hazards_off")
    public static void hazards_off_only_warn(GameTestHelper h) {
        String[] failure = {null};
        Block wire = RadioAntennaContent.COPPER_WIRE.get();
        build(h, new PowerScenarios.Station(wire, RadioAmpContent.AMPLIFIER_1KW.get(), false, true, false));
        Keyer k = new Keyer();
        List<BooleanSupplier> steps = new ArrayList<>(kilowatt(h, wire, 0));
        steps.add(() -> {
            k.tick(h);
            return k.expected > 0 && k.ticks >= k.expected + 20;
        });
        steps.add(() -> {
            var hz = sdr(h).link().hazards();
            EvansComputerMod.LOGGER.info("[ecm_radio] hazards off: {} | {}", hz.status(), hz.lastEvent());
            check(armBlocksLeft(h, wire) == 2 * PowerScenarios.ARM, failure, "a wire broke with hazards off");
            check(hz.wireTheta() >= 1 && !hz.status().isEmpty(), failure, "no warning: theta " + hz.wireTheta() + " " + hz.status());
            check(hz.lastEvent().contains("would melt"), failure, "last event: " + hz.lastEvent());
            clearHazards(h);
            // The gamerule itself (set and restored within one tick; the override above keeps parallel tests apart).
            RadioGameRules.set(h.getLevel(), 0);
            boolean off = RadioGameRules.level(h.getLevel()) == RadioConfig.HazardLevel.OFF && !RadioGameRules.lightningDamage(h.getLevel());
            RadioGameRules.set(h.getLevel(), -1);
            check(off && RadioGameRules.level(h.getLevel()) == RadioConfig.hazardDefault(), failure, "radioHazards gamerule not read");
            return true;
        });
        RadioAntennaTests.steps(h, "hazards_off_only_warn", failure, steps);
    }

    /** A HazardEvent listener cancelling the melt (a claim mod, say) keeps the wire; the warning stays. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".hazard_cancel")
    public static void cancelled_hazard_event_prevents_melt(GameTestHelper h) {
        String[] failure = {null};
        listen();
        Block wire = RadioAntennaContent.COPPER_WIRE.get();
        build(h, new PowerScenarios.Station(wire, RadioAmpContent.AMPLIFIER_1KW.get(), false, true, false));
        Keyer k = new Keyer();
        int[] before = {0};
        List<BooleanSupplier> steps = new ArrayList<>(kilowatt(h, wire, 1));
        steps.add(() -> {
            cancelNear = abs(h, PowerScenarios.FEED);
            before[0] = EVENTS.size();
            return true;
        });
        steps.add(() -> {
            k.tick(h);
            return k.expected > 0 && k.ticks >= k.expected + 20;
        });
        steps.add(() -> {
            cancelNear = null;
            var hz = sdr(h).link().hazards();
            long cancelled = EVENTS.subList(before[0], EVENTS.size()).stream().filter(e -> e.kind() == HazardEvent.Kind.MELT && e.isCanceled()).count();
            EvansComputerMod.LOGGER.info("[ecm_radio] cancelled melts: {} | {}", cancelled, hz.lastEvent());
            check(cancelled >= 1, failure, "no MELT event was posted");
            check(armBlocksLeft(h, wire) == 2 * PowerScenarios.ARM, failure, "wire melted despite the cancelled event");
            check(hz.lastEvent().contains("prevented"), failure, "last event: " + hz.lastEvent());
            clearHazards(h);
            return true;
        });
        RadioAntennaTests.steps(h, "cancelled_hazard_event_prevents_melt", failure, steps);
    }

    // ------------------------------------------------------------ lightning

    private static void strike(GameTestHelper h) {
        LightningBolt bolt = EntityType.LIGHTNING_BOLT.create(h.getLevel());
        bolt.moveTo(Vec3.atBottomCenterOf(abs(h, PowerScenarios.FEED.east(PowerScenarios.ARM - 1).above())));
        h.getLevel().addFreshEntity(bolt);
    }

    private static void lightning(GameTestHelper h, String name, boolean arrestor) {
        String[] failure = {null};
        build(h, new PowerScenarios.Station(RadioAntennaContent.ANTENNA_WIRE.get(), RadioAmpContent.AMPLIFIER_100W.get(), false, arrestor, false));
        RadioAntennaTests.steps(h, name, failure, List.of(
                () -> ready(h),
                () -> {
                    hazards(h, 1, true);
                    check(sdr(h).link().chain().hasArrestor() == arrestor, failure, "chain arrestor flag wrong");
                    check(arrestor || sdr(h).link().warnings().contains("No lightning arrestor in the feedline"), failure,
                            "no arrestor warning: " + sdr(h).link().warnings());
                    strike(h);
                    return true;
                },
                () -> {
                    boolean ampThere = amp(h) != null;
                    boolean sdrThere = h.getLevel().getBlockEntity(abs(h, PowerScenarios.SDR)) instanceof SdrBlockEntity;
                    if (arrestor) {
                        check(ampThere && sdrThere, failure, "arrestor: lightning still destroyed amp=" + !ampThere + " sdr=" + !sdrThere);
                        check(sdr(h).link().warnings().stream().anyMatch(w -> w.contains("grounded by the arrestor")), failure,
                                "strike not seen: " + (sdrThere ? sdr(h).link().warnings() : "-"));
                    } else {
                        check(!ampThere && !sdrThere, failure, "no arrestor: amp intact=" + ampThere + ", sdr intact=" + sdrThere);
                        check(h.getLevel().getBlockState(abs(h, PowerScenarios.FEED)).getBlock() instanceof ConductorBlock, failure,
                                "the antenna itself should survive");
                    }
                    clearHazards(h);
                    return true;
                }));
    }

    /** Lightning on an antenna with no arrestor in the feedline destroys the amplifier and the SDR down the coax. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".lightning")
    public static void lightning_destroys_unarrested_station(GameTestHelper h) {
        lightning(h, "lightning_destroys_unarrested_station", false);
    }

    /** Control: the same strike with a lightning arrestor in the feedline destroys nothing. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".lightning_arrestor")
    public static void lightning_arrestor_protects_station(GameTestHelper h) {
        lightning(h, "lightning_arrestor_protects_station", true);
    }

    /** The ham_station scenario, as spawned by {@code /ecm scenario spawn ham_station}. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ham_station")
    public static void ham_station(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("ham_station"));
    }
}
//?}
