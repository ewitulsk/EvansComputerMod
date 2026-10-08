package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.radio.RadioContent;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.compat.aero.AeroCompat;
import com.example.evanscomputermod.radio.compat.sable.SableShips;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.handheld.HandheldBand;
import com.example.evanscomputermod.radio.handheld.HandheldServer;
import com.example.evanscomputermod.radio.handheld.HandheldSettings;
import com.example.evanscomputermod.radio.medium.LinkCache;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.medium.RfAttenuation;
import com.example.evanscomputermod.radio.medium.RfBlock;
import com.example.evanscomputermod.radio.medium.WorldMediumContent;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import com.example.evanscomputermod.radio.phys.PolarizationLoss;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.sable.SableCompat;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.tags.TagKey;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.UUID;
import java.util.function.BooleanSupplier;

/**
 * Release gate (Phase 8) in namespace {@code ecm_radio}: Create Aeronautics RF data,
 * the {@code airship_radio} scenario, a ship's 90 degree turn against the antenna
 * pattern, assemble/fly/land/reload keeping radio state, and a handheld on a moving
 * ship. Ships are Sable sub-levels; with Aeronautics loaded they carry real
 * Aeronautics envelopes and are assembled by Create Simulated's helper. Without
 * Aeronautics the Aeronautics-only assertions log "skipped: Aeronautics not loaded"
 * and everything Sable-generic still runs.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioAeroTests {

    static final String NS = RadioTests.NS;
    static final String STRUCTURE = RadioTests.STRUCTURE;

    static void log(String f, Object... a) {
        EvansComputerMod.LOGGER.info("[ecm_radio/aero] " + String.format(Locale.ROOT, f, a));
    }

    static void skipped(String what) {
        EvansComputerMod.LOGGER.warn("[ecm_radio/aero] skipped: Aeronautics not loaded ({})", what);
    }

    // ------------------------------------------------------------ 8A: data

    /**
     * The optional Aeronautics/Simulated/Create entries of {@code rf_attenuation}, {@code rf_conductors}
     * and {@code rf_good_ground}: with Aeronautics, every envelope resolves to thin fabric (well under
     * 1 dB per block at 2.4 GHz, where its sound-type fallback would be stone at ~66 dB) and a two-block
     * envelope wall between two 2.4 GHz radios costs under 2 dB in the real medium while the same wall in
     * stone kills the link (control). Create's metal girder is a sparse metal reflector, conductor and
     * (for industrial iron) a good ground. Without the mods the conditional entries are skipped and the
     * vanilla entries still resolve (the data map loaded).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".aero_data")
    public static void aero_rf_data_loads_only_with_its_mods(GameTestHelper h) {
        RadioWorldMediumTests.Rig r = new RadioWorldMediumTests.Rig(h);
        Channel ch = RadioWorldMediumTests.WIFI;
        String[] failure = {null};
        // Vanilla entries are there either way (the file with conditional entries loaded).
        RfBlock stone = RfAttenuation.of(Blocks.STONE.defaultBlockState()), iron = RfAttenuation.of(Blocks.IRON_BLOCK.defaultBlockState());
        if (!stone.name().equals("stone") || !iron.metal()) failure[0] = "vanilla rf_attenuation entries missing: stone " + stone + ", iron " + iron;
        boolean aero = AeroCompat.aeronautics();
        Block envelope = AeroCompat.block("aeronautics:white_envelope");
        if (aero != (envelope != null) && failure[0] == null) failure[0] = "Aeronautics loaded=" + aero + " but white_envelope is " + envelope;
        if (failure[0] == null && aero) {
            TagKey<Block> envelopes = TagKey.create(Registries.BLOCK, ResourceLocation.parse("aeronautics:envelope"));
            int n = 0;
            for (var holder : BuiltInRegistries.BLOCK.getTagOrEmpty(envelopes)) {
                RfBlock b = RfAttenuation.of(holder.value().defaultBlockState());
                double db = b.lossDb(1, ch.centerHz());
                n++;
                if (db > 1 || b.metal()) failure[0] = holder.getRegisteredName() + " resolves to " + b + " (" + db + " dB/block at 2.4 GHz)";
            }
            if (n < 16 && failure[0] == null) failure[0] = "only " + n + " blocks in #aeronautics:envelope";
            RfBlock lev = RfAttenuation.of(AeroCompat.block("aeronautics:levitite").defaultBlockState());
            if (!lev.name().equals("glass") && failure[0] == null) failure[0] = "levitite resolves to " + lev;
            Block prop = AeroCompat.block("aeronautics:smart_propeller");
            if (failure[0] == null && !prop.defaultBlockState().is(RadioContent.RF_CONDUCTORS)) failure[0] = "smart_propeller not in rf_conductors";
            log("%d envelope blocks resolve to fabric: white_envelope %s", n, RfAttenuation.of(envelope.defaultBlockState()));
        } else if (!aero) {
            skipped("envelope/levitite/propeller entries; checked they are absent instead");
        }
        Block girder = AeroCompat.block("create:metal_girder");
        if (failure[0] == null && girder != null) {
            RfBlock g = RfAttenuation.of(girder.defaultBlockState());
            Block indIron = AeroCompat.block("create:industrial_iron_block");
            if (!g.metal() || Math.abs(g.fraction() - 0.4) > 1e-9) failure[0] = "metal_girder resolves to " + g;
            else if (!girder.defaultBlockState().is(RadioContent.RF_CONDUCTORS)) failure[0] = "metal_girder not in rf_conductors";
            else if (!indIron.defaultBlockState().is(RadioContent.RF_GOOD_GROUND)) failure[0] = "industrial_iron_block not in rf_good_ground";
            else log("create:metal_girder %s (conductor), industrial_iron_block good ground", g);
        }
        // In the medium: a 2-thick fabric wall (envelope, or wool without Aeronautics) vs the same wall in stone.
        BlockState fabric = AeroCompat.envelope("white");
        r.fill(8, 1, 4, 9, 4, 6, fabric);
        r.fill(8, 1, 14, 9, 4, 16, Blocks.STONE.defaultBlockState());
        var open = r.pair(2.5, 14.5, 2.5, 24.5, ch);
        var cloth = r.pair(2.5, 14.5, 2.5, 5.5, ch);
        var rock = r.pair(2.5, 14.5, 2.5, 15.5, ch);
        r.run("aero_rf_data_loads_only_with_its_mods", () -> {
            if (failure[0] != null) { r.failure = failure[0]; return; }
            double dCloth = r.gain(open, ch) - r.gain(cloth, ch), dRock = r.gain(open, ch) - r.gain(rock, ch);
            r.log("2-block wall of %s costs %.2f dB at 2.4 GHz, stone %.1f dB", fabric, dCloth, dRock);
            if (dCloth > 2) r.failure = fabric + " wall costs " + dCloth + " dB";
            else if (dRock < 60) r.failure = "control: stone wall only " + dRock + " dB";
            else if (!r.send(cloth, ch, 0, "OFDM-6", 6e6)) r.failure = "0 dBm frame did not pass the fabric wall";
            else if (r.send(rock, ch, 0, "OFDM-6", 6e6)) r.failure = "control: frame passed two blocks of stone";
        });
    }

    // ------------------------------------------------------------ 8B: airship_radio

    /** {@code /ecm scenario spawn airship_radio}: ping holds, RSSI and rate fall, the link drops at 500 m and comes back, the ship lands intact. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".airship_radio")
    public static void airship_radio(GameTestHelper h) {
        if (!AeroCompat.aeronautics()) skipped("airship_radio uses wool instead of envelopes and Sable's assembler instead of Simulated's");
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("airship_radio"));
    }

    // ------------------------------------------------------------ helpers

    /** A radio endpoint whose pose and pattern the test sets (the pose of a feed point on a ship). */
    static final class Probe implements RadioEndpoint {
        final UUID id = UUID.randomUUID();
        volatile Pose pose;
        volatile AntennaPattern pattern;
        final Channel channel;

        Probe(Pose pose, AntennaPattern pattern, Channel channel) {
            this.pose = pose;
            this.pattern = pattern;
            this.channel = channel;
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return pattern; }
        @Override public Channel tunedChannel() { return channel; }
        @Override public double maxTxPowerDbm() { return 30; }
        @Override public void onReceive(Reception r) {}
    }

    /** An isotropic antenna with a fixed linear polarization (local frame = world for an identity pose). */
    static AntennaPattern isotropic(double ex, double ey, double ez) {
        return new AntennaPattern() {
            @Override public double gainDbi(double lx, double ly, double lz) { return 0; }
            @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {ex, ey, ez}; }
            @Override public double peakGainDbi() { return 0; }
        };
    }

    static List<BlockPos> solidIn(GameTestHelper h, BlockPos minRel, BlockPos maxRel) {
        List<BlockPos> out = new ArrayList<>();
        for (BlockPos p : BlockPos.betweenClosed(h.absolutePos(minRel), h.absolutePos(maxRel)))
            if (!h.getLevel().getBlockState(p).isAir()) out.add(p.immutable());
        return out;
    }

    static double[] rot(double[] q, double x, double y, double z, boolean inverse) {
        var quat = new org.joml.Quaterniond(q[0], q[1], q[2], q[3]);
        if (inverse) quat.conjugate();
        var v = quat.transform(new org.joml.Vector3d(x, y, z));
        return new double[] {v.x, v.y, v.z};
    }

    /**
     * What the pattern and polarization alone predict for the ship antenna, given the ship's
     * orientation {@code q} (straight from Sable, not from the endpoint's pose): its gain toward
     * the receiver minus the polarization mismatch against {@code rxPol} across the path, dB.
     */
    static double predictedAntennaTerm(AntennaPattern p, double[] q, double[] u, double[] rxPol) {
        double[] local = rot(q, u[0], u[1], u[2], true);
        double gain = p.gainDbi(local[0], local[1], local[2]);
        double[] lp = p.polarization(local[0], local[1], local[2]);
        double[] e = rot(q, lp[0], lp[1], lp[2], false);
        double[] a = perp(e, u), b = perp(rxPol, u);
        double na = norm(a), nb = norm(b);
        double pol = na < 1e-6 || nb < 1e-6 ? PolarizationLoss.CROSS_POL_CAP_DB
                : PolarizationLoss.db(Math.acos(Math.min(1, Math.abs(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (na * nb))));
        return gain - pol;
    }

    static double[] perp(double[] v, double[] u) {
        double d = v[0] * u[0] + v[1] * u[1] + v[2] * u[2];
        return new double[] {v[0] - d * u[0], v[1] - d * u[1], v[2] - d * u[2]};
    }

    static double norm(double[] v) {
        return Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    }

    // ------------------------------------------------------------ 8B: ship rotation

    /**
     * Release gate: a ship carrying a copper-wire dipole (feed point + 3 + 4 blocks along x) is
     * flown 20 blocks up and turned 90 degrees twice: a yaw (pattern only: the receiver's bearing
     * goes from 60 to 30 degrees off the wire) and a pitch that stands the wire up (pattern plus
     * cross-polarization against the receiver's horizontal antenna). Each time the received level
     * (the medium's path gain) must change by the solved pattern and the polarization loss computed
     * here from Sable's own orientation, plus whatever the path's ground term did, within 0.5 dB.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ship_rotation")
    public static void ship_rotation_changes_level_by_pattern(GameTestHelper h) {
        String[] failure = {null};
        if (!SableCompat.isLoaded()) {
            failure[0] = "Sable is not loaded";
            TestDriver.drive(h, NS, "ship_rotation_changes_level_by_pattern", () -> false, () -> failure[0]);
            return;
        }
        var level = h.getLevel();
        BlockPos feedRel = new BlockPos(20, 4, 20);
        for (int x = 16; x <= 24; x++) h.setBlock(new BlockPos(x, 3, 20), Blocks.SPRUCE_PLANKS);
        RadioAntennaTests.placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X), feedRel);
        for (int i = 1; i <= 3; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feedRel.west(i));
        for (int i = 1; i <= 4; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feedRel.east(i));
        BlockPos feed = h.absolutePos(feedRel);
        SableShips.Ship[] ship = {null};
        Vec3[] home = {null};
        Probe[] eps = {null, null};
        double[] f = {0};
        double[] rxPol = {1, 0, 0};
        double[] u = {Math.cos(Math.toRadians(60)), 0, Math.sin(Math.toRadians(60))};
        double dist = 30;
        long[] mark = {0};
        double[][] before = {null};
        int[] turn = {0};
        WorldRadioMedium medium = WorldMediumContent.medium();
        List<BooleanSupplier> steps = new ArrayList<>();
        steps.add(() -> {
            ship[0] = AeroCompat.assemble(level, feed, solidIn(h, new BlockPos(16, 3, 20), new BlockPos(24, 4, 20)),
                    h.absolutePos(new BlockPos(16, 3, 20)), h.absolutePos(new BlockPos(24, 4, 20)));
            RadioAntennaTests.check(ship[0] != null, failure, "assembly failed");
            if (ship[0] == null) return false;
            home[0] = ship[0].position();
            SableShips.hold(ship[0], home[0].x, home[0].y + 20, home[0].z, new double[] {0, 0, 0, 1});
            log("rotation ship assembled by %s", AeroCompat.lastAssembler);
            return true;
        });
        steps.add(() -> {
            Antenna a = AntennaManager.get(level, ship[0].plotPos(feed));
            if (!a.solved()) return false;
            f[0] = a.resonantHz();
            Channel ch = new Channel(f[0], 10e3);
            Pose p = a.pose(level);
            eps[0] = new Probe(p, a.pattern(f[0]), ch);
            eps[1] = new Probe(Pose.at(p.dimension(), p.x() + dist * u[0], p.y(), p.z() + dist * u[2]), isotropic(rxPol[0], rxPol[1], rxPol[2]), ch);
            medium.register(eps[0]);
            medium.register(eps[1]);
            mark[0] = level.getGameTime();
            log("dipole on the ship: %s; feed pose %s", a.summary(), p);
            return true;
        });
        // Each phase: wait for a fresh trace, record level and prediction, then turn.
        BooleanSupplier measureAndTurn = () -> {
            LinkCache.Link l = medium.link(eps[0], eps[1], f[0]);
            if (l == null || l.computedTick() <= mark[0]) {
                if (level.getGameTime() - mark[0] > 200) failure[0] = "link not retraced after turn " + turn[0];
                return false;
            }
            Antenna a = AntennaManager.get(level, ship[0].plotPos(feed));
            double[] q = ship[0].orientation();
            Pose pa = eps[0].pose, pb = eps[1].pose;
            double dx = pb.x() - pa.x(), dy = pb.y() - pa.y(), dz = pb.z() - pa.z(), dn = Math.sqrt(dx * dx + dy * dy + dz * dz);
            double[] los = {dx / dn, dy / dn, dz / dn};
            double pred = predictedAntennaTerm(a.pattern(f[0]), q, los, rxPol);
            double level0 = medium.pathGainDb(eps[0], eps[1], f[0]);
            double[] now = {level0, pred, l.gainA() + l.gainB() - l.polDb(), l.excessDb()};
            log("turn %d: level %.2f dB (antennas %.2f dB, path excess %.2f dB, pol %.2f dB); predicted antenna term %.2f dB; q %s; path %s",
                    turn[0], level0, now[2], now[3], l.polDb(), pred, java.util.Arrays.toString(q), l.path());
            if (before[0] != null) {
                double dLevel = now[0] - before[0][0], dPred = now[1] - before[0][1], dAnt = now[2] - before[0][2], dExcess = now[3] - before[0][3];
                log("turn %d changed the level by %.2f dB: predicted pattern+polarization %.2f dB, medium antenna terms %.2f dB, path ground term %.2f dB",
                        turn[0], dLevel, dPred, dAnt, -dExcess);
                if (Math.abs(dPred) < 3) failure[0] = "turn " + turn[0] + " is not a meaningful check: predicted only " + dPred + " dB";
                else if (Math.abs(dAnt - dPred) > 0.5) failure[0] = "turn " + turn[0] + ": medium antenna terms changed " + dAnt + " dB, pattern predicts " + dPred;
                else if (Math.abs(dLevel - (dPred - dExcess)) > 0.5)
                    failure[0] = "turn " + turn[0] + ": level changed " + dLevel + " dB, expected " + (dPred - dExcess);
            }
            before[0] = now;
            turn[0]++;
            if (turn[0] == 1) SableShips.turn(ship[0], 0, 1, 0, 90);        // yaw: the wire swings from x to z
            else if (turn[0] == 2) SableShips.turn(ship[0], 1, 0, 0, 90);   // pitch: the wire stands up
            if (turn[0] <= 2) {
                Antenna moved = AntennaManager.get(level, ship[0].plotPos(feed));
                eps[0].pose = moved.pose(level);
                medium.invalidate(eps[0]);
                mark[0] = level.getGameTime();
            }
            return true;
        };
        steps.add(measureAndTurn);
        steps.add(measureAndTurn);
        steps.add(measureAndTurn);
        steps.add(() -> {
            medium.unregister(eps[0]);
            medium.unregister(eps[1]);
            SableShips.discard(ship[0]);
            return true;
        });
        RadioAntennaTests.steps(h, "ship_rotation_changes_level_by_pattern", failure, steps);
    }

    // ------------------------------------------------------------ 8B: assemble, fly, land, reload

    static final String RT_SSID = "ecm-roundtrip", RT_PASS = "keep me secret";

    /**
     * Release gate: assemble -> fly -> disassemble -> reload keeps radio state. A deck carries an
     * Access Point (WPA2, ch 11, 7 dBm) cabled to a second AP (two members of one cable segment), a
     * copper-wire dipole with a wrench cut, and an SDR tuned to 145.5 MHz / 250 kS/s / 20 dB / no
     * AGC. Checked before, in flight (20 up, 12 east, turned 30 degrees), after landing on the build
     * site, and after a reload (each block entity saved with full metadata and loaded into a fresh
     * instance; the antenna forgotten and re-walked from the landed blocks): AP id, settings and the
     * server-side passphrase; both APs on one cable segment; antenna block count and resonance
     * (within 1%); SDR frequency, rate, gain and AGC.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ship_round_trip")
    public static void ship_round_trip_keeps_radio_state(GameTestHelper h) {
        String[] failure = {null};
        if (!SableCompat.isLoaded()) {
            failure[0] = "Sable is not loaded";
            TestDriver.drive(h, NS, "ship_round_trip_keeps_radio_state", () -> false, () -> failure[0]);
            return;
        }
        var level = h.getLevel();
        var reg = level.registryAccess();
        // Deck x 14..26, z 18..23 at y 3, floating (nothing below).
        for (int x = 14; x <= 26; x++) for (int z = 18; z <= 23; z++) h.setBlock(new BlockPos(x, 3, z), Blocks.SPRUCE_PLANKS);
        BlockPos feedRel = new BlockPos(20, 4, 20);
        RadioAntennaTests.placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X), feedRel);
        for (int i = 1; i <= 3; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feedRel.west(i));
        for (int i = 1; i <= 5; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feedRel.east(i));
        ((com.example.evanscomputermod.radio.conductor.ConductorBlock) RadioAntennaContent.COPPER_WIRE.get())
                .toggleCut(level, h.absolutePos(feedRel.east(4)), Direction.EAST);
        BlockPos ap1Rel = new BlockPos(16, 4, 22), ap2Rel = new BlockPos(19, 4, 22), sdrRel = new BlockPos(24, 4, 22);
        List<BlockPos> cableRel = List.of(new BlockPos(17, 4, 22), new BlockPos(18, 4, 22));
        h.setBlock(ap1Rel, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
        h.setBlock(ap2Rel, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
        for (BlockPos c : cableRel) h.setBlock(c, ModBlocks.NETWORK_CABLE.get().defaultBlockState());
        for (BlockPos rel : cableRel) {
            BlockPos p = h.absolutePos(rel);
            BlockState s = level.getBlockState(p);
            for (Direction d : Direction.values()) {
                BlockPos q = p.relative(d);
                s = s.setValue(NetworkCableBlock.getPropertyForDirection(d), NetworkCableBlock.canConnectToFace(level.getBlockState(q), d.getOpposite(), level, q));
            }
            level.setBlock(p, s, 3);
        }
        h.setBlock(sdrRel, RadioSdrContent.SDR_ADVANCED.get().defaultBlockState());
        var ap1 = (AccessPointBlockEntity) h.getBlockEntity(ap1Rel);
        String err = ap1.applySettings(new ApSettings(RT_SSID, false, Security.WPA2_PSK, 11, 7, false, null, null), RT_PASS);
        if (err != null) failure[0] = "AP settings rejected: " + err;
        var sdr = ((SdrBlockEntity) h.getBlockEntity(sdrRel)).getPeripheral().radio();
        sdr.setFrequency(145.5e6);
        sdr.setSampleRate(Math.min(250_000, sdr.maxRate()));
        sdr.setGain(20);
        sdr.setAgc(false);
        BlockPos feed = h.absolutePos(feedRel), ap1Pos = h.absolutePos(ap1Rel), ap2Pos = h.absolutePos(ap2Rel), sdrPos = h.absolutePos(sdrRel);
        SableShips.Ship[] ship = {null};
        Vec3[] home = {null};
        Object[] ref = new Object[8];   // 0 apId, 1 settings, 2 pass, 3 resonance, 4 blockCount, 5 sdr tag
        String[] where = {"before"};
        // Returns true when the state at the given positions matches the reference (recording it the first time).
        java.util.function.Function<BlockPos[], Boolean> verify = pos -> {
            if (!(level.getBlockEntity(pos[0]) instanceof AccessPointBlockEntity a1) || !(level.getBlockEntity(pos[1]) instanceof AccessPointBlockEntity a2)) {
                failure[0] = where[0] + ": APs missing at " + pos[0] + " / " + pos[1];
                return false;
            }
            if (!(level.getBlockEntity(pos[3]) instanceof SdrBlockEntity s)) {
                failure[0] = where[0] + ": SDR missing at " + pos[3];
                return false;
            }
            CableNetworkManager m = CableNetworkManager.getInstance();
            Integer n1 = m == null ? null : m.networkOf(a1.portMac()), n2 = m == null ? null : m.networkOf(a2.portMac());
            if (n1 == null || !n1.equals(n2)) return false;   // segment re-forms a tick after a move
            if (a1.core() == null) return false;
            Antenna ant = AntennaManager.get(level, pos[2]);
            if (!ant.solved()) return false;
            CompoundTag sdrTag = new CompoundTag();
            var st = s.saveWithoutMetadata(reg);
            for (String k : List.of("Freq", "Rate", "Gain", "Agc")) if (st.contains(k)) sdrTag.put(k, st.get(k));
            Object[] now = {a1.apId(), String.valueOf(a1.settings()), a1.saveWithoutMetadata(reg).getString(AccessPointBlockEntity.PASSPHRASE_KEY),
                    ant.resonantHz(), ant.graph().blockCount, sdrTag};
            log("%s: AP %s %s, segment %d, antenna %d blocks %.3f MHz (%s), SDR %s", where[0], now[0], now[1], n1, now[4], ant.resonantHz() / 1e6,
                    ant.graph().groundName, sdrTag);
            if (ref[0] == null) {
                System.arraycopy(now, 0, ref, 0, now.length);
                if (!RT_PASS.equals(now[2])) failure[0] = "passphrase not stored server-side";
                return true;
            }
            if (!ref[0].equals(now[0])) failure[0] = where[0] + ": AP id " + ref[0] + " -> " + now[0];
            else if (!ref[1].equals(now[1])) failure[0] = where[0] + ": AP settings " + ref[1] + " -> " + now[1];
            else if (!ref[2].equals(now[2])) failure[0] = where[0] + ": passphrase lost";
            else if (Math.abs((double) now[3] - (double) ref[3]) > 0.01 * (double) ref[3]) failure[0] = where[0] + ": resonance " + ref[3] + " -> " + now[3];
            else if (!ref[4].equals(now[4])) failure[0] = where[0] + ": antenna blocks " + ref[4] + " -> " + now[4] + " (wrench cut lost?)";
            else if (!ref[5].equals(now[5])) failure[0] = where[0] + ": SDR " + ref[5] + " -> " + now[5];
            return failure[0] == null;
        };
        List<BooleanSupplier> steps = new ArrayList<>();
        steps.add(() -> verify.apply(new BlockPos[] {ap1Pos, ap2Pos, feed, sdrPos}));
        steps.add(() -> {
            ship[0] = AeroCompat.assemble(level, ap1Pos, solidIn(h, new BlockPos(14, 3, 18), new BlockPos(26, 4, 23)),
                    h.absolutePos(new BlockPos(14, 3, 18)), h.absolutePos(new BlockPos(26, 4, 23)));
            RadioAntennaTests.check(ship[0] != null, failure, "assembly failed");
            if (ship[0] == null) return false;
            home[0] = ship[0].position();
            double[] q = {0, Math.sin(Math.toRadians(15)), 0, Math.cos(Math.toRadians(15))};   // 30 degrees about y
            SableShips.hold(ship[0], home[0].x + 12, home[0].y + 20, home[0].z, q);
            where[0] = "in flight (" + AeroCompat.lastAssembler + ")";
            return true;
        });
        steps.add(() -> verify.apply(new BlockPos[] {ship[0].plotPos(ap1Pos), ship[0].plotPos(ap2Pos), ship[0].plotPos(feed), ship[0].plotPos(sdrPos)}));
        steps.add(() -> {
            // Fly home and level out, then set it down on the build site.
            SableShips.hold(ship[0], home[0].x, home[0].y, home[0].z, new double[] {0, 0, 0, 1});
            AeroCompat.disassemble(ship[0], ap1Pos);
            where[0] = "landed (" + AeroCompat.lastDisassembler + ")";
            return true;
        });
        steps.add(() -> verify.apply(new BlockPos[] {ap1Pos, ap2Pos, feed, sdrPos}));
        steps.add(() -> {
            // Reload: every radio block entity through its saved NBT into a fresh instance, and the antenna re-walked.
            for (BlockPos p : List.of(ap1Pos, ap2Pos, sdrPos)) {
                BlockEntity be = level.getBlockEntity(p);
                CompoundTag tag = be.saveWithFullMetadata(reg);
                BlockEntity fresh = BlockEntity.loadStatic(p, level.getBlockState(p), tag, reg);
                if (fresh == null) { failure[0] = "reload: " + p + " did not load"; return false; }
                CompoundTag again = fresh.saveWithFullMetadata(reg);
                if (fresh instanceof AccessPointBlockEntity fa && be instanceof AccessPointBlockEntity oa) {
                    if (!fa.apId().equals(oa.apId()) || !String.valueOf(fa.settings()).equals(String.valueOf(oa.settings()))
                            || !again.getString(AccessPointBlockEntity.PASSPHRASE_KEY).equals(tag.getString(AccessPointBlockEntity.PASSPHRASE_KEY))) {
                        failure[0] = "reload: AP " + p + " " + oa.settings() + " -> " + fa.settings();
                        return false;
                    }
                    if (fa.getUpdateTag(reg).contains(AccessPointBlockEntity.PASSPHRASE_KEY)) { failure[0] = "reload: passphrase in the client sync tag"; return false; }
                } else if (!again.equals(tag)) {
                    failure[0] = "reload: " + p + " " + tag + " -> " + again;
                    return false;
                }
            }
            AntennaManager.forget(level, feed);
            where[0] = "reloaded";
            return true;
        });
        steps.add(() -> verify.apply(new BlockPos[] {ap1Pos, ap2Pos, feed, sdrPos}));
        RadioAntennaTests.steps(h, "ship_round_trip_keeps_radio_state", failure, steps);
    }

    // ------------------------------------------------------------ 8B: handheld on a moving ship

    /**
     * A handheld held on the deck of a ship flying at 10 m/s (the session's pose is the deck point
     * through the ship's transform every tick) hears a ground SDR's FM test tone on 146.52 MHz;
     * an identical handheld on 147.0 MHz does not (control). The ship really moves (more than 5 m
     * during reception) and the handheld's pose follows it.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".handheld_ship")
    public static void handheld_on_moving_ship_hears_ground_station(GameTestHelper h) {
        String[] failure = {null};
        if (!SableCompat.isLoaded()) {
            failure[0] = "Sable is not loaded";
            TestDriver.drive(h, NS, "handheld_on_moving_ship_hears_ground_station", () -> false, () -> failure[0]);
            return;
        }
        var level = h.getLevel();
        BlockPos txRel = new BlockPos(5, 2, 30);
        h.setBlock(txRel, RadioSdrContent.SDR_STANDARD.get().defaultBlockState());
        for (int x = 18; x <= 20; x++) for (int z = 18; z <= 20; z++) h.setBlock(new BlockPos(x, 5, z), Blocks.SPRUCE_PLANKS);
        h.setBlock(new BlockPos(19, 6, 19), AeroCompat.envelope("white"));
        BlockPos deck = h.absolutePos(new BlockPos(19, 5, 19));
        var tuned = new HandheldSettings(true, HandheldBand.VHF, 146.52e6, 80, 0);
        var off = tuned.withFreq(147.0e6);
        var s1 = new HandheldServer.Session(UUID.randomUUID());
        var s2 = new HandheldServer.Session(UUID.randomUUID());
        SableShips.Ship[] ship = {null};
        Vec3[] home = {null};
        Vec3[] firstPose = {null};
        int[] tick = {0};
        List<float[]> heard = new ArrayList<>(), heardOff = new ArrayList<>();
        String dim = level.dimension().location().toString();
        TestDriver.drive(h, NS, "handheld_on_moving_ship_hears_ground_station", () -> {
            var medium = RadioMediumHooks.medium();
            switch (tick[0]++) {
                case 0 -> {
                    ship[0] = AeroCompat.assemble(level, deck, solidIn(h, new BlockPos(18, 5, 18), new BlockPos(20, 6, 20)),
                            h.absolutePos(new BlockPos(18, 5, 18)), h.absolutePos(new BlockPos(20, 6, 20)));
                    if (ship[0] == null) { failure[0] = "assembly failed"; return false; }
                    home[0] = ship[0].position();
                    SableShips.hold(ship[0], home[0].x, home[0].y + 8, home[0].z, new double[] {0, 0, 0, 1});
                    return false;
                }
                case 1 -> {
                    var tx = ((SdrBlockEntity) h.getBlockEntity(txRel)).getPeripheral().radio();
                    tx.setFrequency(146.52e6);
                    tx.setSampleRate(48_000);
                    tx.setTx(true, 10);
                    int n = 48_000;
                    float[] iq = new float[2 * n];
                    double ph = 0;
                    for (int k = 0; k < n; k++) {
                        ph += 2 * Math.PI * 5000 * 0.8 * Math.sin(2 * Math.PI * 1000 * k / 48_000.0) / 48_000.0;
                        iq[2 * k] = (float) Math.cos(ph);
                        iq[2 * k + 1] = (float) Math.sin(ph);
                    }
                    tx.write(medium, iq, n);
                    return false;
                }
                default -> {
                    // 10 m/s east: half a block per tick.
                    double t = tick[0] - 2;
                    SableShips.hold(ship[0], home[0].x + 0.5 * t, home[0].y + 8, home[0].z, new double[] {0, 0, 0, 1});
                    Vec3 w = ship[0].toWorld(Vec3.atCenterOf(ship[0].plotPos(deck)).add(0, 0.5 + 1.6, 0));
                    if (firstPose[0] == null) firstPose[0] = w;
                    Pose pose = Pose.at(dim, w.x, w.y, w.z);
                    float[] a = s1.receive(pose, tuned, medium), b = s2.receive(pose, off, medium);
                    if (t >= 6) {   // after the first retraces
                        if (a != null) heard.add(a);
                        if (b != null) heardOff.add(b);
                    }
                    if (t < 18) return false;
                    s1.close(medium);
                    s2.close(medium);
                    SableShips.discard(ship[0]);
                    float[] all = concat(heard), allOff = concat(heardOff);
                    double moved = w.distanceTo(firstPose[0]);
                    double t1 = RadioTests.tone(all, 1000, 24_000), p1 = RadioTests.peakiness(all), p2 = RadioTests.peakiness(allOff);
                    log("handheld on the ship moved %.1f m (deck at y %.1f): 1 kHz level %.3f, peak ratio %.1f (off-frequency %.1f), signal %.1f dBm",
                            moved, w.y, t1, p1, p2, s1.lastSignalDbm);
                    if (moved < 5) failure[0] = "the ship did not move: " + moved + " m";
                    else if (w.distanceTo(Vec3.atCenterOf(deck).add(0.5 * t, 8 + 0.5 + 1.6, 0)) > 0.5)
                        failure[0] = "handheld pose " + w + " is not on the deck";
                    else if (all.length == 0 || t1 < 0.2 || p1 < 10) failure[0] = "tuned handheld on the moving ship: 1 kHz " + t1 + " peak ratio " + p1 + " signal " + s1.lastSignalDbm;
                    else if (p2 > 4) failure[0] = "control: off-frequency handheld heard the tone (peak ratio " + p2 + ")";
                    return failure[0] == null;
                }
            }
        }, () -> failure[0]);
    }

    static float[] concat(List<float[]> parts) {
        int n = 0;
        for (float[] p : parts) n += p.length;
        float[] out = new float[n];
        int o = 0;
        for (float[] p : parts) {
            System.arraycopy(p, 0, out, o, p.length);
            o += p.length;
        }
        return out;
    }

    private RadioAeroTests() {}
}
//?}
