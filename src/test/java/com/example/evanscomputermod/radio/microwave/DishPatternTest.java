package com.example.evanscomputermod.radio.microwave;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.phys.Noise;

import org.junit.jupiter.api.Test;

import java.util.function.UnaryOperator;

/** Dish pattern vs. the formulas, aiming maths (incl. a rotated Sable frame) and the atmosphere numbers. */
public class DishPatternTest {

    static final double[] DIAMETERS = {0.6, 1.2, 2.4};
    static final double[] FREQS = {10.3e9, 24.1e9, 57.5e9};

    @Test
    void boresightGainMatchesFormula() {
        for (double d : DIAMETERS)
            for (double f : FREQS) {
                DishPattern p = new DishPattern(d, f);
                double lambda = 299_792_458.0 / f;
                double formula = 10 * Math.log10(0.55 * Math.pow(Math.PI * d / lambda, 2));
                assertEquals(formula, p.gainDbi(0, 0, 1), 0.5, d + " m at " + f);
                assertEquals(formula, p.peakGainDbi(), 1e-9);
            }
        // Spot value: 0.6 m at 10.3 GHz is about 34 dBi.
        assertEquals(34.0, new DishPattern(0.6, 10.3e9).peakGainDbi(), 0.5);
    }

    /** Off-axis angle where the gain first falls 3 dB, by bisection. */
    static double halfPowerDeg(DishPattern p) {
        double g0 = p.peakGainDbi(), lo = 0, hi = Math.toRadians(p.beamwidthDeg());
        while (p.gainOffAxisDbi(hi) > g0 - 3) hi *= 1.5;
        for (int i = 0; i < 60; i++) {
            double mid = (lo + hi) / 2;
            if (p.gainOffAxisDbi(mid) > g0 - 3) lo = mid;
            else hi = mid;
        }
        return Math.toDegrees(lo);
    }

    @Test
    void halfPowerBeamwidthIs70LambdaOverD() {
        for (double d : DIAMETERS)
            for (double f : FREQS) {
                DishPattern p = new DishPattern(d, f);
                double rule = 70 * (299_792_458.0 / f) / d;
                double measured = 2 * halfPowerDeg(p);
                assertEquals(rule, measured, 0.2 * rule, d + " m at " + f);
            }
    }

    @Test
    void misaimedByTwoBeamwidthsLosesOver20Db() {
        for (double d : DIAMETERS)
            for (double f : FREQS) {
                DishPattern p = new DishPattern(d, f);
                double loss = p.peakGainDbi() - p.gainOffAxisDbi(Math.toRadians(2 * p.beamwidthDeg()));
                assertTrue(loss > 20, d + " m at " + f + " lost only " + loss + " dB");
            }
    }

    @Test
    void sidelobesAndBackAreWellDown() {
        for (double d : DIAMETERS)
            for (double f : FREQS) {
                DishPattern p = new DishPattern(d, f);
                double g0 = p.peakGainDbi(), bw = Math.toRadians(p.beamwidthDeg());
                double worst = Double.NEGATIVE_INFINITY;
                for (double t = 1.5 * bw; t < Math.PI; t += bw / 20) worst = Math.max(worst, p.gainOffAxisDbi(t));
                assertTrue(worst < g0 - 18, "sidelobe " + (worst - g0) + " dB for " + d + " m at " + f);
                assertTrue(p.gainDbi(0, 0, -1) < g0 - 40, "front-to-back for " + d + " m at " + f);
            }
    }

    @Test
    void aimRoundTripsAndPoseRotatesBoresight() {
        for (double yaw = -170; yaw <= 180; yaw += 37)
            for (double el = -80; el <= 80; el += 23) {
                double[] a = DishAim.aimOf(DishAim.direction(yaw, el));
                assertEquals(DishAim.normalizeYaw(yaw), a[0], 1e-6);
                assertEquals(el, a[1], 1e-6);
                float[] q = DishAim.worldQuaternion(yaw, el, v -> v);
                Pose pose = new Pose("overworld", 0, 0, 0, q[0], q[1], q[2], q[3]);
                double[] bore = pose.toWorld(0, 0, 1), dir = DishAim.direction(yaw, el);
                assertTrue(DishAim.angleDeg(bore, dir) < 1e-3, "boresight at yaw " + yaw + " el " + el);
                assertTrue(pose.toWorld(0, 1, 0)[1] > -1e-6, "dish up points skywards");
            }
        // Minecraft yaw convention: 0 south (+Z), 90 west (-X), 180 north, -90 east.
        assertArrayEquals(new double[] {0, 0, 1}, DishAim.direction(0, 0), 1e-9);
        assertArrayEquals(new double[] {-1, 0, 0}, DishAim.direction(90, 0), 1e-9);
        assertArrayEquals(new double[] {1, 0, 0}, DishAim.direction(-90, 0), 1e-9);
    }

