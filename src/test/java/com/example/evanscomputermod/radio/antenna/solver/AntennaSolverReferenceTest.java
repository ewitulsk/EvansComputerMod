package com.example.evanscomputermod.radio.antenna.solver;

import static com.example.evanscomputermod.radio.antenna.solver.TestAntennas.*;
import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

/**
 * The solver against published reference numbers (Balanis, Antenna Theory; Kraus; NEC-2 examples).
 * Finite-radius MoM differs a little from the infinitely-thin induced-EMF values (73.1 + j42.5 Ω at λ/2):
 * resistance rises with wire radius, which the tests below also check converges to the thin-wire limit.
 */
public class AntennaSolverReferenceTest {
    @Test
    void halfWaveDipoleIsAbout73PlusJ42() {
        AntennaResult r = AntennaSolver.solve(dipole(0.5, 1e-6, 41), F);
        Complex z = r.feedImpedance();
        System.out.println("half-wave dipole a=1e-6λ: Z=" + z + " gain=" + r.peakGainDbi() + " dBi");
        assertEquals(73.1, z.re(), 5.0, "R at λ/2 (±5 Ω)");
        assertEquals(42.5, z.im(), 4.0, "X at λ/2 (±4 Ω)");
        assertEquals(2.15, r.peakGainDbi(), 0.3, "gain (±0.3 dB)");
        assertEquals(90, r.peakThetaDeg(), 1e-9, "broadside peak");
        assertEquals(1.0, r.efficiency(), 1e-9, "perfect conductor is lossless");
    }

    @Test
    void thinWireLimitApproachesInducedEmf() {
        double thin = AntennaSolver.solve(dipole(0.5, 1e-9, 41), F).feedImpedance().re();
        double thick = AntennaSolver.solve(dipole(0.5, 1e-3, 41), F).feedImpedance().re();
        System.out.println("R(λ/2): a=1e-9λ " + thin + ", a=1e-3λ " + thick);
        assertTrue(thin < thick, "thicker wire raises R at λ/2");
        assertEquals(73.1, thin, 3.0, "thin-wire limit");
    }

    @Test
    void resonantDipoleIsSlightlyShorterThanHalfWaveAndMatches72Ohms() {
        double a = 1e-3; // typical thickness: resonance near 0.475λ
        double lo = 0.44, hi = 0.5;
        for (int i = 0; i < 40; i++) {
            double mid = (lo + hi) / 2;
            if (AntennaSolver.solve(AntennaMesh.build(dipole(mid, a, 41), F), F, false).feedImpedance().im() < 0) lo = mid; else hi = mid;
        }
        double length = (lo + hi) / 2;
        AntennaResult r = AntennaSolver.solve(dipole(length, a, 41), F);
        System.out.println("resonant length " + length + "λ, Z=" + r.feedImpedance() + ", SWR(72)=" + r.swr(72) + ", SWR(50)=" + r.swr());
        assertTrue(length > 0.46 && length < 0.49, "resonant length 0.46–0.49λ, got " + length);
        assertEquals(0, r.feedImpedance().im(), 0.5);
        assertEquals(1.0, r.swr(72), 0.06, "SWR against 72 Ω");
        assertEquals(1.44, r.swr(50), 0.1, "SWR against 50 Ω ≈ 72/50");
    }

    @Test
    void quarterWaveMonopoleOverPerfectGroundIsHalfTheDipole() {
        AntennaResult mono = AntennaSolver.solve(monopole(0.25, 1e-6, 20, Ground.PERFECT), F);
        AntennaResult dip = AntennaSolver.solve(dipole(0.5, 1e-6, 40), F);
        Complex z = mono.feedImpedance();
        System.out.println("monopole Z=" + z + " gain=" + mono.peakGainDbi() + " dBi at θ=" + mono.peakThetaDeg());
        assertEquals(36.5, z.re(), 3.0, "R ≈ 36 Ω");
        assertEquals(dip.feedImpedance().re() / 2, z.re(), dip.feedImpedance().re() * 0.01, "image theory: Z_mono = Z_dip / 2");
        assertEquals(dip.feedImpedance().im() / 2, z.im(), 1.0);
        assertEquals(5.15, mono.peakGainDbi(), 0.3, "gain = dipole + 3 dB");
        assertEquals(90, mono.peakThetaDeg(), 1e-9, "peak at the horizon");
        assertEquals(0, mono.pattern().gain(GainPattern.THETA_COUNT - 1, 0), 0, "nothing below ground");
    }

    @Test
    void shortDipoleRadiationResistance() {
        for (double l : new double[] {0.02, 0.05, 0.1}) {
            AntennaResult r = AntennaSolver.solve(dipole(l, 1e-4 * l, 20), F);
            double expected = 20 * Math.PI * Math.PI * l * l;
            System.out.println("short dipole " + l + "λ: R=" + r.feedImpedance().re() + " expected " + expected);
            assertEquals(expected, r.feedImpedance().re(), expected * 0.07, "20π²(L/λ)² within 7%");
            assertTrue(r.feedImpedance().im() < -500, "short dipole is strongly capacitive");
            assertEquals(1.76, r.peakGainDbi(), 0.1, "Hertzian dipole directivity 1.5");
        }
    }

