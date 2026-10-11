package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.phys.Fading;
import com.example.evanscomputermod.radio.phys.Ground;
import com.example.evanscomputermod.radio.phys.PathLossModel;
import com.example.evanscomputermod.radio.phys.Polarization;
import com.example.evanscomputermod.radio.phys.TwoRay;

import org.junit.jupiter.api.Test;

import java.util.concurrent.atomic.AtomicLong;

/**
 * Regressions for the in-world medium (radio bug list 2026-10): skywave re-evaluation on
 * ionosphere changes, per-frequency trace bins, the request ring's dedup slots after an
 * overflow, ground changes under an antenna, no negative path loss, and one movement policy.
 */
public class MediumRegressionTest {

    static final String DIM = "minecraft:overworld";
    final AtomicLong clock = new AtomicLong(1_000_000);
    final GridWorld world = new GridWorld(64, RfBlock.DIRT);
    long dayTime = 6000;
    Object ship = null;
    final WorldRadioMedium.WorldAccess access = new WorldRadioMedium.WorldAccess() {
        @Override public RfWorld world(String d) { return world; }
        @Override public WorldRadioMedium.Conditions conditions(String d) { return new WorldRadioMedium.Conditions(dayTime, false, true); }
        @Override public Object subLevelOf(String d, double x, double y, double z) { return ship; }
    };
    final WorldRadioMedium m = new WorldRadioMedium(clock::get, 7);
    long tick = 1;

    void tick() {
        m.tick(tick++, access);
    }

    // ------------------------------------------------------------ 6a: skywave on ionosphere change

    @Test
    void dayNightChangeSwitchesAGroundWavePairIntoSkywave() {
        // Find an HF path that is not skywave at one time of day and is at the other.
        double[] freqs = {3.6e6, 5e6, 7.1e6, 10.1e6, 14.1e6, 18.1e6, 21.1e6};
        int[] dists = {300, 600, 1000, 1500, 2000, 3000, 4000, 5000, 6000};
        long[] times = {6000, 18000};
        double fHz = 0;
        int dist = 0;
        long first = 0, second = 0;
        search:
        for (double f : freqs)
            for (int d : dists)
                for (int i = 0; i < 2; i++) {
                    long t0 = times[i], t1 = times[1 - i];
                    var r0 = m.traceNow(world, Pose.at(DIM, 0.5, 66, 0.5), Pose.at(DIM, d + 0.5, 66, 0.5), f,
                            new WorldRadioMedium.Conditions(t0, false, true));
                    var r1 = m.traceNow(world, Pose.at(DIM, 0.5, 66, 0.5), Pose.at(DIM, d + 0.5, 66, 0.5), f,
                            new WorldRadioMedium.Conditions(t1, false, true));
                    if (r0.mode() != PathLossModel.Mode.SKYWAVE && r1.mode() == PathLossModel.Mode.SKYWAVE) {
                        fHz = f;
                        dist = d;
                        first = t0;
                        second = t1;
                        break search;
                    }
                }
        assertTrue(fHz > 0, "no HF path changes mode between day and night");
        Channel ch = new Channel(fHz, 3e3);
        TestEp a = new TestEp(0.5, 66, 0.5, ch), b = new TestEp(dist + 0.5, 66, 0.5, ch);
        dayTime = first;
        m.register(a);
        m.register(b);
        m.pathGainDb(a, b, fHz);
        tick();
        tick();
        assertNotEquals(PathLossModel.Mode.SKYWAVE, m.link(a, b, fHz).path().mode(), "starts on the ground");
        assertTrue(m.ionospherePairs() >= 1, "the HF pair is watched for ionosphere changes");
        dayTime = second;   // the sun moves (night falls / day breaks)
        for (int i = 0; i < 2 * WorldRadioMedium.SKY_REFRESH_TICKS; i++) tick();
        assertEquals(PathLossModel.Mode.SKYWAVE, m.link(a, b, fHz).path().mode(),
                "after the ionosphere changed the pair must be re-traced into skywave (" + fHz / 1e6 + " MHz, " + dist + " blocks)");
    }

