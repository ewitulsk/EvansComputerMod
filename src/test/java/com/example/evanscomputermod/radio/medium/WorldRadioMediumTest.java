package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.phys.Materials;
import com.example.evanscomputermod.radio.phys.SpectralMask;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

/** Link cache, invalidation, spatial index, sharding and delivery of {@link WorldRadioMedium} over a synthetic world. */
public class WorldRadioMediumTest {

    static final Channel CH6 = Channel.wifi24(6), CH1 = Channel.wifi24(1), CH11 = Channel.wifi24(11);

    final AtomicLong clock = new AtomicLong(1_000_000);
    final GridWorld world = new GridWorld(64, RfBlock.DIRT);
    final List<Object> subLevels = new ArrayList<>();
    Object shipOf = null;
    final WorldRadioMedium.WorldAccess access = new WorldRadioMedium.WorldAccess() {
        @Override public RfWorld world(String d) { return world; }
        @Override public WorldRadioMedium.Conditions conditions(String d) { return WorldRadioMedium.Conditions.DEFAULT; }
        @Override public Object subLevelOf(String d, double x, double y, double z) { return shipOf; }
    };
    final WorldRadioMedium m = new WorldRadioMedium(clock::get, 7);
    long tick = 100;

    void tick() {
        m.tick(tick++, access);
    }

    Emission frame(Channel ch, String mod) {
        return Emission.frame(ch, 20, clock.get(), 200, mod, 1e6, new byte[] {1, 2, 3, 4});
    }

