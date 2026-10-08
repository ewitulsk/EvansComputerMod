package com.example.evanscomputermod.radio.microwave;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicLong;

/** Two microwave links on the reference medium: bridging, MAC filtering, misaim, weather and adaptive modulation. */
public class MicrowaveLinkTest {

    final AtomicLong clock = new AtomicLong(1_000_000);
    final BasicRadioMedium medium = new BasicRadioMedium(clock::get, 42);
    final List<MicrowaveLink> links = new ArrayList<>();

    final class End {
        final List<byte[]> cable = new CopyOnWriteArrayList<>();
        final byte[] mac;
        final MicrowaveLink link;
        final double[] at;

        End(int n, double x, double y, double z) {
            mac = new byte[] {0x02, 0x6d, 0, 0, 0, (byte) n};
            at = new double[] {x, y, z};
            link = new MicrowaveLink(UUID.randomUUID(), mac, () -> medium, cable::add, System::currentTimeMillis);
            links.add(link);
        }

        void aimAt(End o, double diameter, double offYawDeg) {
            double[] aim = DishAim.aimForWorld(new double[] {o.at[0] - at[0], o.at[1] - at[1], o.at[2] - at[2]}, v -> v);
            float[] q = DishAim.worldQuaternion(aim[0] + offYawDeg, aim[1], v -> v);
            link.setDish(new Pose("overworld", at[0], at[1], at[2], q[0], q[1], q[2], q[3]), diameter);
        }
    }

    @AfterEach
    void stop() {
        for (MicrowaveLink l : links) l.stop();
    }

    static byte[] eth(byte[] dst, byte[] src, int tag) {
        byte[] f = new byte[60];
        System.arraycopy(dst, 0, f, 0, 6);
        System.arraycopy(src, 0, f, 6, 6);
        f[12] = 0x08;
        f[20] = (byte) tag;
        return f;
    }

    static final byte[] BCAST = {-1, -1, -1, -1, -1, -1};
    static final byte[] HOST_A = {0x02, 0x11, 0, 0, 0, 1}, HOST_B = {0x02, 0x22, 0, 0, 0, 2}, HOST_A2 = {0x02, 0x11, 0, 0, 0, 3};

    End[] pair(MwBand band, double distance, double diameter, double txDbm, double offYawB) {
        End a = new End(1, 0, 70, 0), b = new End(2, distance, 75, 0);
        for (End e : new End[] {a, b}) {
            e.link.configure(band, band.defaultWidthMhz, 0);
            e.link.setTxPowerDbm(txDbm);
        }
        a.aimAt(b, diameter, 0);
        b.aimAt(a, diameter, offYawB);
        a.link.tick();
        b.link.tick();
        return new End[] {a, b};
    }

    void advance(End[] p, long micros) {
        clock.addAndGet(micros);
        for (End e : p) e.link.tick();
    }

    @Test
    void bridgesBroadcastAndUnicastReplyAcross30Blocks() {
        End[] p = pair(MwBand.GHZ_24, 30, 1.2, 20, 0);
        p[0].link.fromCable(eth(BCAST, HOST_A, 1));
        assertEquals(1, p[1].cable.size(), "broadcast crossed the link");
        assertArrayEquals(HOST_A, Arrays.copyOfRange(p[1].cable.get(0), 6, 12));
        p[1].link.fromCable(eth(HOST_A, HOST_B, 2));
        assertEquals(1, p[0].cable.size(), "unicast reply came back");
        assertEquals(2, p[0].cable.get(0)[20]);
        // A frame between two local hosts on A's side stays off the air once both are learned.
        p[0].link.fromCable(eth(BCAST, HOST_A2, 3));
        int before = p[1].cable.size();
        p[0].link.fromCable(eth(HOST_A2, HOST_A, 4));
        assertEquals(before, p[1].cable.size(), "local unicast filtered");
        assertEquals(1L, p[0].link.status().get("filtered_frames"));
    }

    @Test
    void misaimedDishLosesTheLinkAtAtpcPower() {
        End[] good = pair(MwBand.GHZ_24, 30, 1.2, -40, 0);
        good[0].link.fromCable(eth(BCAST, HOST_A, 1));
        assertEquals(1, good[1].cable.size(), "aligned at -40 dBm works");
        stop();
        links.clear();
        End[] bad = pair(MwBand.GHZ_24, 30, 1.2, -40, 30);
        for (int i = 0; i < 20; i++) {
            bad[0].link.fromCable(eth(BCAST, HOST_A, i));
            advance(bad, 1000);
        }
        assertEquals(0, bad[1].cable.size(), "30 degrees off: nothing delivered");
    }