    @Test
    void frozenClockDoesNotRetraceIonospherePairs() {
        Channel ch = new Channel(7.1e6, 3e3);
        TestEp a = new TestEp(0.5, 66, 0.5, ch), b = new TestEp(40.5, 66, 0.5, ch);
        m.register(a);
        m.register(b);
        m.pathGainDb(a, b, 7.1e6);
        tick();
        tick();
        long before = m.computedTotal();
        for (int i = 0; i < 3 * WorldRadioMedium.SKY_REFRESH_TICKS; i++) tick();   // dayTime never moves
        assertTrue(m.computedTotal() - before <= 1, "frozen time: at most the first refresh");
    }

    @Test
    void ionosphereRefreshStaysWithinTheRayBudget() {
        // 120 HF stations in a 200-block square (7140 pairs): a refresh re-queues them in the
        // background; each tick still traces only what the ray budget allows.
        Channel ch = new Channel(7.1e6, 3e3);
        java.util.List<TestEp> eps = new java.util.ArrayList<>();
        java.util.SplittableRandom rnd = new java.util.SplittableRandom(3);
        for (int i = 0; i < 120; i++) {
            TestEp e = new TestEp(rnd.nextDouble(200), 66, rnd.nextDouble(200), ch, false);
            eps.add(e);
            m.register(e);
        }
        for (int i = 0; i < 40; i++) tick();
        int watched = m.ionospherePairs();
        dayTime = 12000;
        tick = (tick / WorldRadioMedium.SKY_REFRESH_TICKS + 1) * WorldRadioMedium.SKY_REFRESH_TICKS;
        long t0 = System.nanoTime();
        tick();
        double ms = (System.nanoTime() - t0) / 1e6;
        int traced = m.computedLastTick();
        System.out.printf("ionosphere refresh: %d pairs watched, %d queued, %d traced in the refresh tick (%.2f ms)%n",
                watched, m.queuedLinks(), traced, ms);
        assertTrue(watched > 1000, "pairs watched: " + watched);
        assertTrue(traced < watched, "the refresh is spread over ticks by the ray budget");
        assertTrue(ms < 250, "refresh tick took " + ms + " ms");
    }

    // ------------------------------------------------------------ 6b: per-frequency trace bins

    @Test
    void aTraceAtOneFrequencyIsNotReusedFarAcrossTheBand() {
        world.fill(10, 64, -3, 10, 70, 3, RfBlock.STONE);
        Channel ch = new Channel(3.6e6, 3e3);
        TestEp a = new TestEp(0.5, 65.5, 0.5, ch), b = new TestEp(20.5, 65.5, 0.5, ch);
        m.register(a);
        m.register(b);
        tick();
        tick();
        LinkCache.Link low = m.link(a, b, 3.6e6);
        assertNotNull(low);
        assertEquals(3.6e6, low.freqHz(), 1);
        LinkCache.Link high = m.link(a, b, 28e6);
        assertTrue(high == null || Math.abs(high.freqHz() - 28e6) < 0.2 * 28e6,
                "28 MHz must not get the 3.6 MHz trace: " + (high == null ? "-" : high.freqHz()));
        tick();
        high = m.link(a, b, 28e6);
        assertNotNull(high);
        assertEquals(28e6, high.freqHz(), 0.2 * 28e6);
        assertTrue(high.path().obstructionDb() > low.path().obstructionDb(), "a stone wall loses more at 28 MHz than at 3.6");
        // Nearby frequencies (same quarter octave) still share one trace.
        assertSame(m.link(a, b, 3.6e6), m.link(a, b, 3.7e6));
    }

    // ------------------------------------------------------------ 29a: request ring overflow