    @Test
    void unknownPairUsesFreeSpaceMarginThenTheTraceAfterATick() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, CH6), b = new TestEp(10.5, 65.5, 0.5, CH6);
        world.fill(5, 64, -3, 5, 68, 3, RfBlock.STONE);
        m.register(a);
        m.register(b);
        assertTrue(Double.isNaN(m.pathGainDb(a, b, CH6.centerHz())), "not traced yet");
        // Before tracing: free space + margin, the frame gets through.
        m.transmit(a, frame(CH6, "DSSS-1"));
        assertEquals(1, b.got.size());
        tick();
        double g = m.pathGainDb(a, b, CH6.centerHz());
        assertFalse(Double.isNaN(g));
        assertTrue(-g > 65 + 40, "wall in the trace: " + g);
        clock.addAndGet(10_000);
        m.transmit(a, frame(CH6, "DSSS-1"));
        assertEquals(1, b.got.size(), "the stone wall now blocks 2.4 GHz");
    }

    @Test
    void blockChangeOnThePathInvalidatesThePair() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, CH6), b = new TestEp(10.5, 65.5, 0.5, CH6);
        m.register(a);
        m.register(b);
        m.pathGainDb(a, b, CH6.centerHz());
        tick();
        double open = m.pathGainDb(a, b, CH6.centerHz());
        world.fill(5, 64, -3, 5, 68, 3, RfBlock.STONE);
        tick();
        assertEquals(open, m.pathGainDb(a, b, CH6.centerHz()), 1e-9, "no event yet: cache unchanged (no polling)");
        m.onBlockChanged("minecraft:overworld", 5, 65, 0, false);
        tick();
        assertEquals(Materials.STONE.dbPerBlock(CH6.centerHz()), open - m.pathGainDb(a, b, CH6.centerHz()), 1e-6);
        // A change elsewhere (another section) does not queue the pair.
        m.onBlockChanged("minecraft:overworld", 500, 65, 500, false);
        assertEquals(0, m.queuedLinks());
    }

    @Test
    void movingPastTheThresholdRetracesAndInterpolatesBelowIt() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, CH6), b = new TestEp(20.5, 65.5, 0.5, CH6);
        m.register(a);
        m.register(b);
        m.pathGainDb(a, b, CH6.centerHz());
        tick();
        long before = m.computedTotal();
        double g0 = m.pathGainDb(a, b, CH6.centerHz());
        // 0.3 m: under the 0.5 m threshold → no retrace, free-space delta only.
        b.pose = Pose.at("minecraft:overworld", 20.8, 65.5, 0.5);
        tick();
        assertEquals(before, m.computedTotal());
        double g1 = m.pathGainDb(a, b, CH6.centerHz());
        assertEquals(20 * Math.log10(20.3 / 20.0), g0 - g1, 1e-6);
        // Put a wall at the new place and move 5 blocks: retraced.
        world.fill(30, 64, -3, 30, 68, 3, RfBlock.STONE);
        b.pose = Pose.at("minecraft:overworld", 40.5, 65.5, 0.5);
        tick();
        assertTrue(m.computedTotal() > before);
        assertTrue(-m.pathGainDb(a, b, CH6.centerHz()) > 65 + 60);
    }

    @Test
    void subLevelMovesAreRateLimitedPerShip() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, CH6), b = new TestEp(20.5, 65.5, 0.5, CH6), c = new TestEp(0.5, 65.5, 20.5, CH6);
        m.register(a);
        m.register(b);
        m.register(c);
        tick();
        tick();
        long base = m.computedTotal();
        shipOf = "ship-1";   // every endpoint "is on" the same ship
        for (int i = 1; i <= 3; i++) {
            a.pose = Pose.at("minecraft:overworld", 0.5 + i, 65.5, 0.5);
            b.pose = Pose.at("minecraft:overworld", 20.5 + i, 65.5, 0.5);
            tick();
        }
        long afterFast = m.computedTotal() - base;
        // Default minRecomputeTicks = 4: within 3 ticks the ship may trigger one recompute round (a's pairs),
        // b is held back (interpolating) until the limiter allows.
        assertTrue(afterFast <= 3, "rate-limited: " + afterFast);
        for (int i = 0; i < 8; i++) tick();
        assertTrue(m.computedTotal() - base > afterFast, "held-back moves are applied later");
    }

    @Test
    void ironRoomIsolatesGlassRoomDoesNot() {
        TestEp tx = new TestEp(0.5, 65.5, 0.5, CH6), inIron = new TestEp(20.5, 65.5, 0.5, CH6), inGlass = new TestEp(0.5, 65.5, 20.5, CH6);
        world.shell(17, 64, -3, 23, 68, 3, RfBlock.IRON).shell(-3, 64, 17, 3, 68, 23, RfBlock.GLASS);
        for (TestEp e : List.of(tx, inIron, inGlass)) m.register(e);
        tick();
        tick();
        m.transmit(tx, frame(CH6, "OFDM-6"));
        assertEquals(0, inIron.got.size(), "Faraday cage");
        assertEquals(1, inGlass.got.size(), "glass control");
        assertTrue(inGlass.got.get(0).timestampMicros() >= inGlass.got.get(0).emission().endMicros());
    }

    @Test
    void spatialIndexOnlyVisitsOverlappingChannelsAndNearbyCells() {
        TestEp tx = new TestEp(0.5, 65.5, 0.5, CH6);
        m.register(tx);
        List<TestEp> otherChannel = new ArrayList<>(), farAway = new ArrayList<>();
        for (int i = 0; i < 50; i++) {
            TestEp o = new TestEp(5 + i, 65.5, 3, i % 2 == 0 ? CH1 : CH11);
            otherChannel.add(o);
            m.register(o);
            TestEp f = new TestEp(2_000_000 + i * 100, 65.5, 0.5, CH6);
            farAway.add(f);
            m.register(f);
        }
        TestEp near = new TestEp(10.5, 65.5, 0.5, CH6);
        m.register(near);
        m.transmit(tx, frame(CH6, "DSSS-1"));
        assertEquals(1, near.got.size());
        for (TestEp o : otherChannel) assertEquals(0, o.asked.get(), "channel 1/11 radios never visited");
        for (TestEp f : farAway) assertEquals(0, f.asked.get(), "radios beyond the link-budget range are culled before any per-receiver work");
    }

    @Test
    void adjacentChannelInterferenceUsesTheSpectralMask() {
        TestEp tx = new TestEp(0.5, 65.5, 0.5, CH6), rx = new TestEp(60.5, 65.5, 0.5, CH6);
        TestEp jammer3 = new TestEp(60.5, 65.5, 10.5, Channel.wifi24(3)), jammer11 = new TestEp(60.5, 65.5, -29.5, CH11);
        for (TestEp e : List.of(tx, rx, jammer3, jammer11)) m.register(e);
        tick();
        tick();
        double w3 = SpectralMask.DSSS_22MHZ.weight(Channel.wifi24(3).centerHz(), 22e6, CH6.centerHz(), 22e6);
        double w11 = SpectralMask.DSSS_22MHZ.weight(CH11.centerHz(), 22e6, CH6.centerHz(), 22e6);
        assertTrue(w3 > 100 * w11, "ch 3 leaks into ch 6 far more than ch 11");
        // Jammer on 11 (non-overlapping) then the frame: delivered.
        m.transmit(jammer11, Emission.energy(CH11, 20, clock.get(), 5_000));
        m.transmit(tx, Emission.frame(CH6, 20, clock.get(), 400, "OFDM-24", 24e6, new byte[1000]));
        assertEquals(1, rx.got.size(), "1/6/11 coexist");
        clock.addAndGet(20_000);
        m.transmit(jammer3, Emission.energy(Channel.wifi24(3), 20, clock.get(), 5_000));
        m.transmit(tx, Emission.frame(CH6, 20, clock.get(), 400, "OFDM-24", 24e6, new byte[1000]));
        assertEquals(1, rx.got.size(), "overlapping channel 3 near the receiver drowns 24 Mbps");
        assertTrue(m.channelPowerDbm(rx, CH6) > -60);
    }

    @Test
    void crossPolarizedAntennasLoseAbout20Db() {
        TestEp a = new TestEp(0.5, 70.5, 0.5, CH6), v = new TestEp(30.5, 70.5, 0.5, CH6), h = new TestEp(30.5, 70.5, 0.5, CH6);
        // h: the same dipole lying along z (rotated 90° about x).
        float s = (float) Math.sqrt(0.5);
        h.pose = new Pose("minecraft:overworld", 30.5, 70.5, 0.5, s, 0, 0, s);
        m.register(a);
        m.register(v);
        tick();
        LinkCache.Link lv = m.link(a, v, CH6.centerHz());
        m.unregister(v);
        m.register(h);
        m.link(a, h, CH6.centerHz());
        tick();
        LinkCache.Link lh = m.link(a, h, CH6.centerHz());
        assertNotNull(lv);
        assertNotNull(lh);
        assertEquals(0, lv.polDb(), 0.01);
        assertEquals(20, lh.polDb(), 0.01);
    }

    @Test
    void modulationNamesMapToPer() {
        for (String n : List.of("DSSS-1", "DSSS-2", "CCK-5.5", "CCK-11", "OFDM-6", "OFDM-54", "HT-MCS0", "HT-MCS7", "HT-MCS15",
                "CHIRP-SF12", "AFSK1200", "FSK", "BPSK", "QPSK", "CTRL", "SOMETHING")) {
            double good = PerModels.per(n, 40, 1000, 22e6, 1e6), bad = PerModels.per(n, -30, 1000, 22e6, 1e6);
            assertTrue(good < 0.01, n + " at 40 dB: " + good);
            assertTrue(bad > 0.5, n + " at -30 dB: " + bad);
        }
        assertTrue(PerModels.per("CHIRP-SF12", -15, 200, 125e3, 0) < 0.1, "LoRa SF12 decodes below the noise");
        assertTrue(PerModels.per("OFDM-54", 15, 8000, 20e6, 54e6) > PerModels.per("OFDM-6", 15, 8000, 20e6, 6e6));
    }

    @Test
    void unregisterDropsCachedPairs() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, CH6), b = new TestEp(10.5, 65.5, 0.5, CH6);
        m.register(a);
        m.register(b);
        tick();
        assertEquals(1, m.cachedLinks());
        m.unregister(b);
        tick();
        assertEquals(0, m.cachedLinks());
    }

    @Test
    void bandShardsTransmitConcurrently() throws Exception {
        List<TestEp> eps = new ArrayList<>();
        Channel[] chans = {CH1, CH6, CH11, new Channel(7.1e6, 3e3), new Channel(145e6, 12.5e3), new Channel(433e6, 125e3)};
        for (int i = 0; i < 60; i++) {
            TestEp e = new TestEp((i % 10) * 3 + 0.5, 65.5, (i / 10) * 3 + 0.5, chans[i % chans.length]);
            eps.add(e);
            m.register(e);
        }
        tick();
        tick();
        ExecutorService pool = Executors.newFixedThreadPool(6);
        List<Future<?>> fs = new ArrayList<>();
        for (int t = 0; t < 6; t++) {
            int shard = t;
            fs.add(pool.submit(() -> {
                for (int k = 0; k < 500; k++) {
                    TestEp from = eps.get(shard + chans.length * (k % 10));
                    Channel c = from.ch;
                    m.transmit(from, Emission.frame(c, 20, clock.get() + k * 1000L, 100,
                            c.centerHz() > 1e9 ? "DSSS-1" : "BPSK", 1e3, new byte[] {(byte) k, 1}));
                }
            }));
        }
        for (Future<?> f : fs) f.get(30, TimeUnit.SECONDS);
        pool.shutdown();
        for (TestEp e : eps) {
            assertFalse(e.got.isEmpty(), "every radio heard its channel: " + e.ch);
            for (var r : e.got) assertTrue(r.emission().channel().overlaps(e.ch), "no cross-band delivery");
        }
    }

    @Test
    void sameEndpointIsNotDuplicatedAndAntennaChangeRetraces() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, CH6), b = new TestEp(10.5, 65.5, 0.5, CH6);
        m.register(a);
        m.register(a);
        m.register(b);
        assertEquals(2, m.endpointCount());
        tick();
        LinkCache.Link before = m.link(a, b, CH6.centerHz());
        a.antenna = AntennaPattern.ISOTROPIC;
        m.invalidate(a);
        tick();
        LinkCache.Link after = m.link(a, b, CH6.centerHz());
        assertNotSame(before, after);
        assertEquals(0, after.gainA(), 1e-9, "a (registered first, lower index) is isotropic now");
        assertEquals(2.15, before.gainA(), 0.01);
    }
}
