package com.example.evanscomputermod.radio.antenna.solver;

import static com.example.evanscomputermod.radio.antenna.solver.TestAntennas.*;
import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

import java.util.List;

public class HeuristicAntennaTest {
    @Test
    void sineAndCosineIntegrals() {
        // Abramowitz & Stegun table 5.1.
        assertEquals(0.946083070, HeuristicAntenna.si(1), 1e-8);
        assertEquals(0.337403923, HeuristicAntenna.ci(1), 1e-8);
        assertEquals(1.658347594, HeuristicAntenna.si(10), 1e-6);
        assertEquals(-0.045456433, HeuristicAntenna.ci(10), 1e-6);
        assertEquals(1.851937052, HeuristicAntenna.si(Math.PI), 1e-8);
        assertEquals(1.548241701, HeuristicAntenna.si(20), 1e-8);
        assertEquals(0.044419820, HeuristicAntenna.ci(20), 1e-8);
        // Series and asymptotic branches agree across the switch-over.
        assertEquals(HeuristicAntenna.si(20), HeuristicAntenna.si(20 + 1e-9), 1e-7);
        assertEquals(HeuristicAntenna.ci(20), HeuristicAntenna.ci(20 + 1e-9), 1e-7);
    }

    @Test
    void halfWaveDipoleAndMonopoleMatchInducedEmf() {
        HeuristicAntenna.Estimate d = HeuristicAntenna.estimate(dipole(0.5, 1e-5, Wire.AUTO), F);
        assertEquals(HeuristicAntenna.Type.DIPOLE, d.type());
        assertEquals(73.1, d.feedImpedance().re(), 0.3);
        assertEquals(42.5, d.feedImpedance().im(), 0.3);
        assertEquals(2.15, d.gainDbi(), 0.05);
        HeuristicAntenna.Estimate m = HeuristicAntenna.estimate(monopole(0.25, 1e-5, Wire.AUTO, Ground.PERFECT), F);
        assertEquals(HeuristicAntenna.Type.MONOPOLE, m.type());
        assertEquals(36.5, m.feedImpedance().re(), 0.3);
        assertEquals(5.15, m.gainDbi(), 0.1);
        HeuristicAntenna.Estimate shortDipole = HeuristicAntenna.estimate(dipole(0.05, 1e-5, Wire.AUTO), F);
        assertEquals(20 * Math.PI * Math.PI * 0.0025, shortDipole.feedImpedance().re(), 0.01);
        assertTrue(shortDipole.feedImpedance().im() < -1000);
    }

    @Test
    void tracksTheSolverOnRealisticAntennas() {
        // 40 m copper dipole: heuristic within 15% of the MoM impedance magnitude and 0.5 dB of its gain.
        double l = AntennaSolver.C0 / 7.1e6 / 2 * 0.96;
        AntennaModel hf = new AntennaModel(List.of(Wire.of(0, 0, 0, l, 0, 0, 0.001, Wire.COPPER)), Feed.center(0), Ground.NONE);
        AntennaResult mom = AntennaSolver.solve(hf, 7.1e6);
        HeuristicAntenna.Estimate est = HeuristicAntenna.estimate(hf, 7.1e6);
        System.out.println("40 m dipole: MoM " + mom.feedImpedance() + " " + mom.peakGainDbi() + " dBi η " + mom.efficiency()
                + "; heuristic " + est);
        assertEquals(mom.feedImpedance().abs(), est.feedImpedance().abs(), 0.15 * mom.feedImpedance().abs());
        assertEquals(mom.peakGainDbi(), est.gainDbi(), 0.5);
        assertEquals(mom.efficiency(), est.efficiency(), 0.03);
        // Full-wave loop.
        AntennaResult loopMom = AntennaSolver.solve(squareLoop(1.05, 1e-4, 10), F);
        HeuristicAntenna.Estimate loopEst = HeuristicAntenna.estimate(squareLoop(1.05, 1e-4, 10), F);
        assertEquals(HeuristicAntenna.Type.LOOP, loopEst.type());
        assertEquals(loopMom.feedImpedance().re(), loopEst.feedImpedance().re(), 0.2 * loopMom.feedImpedance().re());
        assertEquals(loopMom.peakGainDbi(), loopEst.gainDbi(), 0.5);
    }

    @Test
    void classifiesShapes() {
        // Small loop: uniform-current radiation resistance.
        HeuristicAntenna.Estimate small = HeuristicAntenna.estimate(squareLoop(0.1, 1e-3, 4), F);
        assertEquals(HeuristicAntenna.Type.LOOP, small.type());
        double area = 0.025 * 0.025;
        assertEquals(31171 * area * area, small.feedImpedance().re(), 1e-6);
        assertTrue(small.feedImpedance().im() > 0, "small loops are inductive");
        // End-fed 2λ wire.
        HeuristicAntenna.Estimate lw = HeuristicAntenna.estimate(new AntennaModel(List.of(Wire.of(0, 0, 5, 2, 0, 5, 1e-3, 0)),
                Feed.at(0, 0), Ground.NONE), F);
        assertEquals(HeuristicAntenna.Type.LONG_WIRE, lw.type());
        assertTrue(lw.gainDbi() > 3.5 && lw.gainDbi() < 5, "2λ long wire ≈ 4.4 dBi");
        // Inverted V: a bent dipole, slightly less gain.
        var v = new AntennaModel(List.of(Wire.of(-0.2, 0, 0.5, 0, 0, 0.65, 1e-4, 0), Wire.of(0, 0, 0.65, 0.2, 0, 0.5, 1e-4, 0)),
                Feed.at(0, 1), Ground.NONE);
        HeuristicAntenna.Estimate ve = HeuristicAntenna.estimate(v, F);
        assertEquals(HeuristicAntenna.Type.DIPOLE, ve.type());
        // T junction: unknown, falls back to the total length.
        var t = new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, 0.2, 1e-3, 0), Wire.of(0, 0, 0.2, 0.1, 0, 0.2, 1e-3, 0),
                Wire.of(0, 0, 0.2, -0.1, 0, 0.2, 1e-3, 0)), Feed.base(0), Ground.PERFECT);
        assertEquals(HeuristicAntenna.Type.UNKNOWN, HeuristicAntenna.estimate(t, F).type());
        // Parasitic elements are ignored: a Yagi classifies by its driven element.
        assertEquals(HeuristicAntenna.Type.DIPOLE, HeuristicAntenna.estimate(yagi(), F).type());
    }

    @Test
    void worksAboveTheSegmentCap() {
        AntennaModel huge = new AntennaModel(List.of(Wire.of(0, 0, -15, 0, 0, 15, 1e-3, Wire.COPPER)), Feed.center(0), Ground.NONE);
        assertEquals(-1, AntennaMesh.estimateSegments(huge, F));
        HeuristicAntenna.Estimate e = HeuristicAntenna.estimate(huge, F);
        assertTrue(e.feedImpedance().isFinite() && e.efficiency() > 0 && e.efficiency() <= 1);
    }
}