    @Test
    void aRequestLostToRingOverflowCanBeAskedAgain() {
        TestEp a = new TestEp(0.5, 65.5, 0.5, Channel.wifi24(6)), b = new TestEp(300.5, 65.5, 0.5, Channel.wifi24(6));
        m.register(a);
        m.register(b);
        tick();   // nodes known; 300 blocks apart: discovery doesn't pair them
        assertTrue(Double.isNaN(m.pathGainDb(a, b, Channel.wifi24(6).centerHz())), "asked, not traced");
        long key = WorldRadioMedium.pairKey(1, 2, WorldRadioMedium.traceBin(Channel.wifi24(6).centerHz()));
        int slot = (int) (Fading.mix64(key) & 4095);
        // 70,000 other requests (for endpoints that don't exist) overflow the 65,536-entry ring,
        // all in other dedup slots so nothing overwrites this pair's "already asked" mark.
        int sent = 0;
        for (long k = 1; sent < 70_000; k++) {
            long junk = WorldRadioMedium.pairKey(1_000_000 + (int) (k % 9000), 2_000_000 + (int) (k / 9000), 99);
            if ((int) (Fading.mix64(junk) & 4095) == slot) continue;
            m.request(junk);
            sent++;
        }
        tick();   // drains what is left; this pair's request was lost
        assertTrue(Double.isNaN(m.pathGainDb(a, b, Channel.wifi24(6).centerHz())), "the lost request: still not traced");
        tick();
        assertFalse(Double.isNaN(m.pathGainDb(a, b, Channel.wifi24(6).centerHz())), "asking again after the overflow must work");
    }

    // ------------------------------------------------------------ 29b: ground under an antenna

    @Test
    void groundChangingUnderAnAntennaRetracesThePair() {
        // The antennas are up at y 88-90 (section 5); the ground below them is in sections 3-4.
        TestEp a = new TestEp(0.5, 90.5, 0.5, Channel.wifi24(6)), b = new TestEp(20.5, 88.5, 0.5, Channel.wifi24(6));
        m.register(a);
        m.register(b);
        tick();
        tick();
        double before = m.pathGainDb(a, b, Channel.wifi24(6).centerHz());
        assertFalse(Double.isNaN(before));
        assertEquals(0, m.queuedLinks());
        world.fill(-2, 64, -2, 2, 70, 2, RfBlock.STONE);   // a mound under a: its height above ground changes
        m.onBlockChanged(DIM, 0, 70, 0, false);
        assertTrue(m.queuedLinks() > 0, "the pair must be queued when the ground under an antenna changes");
        tick();
        assertNotEquals(before, m.pathGainDb(a, b, Channel.wifi24(6).centerHz()), 1e-9);
    }

    // ------------------------------------------------------------ 29c: no negative path loss

    @Test
    void shortVlfLinksNeverShowAPathGain() {
        for (double f : new double[] {15e3, 60e3, 200e3, 1e6})
            for (double d : new double[] {2, 10, 50, 200}) {
                double tr = TwoRay.lossDb(d, f, 10, 2, Ground.AVERAGE_GROUND, Polarization.VERTICAL);
                assertTrue(tr >= 0, "two-ray " + f + " Hz " + d + " m: " + tr);
            }
        TestEp a = new TestEp(0.5, 66.5, 0.5, new Channel(20e3, 200)), b = new TestEp(30.5, 66.5, 0.5, new Channel(20e3, 200));
        m.register(a);
        m.register(b);
        m.pathGainDb(a, b, 20e3);
        tick();
        double g = m.pathGainDb(a, b, 20e3);
        assertFalse(Double.isNaN(g));
        assertTrue(g <= 0, "path gain at 20 kHz over 30 blocks: " + g + " dB");
    }

    // ------------------------------------------------------------ 7: one movement policy