    @Test
    void fullWaveLoopResonance() {
        double prevX = Double.NaN, prevP = 0, resonance = Double.NaN, r = 0;
        for (double p = 0.9; p <= 1.2; p += 0.01) {
            Complex z = AntennaSolver.solve(AntennaMesh.build(squareLoop(p, 1e-4, 10), F), F, false).feedImpedance();
            if (prevX < 0 && z.im() >= 0) {
                resonance = prevP + (p - prevP) * (-prevX) / (z.im() - prevX);
                r = z.re();
                break;
            }
            prevX = z.im();
            prevP = p;
        }
        AntennaResult res = AntennaSolver.solve(squareLoop(resonance, 1e-4, 10), F);
        System.out.println("square loop resonant at perimeter " + resonance + "λ, R≈" + r + ", gain " + res.peakGainDbi() + " dBi");
        assertTrue(resonance > 1.0 && resonance < 1.12, "resonant perimeter slightly above 1λ, got " + resonance);
        assertTrue(r > 100 && r < 150, "R at resonance 100–150 Ω, got " + r);
        assertEquals(3.2, res.peakGainDbi(), 0.4, "full-wave loop ≈ 3.1–3.3 dBi broadside");
        assertEquals(90, res.peakPhiDeg() % 180, 1e-9, "beam is normal to the loop plane (±y)");
    }

    @Test
    void threeElementYagi() {
        AntennaResult r = AntennaSolver.solve(yagi(), F);
        GainPattern p = r.pattern();
        double forward = p.totalDbi(18, 0), back = p.totalDbi(18, 36);
        System.out.println("Yagi Z=" + r.feedImpedance() + " forward=" + forward + " dBi back=" + back + " F/B=" + (forward - back));
        assertEquals(90, r.peakThetaDeg(), 1e-9);
        assertEquals(0, r.peakPhiDeg(), 1e-9, "beam towards the director (+x)");
        assertTrue(forward > 7.0 && forward < 8.8, "forward gain 7–8.8 dBi, got " + forward);
        assertTrue(forward - back > 10, "front-to-back > 10 dB, got " + (forward - back));
        assertTrue(r.feedImpedance().re() > 15 && r.feedImpedance().re() < 45, "Yagi driven element R drops to 15–45 Ω");
    }

    @Test
    void radiatedPowerMatchesInputPower() {
        // Galerkin MoM is power-consistent: the far field integrates to the input power when lossless.
        assertEquals(1.0, integratedGain(AntennaSolver.solve(dipole(0.5, 1e-5, 41), F).pattern(), false), 0.01);
        assertEquals(1.0, integratedGain(AntennaSolver.solve(yagi(), F).pattern(), false), 0.01);
        assertEquals(1.0, integratedGain(AntennaSolver.solve(monopole(0.25, 1e-5, 20, Ground.PERFECT), F).pattern(), true), 0.01);
        assertEquals(1.0, integratedGain(AntennaSolver.solve(squareLoop(1.05, 1e-4, 10), F).pattern(), false), 0.01);
    }

    @Test
    void horizontalDipoleOverGround() {
        // λ/4 high horizontal dipole: perfect ground gives ≈ 7.5 dBi straight up (Kraus); real ground less.
        var wire = new Wire(-0.24, 0, 0.25, 0.24, 0, 0.25, 1e-3, Wire.PERFECT, 21);
        AntennaResult pec = AntennaSolver.solve(new AntennaModel(java.util.List.of(wire), Feed.center(0), Ground.PERFECT), F);
        AntennaResult avg = AntennaSolver.solve(new AntennaModel(java.util.List.of(wire), Feed.center(0), Ground.AVERAGE), F);
        AntennaResult sea = AntennaSolver.solve(new AntennaModel(java.util.List.of(wire), Feed.center(0), Ground.SEA_WATER), F);
        System.out.println("hdipole λ/4 high: PEC " + pec.peakGainDbi() + " dBi, sea " + sea.peakGainDbi() + ", average " + avg.peakGainDbi()
                + " (η " + avg.efficiency() + ")");
        assertEquals(7.5, pec.peakGainDbi(), 0.5);
        assertEquals(0, pec.peakThetaDeg(), 1e-9, "zenith peak at λ/4 height");
        assertTrue(sea.peakGainDbi() < pec.peakGainDbi() && avg.peakGainDbi() < sea.peakGainDbi(), "worse ground, less gain");
        assertTrue(avg.efficiency() < 0.95 && avg.efficiency() > 0.4, "real ground absorbs part of the downward wave");
        assertTrue(pec.pattern().verticalFraction(9, 18) < 0.01, "horizontal wire is horizontally polarised");
    }
}