    @Test
    void rainFadeAndOxygenOnA3kmSixtyGhzHop() {
        End[] p = pair(MwBand.GHZ_60, 3000, 1.2, 20, 0);
        // Clear: ~37 dB oxygen, still ~30 dB SINR.
        p[0].link.fromCable(eth(BCAST, HOST_A, 1));
        assertEquals(1, p[1].cable.size(), "clear-sky 60 GHz hop works");
        double clearSinr = p[1].link.lastSinrDb();
        // Thunderstorm at both ends: ~50 dB more, the link is gone.
        for (End e : p) e.link.setRainRate(Atmosphere.rainRateMmPerH(true, true));
        for (int i = 0; i < 20; i++) {
            p[0].link.fromCable(eth(BCAST, HOST_A, 10 + i));
            advance(p, 1000);
        }
        assertEquals(1, p[1].cable.size(), "thunderstorm kills a 3 km 60 GHz hop");
        // Ordinary rain: it survives with less SINR.
        for (End e : p) e.link.setRainRate(Atmosphere.rainRateMmPerH(true, false));
        p[0].link.fromCable(eth(BCAST, HOST_A, 40));
        assertEquals(2, p[1].cable.size(), "rain only fades it");
        double rainSinr = p[1].link.lastSinrDb();
        assertEquals(Atmosphere.rainDb(57.5e9, 10, Math.hypot(3000, 5)), clearSinr - rainSinr, 0.2);
    }

    @Test
    void adaptiveModulationFollowsFeedback() {
        End[] p = pair(MwBand.GHZ_60, 3000, 1.2, 20, 0);
        assertEquals(MwModulation.BPSK, p[0].link.modulationFor(1500 * 8, clock.get()), "no feedback yet");
        for (int i = 0; i < 4; i++) advance(p, MicrowaveLink.BEACON_INTERVAL_US);
        MwModulation clear = p[0].link.modulationFor(1500 * 8, clock.get());
        assertTrue(clear.ordinal() >= MwModulation.QAM64.ordinal(), "clear sky uses a dense constellation: " + clear);
        for (End e : p) e.link.setRainRate(Atmosphere.rainRateMmPerH(true, false));
        for (int i = 0; i < 4; i++) advance(p, MicrowaveLink.BEACON_INTERVAL_US);
        MwModulation rain = p[0].link.modulationFor(1500 * 8, clock.get());
        assertTrue(rain.ordinal() < clear.ordinal(), "rain drops the rate: " + clear + " -> " + rain);
        assertTrue((Double) p[0].link.status().get("rate_mbps") > 1000, "still Gbps class");
        // Feedback goes stale: back to BPSK.
        clock.addAndGet(MicrowaveLink.FEEDBACK_STALE_US + 10_000);
        assertEquals(MwModulation.BPSK, p[0].link.modulationFor(1500 * 8, clock.get()));
    }

    @Test
    void differentChannelDoesNotPair() {
        End[] p = pair(MwBand.GHZ_24, 30, 1.2, 20, 0);
        p[1].link.setChannelNumber(1);
        p[0].link.fromCable(eth(BCAST, HOST_A, 1));
        assertEquals(0, p[1].cable.size());
    }

    @Test
    void alignmentPredictionPeaksOnTheFarDish() {
        End[] p = pair(MwBand.GHZ_24, 500, 1.2, 20, 5);
        double[] toA = {p[0].at[0] - p[1].at[0], p[0].at[1] - p[1].at[1], 0};
        double[] best = DishAim.aimForWorld(toA, v -> v);
        float[] q = DishAim.worldQuaternion(best[0], best[1], v -> v);
        Pose aligned = new Pose("overworld", p[1].at[0], p[1].at[1], p[1].at[2], q[0], q[1], q[2], q[3]);
        double on = p[1].link.predictedRxDbm(p[0].link, aligned, medium);
        double off = p[1].link.predictedRxDbm(p[0].link, p[1].link.pose(), medium);
        assertTrue(on - off > 20, "5 degrees off at 24 GHz costs a lot: " + (on - off));
        assertEquals(List.of(p[0].link), p[1].link.sameChannelRadios());
    }
}