    /** A ship (Sable sub-level) turning 30 degrees drags the beam off the far dish; re-aiming through the frame restores it. */
    @Test
    void shipRotationLosesAlignmentAndReaimHoldsIt() {
        DishPattern p = new DishPattern(1.2, 24.1e9);
        double[] target = {500, 20, 300};
        double[] aim = DishAim.aimForWorld(target, v -> v);
        double turn = Math.toRadians(30);
        UnaryOperator<double[]> ship = v -> new double[] {
                v[0] * Math.cos(turn) + v[2] * Math.sin(turn), v[1], -v[0] * Math.sin(turn) + v[2] * Math.cos(turn)};
        double aligned = gainTowards(p, aim, v -> v, target);
        double dragged = gainTowards(p, aim, ship, target);
        double[] reaim = DishAim.aimForWorld(target, ship);
        double held = gainTowards(p, reaim, ship, target);
        assertEquals(p.peakGainDbi(), aligned, 0.01);
        assertTrue(aligned - dragged > 40, "turning ship should lose the link, lost " + (aligned - dragged));
        assertEquals(p.peakGainDbi(), held, 0.01);
        assertTrue(Math.abs(DishAim.normalizeYaw(reaim[0] - aim[0])) > 29, "re-aim compensates the ship's yaw");
    }

    static double gainTowards(DishPattern p, double[] aim, UnaryOperator<double[]> frame, double[] target) {
        float[] q = DishAim.worldQuaternion(aim[0], aim[1], frame);
        Pose pose = new Pose("overworld", 0, 0, 0, q[0], q[1], q[2], q[3]);
        double n = Math.sqrt(target[0] * target[0] + target[1] * target[1] + target[2] * target[2]);
        double[] l = pose.toLocal(target[0] / n, target[1] / n, target[2] / n);
        return p.gainDbi(l[0], l[1], l[2]);
    }

    @Test
    void atmosphereNumbers() {
        assertEquals(15.0, Atmosphere.gasDbPerKm(60e9), 0.5);
        assertTrue(Atmosphere.gasDbPerKm(57.5e9) > 10, "57.5 GHz oxygen");
        assertTrue(Atmosphere.gasDbPerKm(10.3e9) < 0.05, "10 GHz is nearly clear");
        assertTrue(Atmosphere.gasDbPerKm(24.1e9) < 0.3, "24 GHz water-vapour line is small");
        // Rain: P.838 at 24 GHz, 10 mm/h is about 1-2 dB/km, a thunderstorm several times that.
        double rain24 = Atmosphere.rainDb(24.1e9, 10, 1000), storm24 = Atmosphere.rainDb(24.1e9, 50, 1000);
        assertEquals(Noise.rainFadeDb(24.1e9, 10, 1000), rain24, 1e-12);
        assertTrue(rain24 > 0.8 && rain24 < 2.5, "24 GHz rain " + rain24);
        assertTrue(storm24 > 3 * rain24, "storm " + storm24);
        assertTrue(Atmosphere.rainDb(60e9, 50, 1000) > 12, "60 GHz storm");
        assertEquals(0, Atmosphere.rainDb(10e9, 50, 1000), "no rain fade at or below 10 GHz");
        assertEquals(0, Atmosphere.rainDb(24e9, 0, 1000));
        assertEquals(Noise.RAIN_RATE_MM_PER_H, Atmosphere.rainRateMmPerH(true, false));
        assertEquals(Noise.THUNDER_RAIN_RATE_MM_PER_H, Atmosphere.rainRateMmPerH(true, true));
        assertEquals(0, Atmosphere.rainRateMmPerH(false, false));
        // A 3 km 60 GHz hop: ~37 dB of oxygen in the clear, ~50 dB more in a storm.
        double clear = Atmosphere.lossDb(57.5e9, 3000, 0), storm = Atmosphere.lossDb(57.5e9, 3000, 50);
        assertEquals(36.6, clear, 1.5);
        assertTrue(storm - clear > 40, "storm adds " + (storm - clear));
    }

    @Test
    void bandsAndModulations() {
        assertEquals(12, MwBand.GHZ_10.channelCount(56));
        assertEquals(2, MwBand.GHZ_24.channelCount(112));
        assertEquals(3, MwBand.GHZ_60.channelCount(1000));
        assertEquals(57.5e9, MwBand.GHZ_60.channel(0, 1000).centerHz(), 1);
        assertThrows(IllegalArgumentException.class, () -> MwBand.GHZ_24.channel(2, 112));
        assertThrows(IllegalArgumentException.class, () -> MwBand.GHZ_10.channel(0, 1000));
        for (MwBand b : MwBand.values())
            for (int w : b.widthsMhz())
                for (int c = 0; c < b.channelCount(w); c++)
                    assertEquals(com.example.evanscomputermod.radio.api.Band.MICROWAVE, b.channel(c, w).band(), b + " " + w + " " + c);
        // Gbps class: 112 MHz at 4096-QAM is over 1 Gbit/s; 1 GHz at 256-QAM several.
        assertTrue(MwModulation.QAM4096.bitRate(112e6) > 1e9);
        assertTrue(MwModulation.QAM256.bitRate(1e9) > 6e9);
        assertEquals(MwModulation.BPSK, MwModulation.choose(Double.NaN, 12000));
        assertEquals(MwModulation.BPSK, MwModulation.choose(0, 12000));
        assertEquals(MwModulation.QAM4096, MwModulation.choose(60, 12000));
        MwModulation prev = MwModulation.BPSK;
        for (double s = 0; s < 60; s += 1) {
            MwModulation m = MwModulation.choose(s, 12000);
            assertTrue(m.ordinal() >= prev.ordinal());
            prev = m;
        }
        for (MwModulation m : MwModulation.values()) assertNotEquals(10.0, m.requiredSinrDb(), "medium knows " + m.id);
    }
}
