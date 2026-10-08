package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class TwoRayGroundTest {
    private static final double F = 900e6, HT = 10, HR = 2;

    @Test
    void crossoverDistance() {
        // dc = 4 pi ht hr / lambda = 4 pi * 20 / 0.3331 = 754.5 m
        assertEquals(754.5, TwoRay.crossoverDistanceM(F, HT, HR), 0.5);
    }

    @Test
    void fortyDbPerDecadePastCrossover() {
        double dc = TwoRay.crossoverDistanceM(F, HT, HR);
        double a = TwoRay.lossDb(10 * dc, F, HT, HR, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL);
        double b = TwoRay.lossDb(100 * dc, F, HT, HR, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL);
        assertEquals(40, b - a, 1.0);
        // and approaches the plane-earth formula 40 log d - 20 log(ht hr)
        assertEquals(TwoRay.planeEarthLossDb(100 * dc, HT, HR), b, 1.0);
        // vertical polarization over real ground also tends to d^4 far out
        double va = TwoRay.lossDb(100 * dc, F, HT, HR, Ground.AVERAGE_GROUND, Polarization.VERTICAL);
        double vb = TwoRay.lossDb(1000 * dc, F, HT, HR, Ground.AVERAGE_GROUND, Polarization.VERTICAL);
        assertEquals(40, vb - va, 1.5);
    }

    @Test
    void insideCrossoverTracksFreeSpaceWithinLobes() {
        double dc = TwoRay.crossoverDistanceM(F, HT, HR);
        for(double d = 20; d < dc / 2; d += 3.7) {
            double l = TwoRay.lossDb(d, F, HT, HR, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL);
            double fs = FreeSpace.lossDb(Math.hypot(d, HT - HR), F);
            assertTrue(l >= fs - 6.03, "at most +6 dB constructive gain at d=" + d);
        }
        // and far past crossover it is well below free space
        assertTrue(TwoRay.lossDb(50 * dc, F, HT, HR, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL)
                > FreeSpace.lossDb(50 * dc, F) + 20);
    }

    @Test
    void antennaHeightMattersPastCrossover() {
        double d = 20_000;
        double low = TwoRay.lossDb(d, F, 2, 2, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL);
        double high = TwoRay.lossDb(d, F, 20, 2, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL);
        assertEquals(20, low - high, 1.0); // 20 log(10)
    }

    @Test
    void reflectionCoefficients() {
        // grazing: -1 for both polarizations over real ground
        assertEquals(-1, TwoRay.reflectionCoefficient(1e-6, F, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL).re(), 1e-3);
        assertEquals(-1, TwoRay.reflectionCoefficient(1e-6, F, Ground.AVERAGE_GROUND, Polarization.VERTICAL).re(), 1e-3);
        // normal incidence on a near-lossless dielectric: |Gamma| = (sqrt(er)-1)/(sqrt(er)+1)
        double n = Math.sqrt(3);
        assertEquals((n - 1) / (n + 1),
                TwoRay.reflectionCoefficient(Math.PI / 2, 10e9, Ground.DRY_SAND, Polarization.HORIZONTAL).abs(), 1e-3);
        // Brewster angle: tan psi = 1/sqrt(er) -> vertical reflection vanishes
        double brewster = Math.atan(1 / n);
        assertTrue(TwoRay.reflectionCoefficient(brewster, 10e9, Ground.DRY_SAND, Polarization.VERTICAL).abs() < 0.01);
        // perfect conductor
        assertEquals(-1, TwoRay.reflectionCoefficient(0.3, F, Ground.METAL, Polarization.HORIZONTAL).re());
        assertEquals(1, TwoRay.reflectionCoefficient(0.3, F, Ground.METAL, Polarization.VERTICAL).re());
        // passive ground never reflects more than it receives
        for(double psi = 0.01; psi < 1.57; psi += 0.05)
            for(Polarization p : Polarization.values())
                assertTrue(TwoRay.reflectionCoefficient(psi, F, Ground.SEA_WATER, p).abs() <= 1 + 1e-9);
    }

    @Test
    void verticalOverMetalDoublesField() {
        // PEC with vertical polarization: no d^4 cancellation, +6 dB over free space far out
        double d = 1e5;
        double l = TwoRay.lossDb(d, F, HT, HR, Ground.METAL, Polarization.VERTICAL);
        assertEquals(FreeSpace.lossDb(d, F) - 6.02, l, 0.05);
    }

    @Test
    void twoRayControlZeroDistance() {
        assertEquals(0, TwoRay.lossDb(0, F, 2, 2, Ground.AVERAGE_GROUND, Polarization.VERTICAL));
        assertTrue(Double.isFinite(TwoRay.lossDb(1e6, F, 0, 0, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL)));
    }

    @Test
    void groundWaveSeaBeatsLandAndVerticalBeatsHorizontal() {
        double f = 1e6, d = 50_000;
        double sea = GroundWave.lossDb(d, f, Ground.SEA_WATER, Polarization.VERTICAL);
        double land = GroundWave.lossDb(d, f, Ground.DRY_GROUND, Polarization.VERTICAL);
        assertTrue(land > sea + 10, "sea " + sea + " land " + land);
        // over sea at MF the attenuation factor stays near 1 (field doubled vs free space)
        assertEquals(FreeSpace.lossDb(d, f) - 6.02, sea, 1.0);
        double hor = GroundWave.lossDb(d, f, Ground.AVERAGE_GROUND, Polarization.HORIZONTAL);
        assertTrue(hor > GroundWave.lossDb(d, f, Ground.AVERAGE_GROUND, Polarization.VERTICAL) + 40);
        // GHz ground wave dies within metres
        assertTrue(GroundWave.lossDb(100, 2.4e9, Ground.AVERAGE_GROUND, Polarization.VERTICAL)
                > FreeSpace.lossDb(100, 2.4e9) + 30);
    }

    @Test
    void groundWaveControl() {
        assertEquals(0, GroundWave.lossDb(0, 1e6, Ground.AVERAGE_GROUND, Polarization.VERTICAL));
        assertEquals(1.0, GroundWave.attenuationFactor(1e4, 1e6, Ground.METAL, Polarization.VERTICAL), 1e-12);
        // attenuation factor is a monotone falling function of distance
        double prev = 1;
        for(double d = 10; d < 1e6; d *= 1.5) {
            double a = GroundWave.attenuationFactor(d, 7e6, Ground.AVERAGE_GROUND, Polarization.VERTICAL);
            assertTrue(a <= prev + 1e-12 && a > 0);
            prev = a;
        }
    }
}
