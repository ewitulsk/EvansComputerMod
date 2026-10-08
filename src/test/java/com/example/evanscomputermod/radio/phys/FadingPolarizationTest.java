package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import java.util.SplittableRandom;

import org.junit.jupiter.api.Test;

public class FadingPolarizationTest {
    @Test
    void deterministicGivenSeed() {
        assertEquals(Fading.rayleighGainDb(1234), Fading.rayleighGainDb(1234));
        assertEquals(Fading.ricianPhasor(5, 99), Fading.ricianPhasor(5, 99));
        assertNotEquals(Fading.rayleighGainDb(1234), Fading.rayleighGainDb(1235));
        SplittableRandom a = new SplittableRandom(7), b = new SplittableRandom(7);
        for(int i = 0; i < 100; i++) assertEquals(Fading.ricianPhasor(3, a), Fading.ricianPhasor(3, b));
        // the dB and phasor paths agree for the same seed
        assertEquals(Fading.ricianPhasor(4, 55L).powerDb(), Fading.ricianGainDb(4, 55L), 1e-9);
    }

    @Test
    void rayleighStatistics() {
        int n = 200_000;
        double sum = 0;
        int below = 0;
        for(int i = 0; i < n; i++) {
            double p = Units.dbToLinear(Fading.rayleighGainDb(i));
            sum += p;
            if(p < 0.1) below++;
        }
        assertEquals(1.0, sum / n, 0.01, "unit mean power");
        // exponential power: P(|h|^2 < 0.1) = 1 - e^-0.1 = 0.0952
        assertEquals(0.0952, below / (double) n, 0.003);
    }

    @Test
    void ricianIsMilderThanRayleigh() {
        int n = 100_000;
        SplittableRandom rng = new SplittableRandom(2024);
        double k = 10, sum = 0, sum2 = 0;
        for(int i = 0; i < n; i++) {
            double p = Fading.ricianPhasor(k, rng).abs2();
            sum += p;
            sum2 += p * p;
        }
        double mean = sum / n, var = sum2 / n - mean * mean;
        assertEquals(1.0, mean, 0.01);
        // Var|h|^2 = (1 + 2K) / (1 + K)^2 = 0.1736 for K = 10 (Rayleigh: 1)
        assertEquals((1 + 2 * k) / ((1 + k) * (1 + k)), var, 0.01);
    }

    @Test
    void coherenceTime() {
        // 2.4 GHz, 5 m/s: fD = 40 Hz, Tc = 0.423 / 40 = 10.6 ms
        assertEquals(0.423 / (5 / Units.wavelengthM(2.4e9)), Fading.coherenceTimeS(5, 2.4e9), 1e-12);
        assertEquals(0.0106, Fading.coherenceTimeS(5, 2.4e9), 0.0002);
        // control: a static link never re-draws
        assertEquals(Double.POSITIVE_INFINITY, Fading.coherenceTimeS(0, 2.4e9));
        assertEquals(Fading.blockSeed(9, 0, Double.POSITIVE_INFINITY), Fading.blockSeed(9, 1e6, Double.POSITIVE_INFINITY));
        double tc = 0.01;
        assertEquals(Fading.blockSeed(9, 0.101, tc), Fading.blockSeed(9, 0.109, tc));
        assertNotEquals(Fading.blockSeed(9, 0.101, tc), Fading.blockSeed(9, 0.111, tc));
        assertNotEquals(Fading.blockSeed(9, 0.101, tc), Fading.blockSeed(10, 0.101, tc));
    }

    @Test
    void polarizationLoss() {
        assertEquals(0, PolarizationLoss.db(0), 1e-12);
        assertEquals(3.0103, PolarizationLoss.db(Math.PI / 4), 1e-3);
        assertEquals(20, PolarizationLoss.db(Math.PI / 2), 1e-12);
        assertEquals(20, PolarizationLoss.db(Polarization.VERTICAL, Polarization.HORIZONTAL), 1e-12);
        assertEquals(0, PolarizationLoss.db(Polarization.VERTICAL, Polarization.VERTICAL), 1e-12);
        assertEquals(3.0103, PolarizationLoss.db(Polarization.VERTICAL, Polarization.CIRCULAR_RIGHT), 1e-3);
        assertEquals(20, PolarizationLoss.db(Polarization.CIRCULAR_LEFT, Polarization.CIRCULAR_RIGHT), 1e-12);
        // a dipole rotated 90 degrees on a Sable ship: full cross-pol
        assertEquals(20, PolarizationLoss.linearDb(Math.PI / 2, Math.PI), 1e-9);
    }

    @Test
    void skywaveScramblesPolarization() {
        int n = 100_000;
        double sum = 0;
        for(long s = 0; s < n; s++) {
            double l = PolarizationLoss.skywaveDb(Polarization.VERTICAL, s);
            assertTrue(l >= 0 && l <= PolarizationLoss.CROSS_POL_CAP_DB);
            sum += Units.dbToLinear(-l);
        }
        assertEquals(0.5, sum / n, 0.01, "random plane: mean cos^2 = 1/2");
        assertEquals(PolarizationLoss.skywaveDb(Polarization.HORIZONTAL, 77), PolarizationLoss.skywaveDb(Polarization.HORIZONTAL, 77));
        assertEquals(PolarizationLoss.LINEAR_TO_CIRCULAR_DB, PolarizationLoss.skywaveDb(Polarization.CIRCULAR_LEFT, 3));
    }
}