    @Test
    void turningUpdatesGainsAtOnceWhileShipRetracesAreRateLimited() {
        // A horizontal-ish pattern: strong along +X, weak along +Z.
        AntennaPattern beam = new AntennaPattern() {
            @Override public double gainDbi(double lx, double ly, double lz) { return lx > 0.7 ? 10 : -10; }
            @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {0, 1, 0}; }
            @Override public double peakGainDbi() { return 10; }
        };
        TestEp a = new TestEp(0.5, 65.5, 0.5, Channel.wifi24(6)), b = new TestEp(20.5, 65.5, 0.5, Channel.wifi24(6));
        a.antenna = beam;
        m.register(a);
        m.register(b);
        tick();
        tick();
        assertEquals(10, m.link(a, b, Channel.wifi24(6).centerHz()).gainA(), 1e-9);
        ship = "ship";
        // The ship turns a by 90° (now pointing its beam along -Z) on consecutive ticks.
        long traced = m.computedTotal();
        a.pose = new Pose(DIM, 0.5, 65.5, 0.5, 0, (float) Math.sin(Math.PI / 4), 0, (float) Math.cos(Math.PI / 4));
        tick();
        assertEquals(-10, m.link(a, b, Channel.wifi24(6).centerHz()).gainA(), 1e-9, "gain follows the turn on the next tick");
        long afterFirst = m.computedTotal() - traced;
        for (int i = 0; i < 3; i++) {
            a.pose = new Pose(DIM, 0.5 + 0.6 * (i + 1), 65.5, 0.5, 0, (float) Math.sin(Math.PI / 4), 0, (float) Math.cos(Math.PI / 4));
            tick();
        }
        assertTrue(m.computedTotal() - traced <= afterFirst + 1, "moves within minRecomputeTicks don't retrace every tick");
    }

    @Test
    void hardwareMovingDoesNotInvalidateTheMediumItself() {
        int[] invalidations = {0};
        BasicRadioMedium counting = new BasicRadioMedium(clock::get, 1) {
            @Override public void invalidate(com.example.evanscomputermod.radio.api.RadioEndpoint e) { invalidations[0]++; }
        };
        var ap = new com.example.evanscomputermod.radio.wifi.ap.WifiAirLink(java.util.UUID.randomUUID(), AntennaPattern.VERTICAL_DIPOLE, 20, 6, 20);
        ap.setPose(Pose.at(DIM, 0, 70, 0));
        ap.attach(counting);
        for (int i = 1; i <= 5; i++) ap.setPose(Pose.at(DIM, i * 2.0, 70, 0));   // a ship carrying it moves 10 m
        assertEquals(0, invalidations[0], "the AP must leave movement to the medium's rate-limited policy");
        var mw = new com.example.evanscomputermod.radio.microwave.MicrowaveLink(java.util.UUID.randomUUID(), new byte[] {2, 0, 0, 0, 0, 1},
                () -> counting, f -> {}, System::currentTimeMillis);
        mw.setDish(Pose.at(DIM, 0, 70, 0), 0.6);
        mw.tick();
        int base = invalidations[0];
        for (int i = 1; i <= 5; i++) mw.setDish(new Pose(DIM, 0, 70, 0, 0, (float) Math.sin(0.05 * i), 0, (float) Math.cos(0.05 * i)), 0.6);
        assertEquals(base, invalidations[0], "turning a dish is noticed by the medium, not pushed by the radio");
        mw.setDish(null, 0);
        assertEquals(base + 1, invalidations[0], "removing the dish (an antenna change) still invalidates");
        mw.stop();
    }

    // ------------------------------------------------------------ SDR transmit lead vs the airwaves ring

    @Test
    void anSdrTransmissionScheduledAheadIsHeardWhenAnotherTransmitterSendsAfterIt() {
        // Two VHF transmitters: one schedules a 20 ms chunk 300 ms ahead (an SDR's transmit lead),
        // then the other sends a chunk starting now. A receiver reading the window 300-320 ms ahead
        // must hear the first one.
        Channel ch = new Channel(144.39e6, 48_000);
        TestEp t1 = new TestEp(0.5, 66, 0.5, ch), t2 = new TestEp(5.5, 66, 0.5, ch), rx = new TestEp(10.5, 66, 0.5, ch);
        for (TestEp e : java.util.List.of(t1, t2, rx)) m.register(e);
        tick();
        tick();
        long now = clock.get();
        float[] chunk = new float[2 * 960];
        java.util.Arrays.fill(chunk, 0.5f);
        m.transmit(t1, com.example.evanscomputermod.radio.api.Emission.iq(ch, 0, now + 300_000, chunk, 48_000));
        m.transmit(t2, com.example.evanscomputermod.radio.api.Emission.iq(new Channel(144.80e6, 48_000), 0, now, chunk, 48_000));
        java.util.List<java.util.UUID> heard = new java.util.ArrayList<>();
        m.forEachHeard(rx, ch, now + 300_000, now + 320_000, h -> heard.add(h.from().id()));
        assertTrue(heard.contains(t1.id), "the chunk scheduled ahead was skipped by the ring scan");
    }
}
