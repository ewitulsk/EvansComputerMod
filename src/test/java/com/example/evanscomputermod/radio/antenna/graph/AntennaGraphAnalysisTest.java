package com.example.evanscomputermod.radio.antenna.graph;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.antenna.solver.Ground;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import org.junit.jupiter.api.Test;

/**
 * The graph → model → analysis path the antenna cache runs, on graphs built
 * the way the world walker builds them (block wires as centre-to-face half
 * edges, 1 m per block, Minecraft y-up coordinates).
 */
public class AntennaGraphAnalysisTest {
    static final double C = 299_792_458.0;

    /** A horizontal dipole along x: feed point at (0.5, h, 0.5), {@code arm} blocks each side. */
    static AntennaGraph.Builder dipole(int arm, double height, ConductorSpec spec, Ground ground) {
        double y = height + 0.5;
        var b = AntennaGraph.builder(new AntennaGraph.Point(0, y, 0.5), new AntennaGraph.Point(1, y, 0.5));
        AntennaGraph.blockRun(b, new AntennaGraph.Point(0, y, 0.5), -1, 0, 0, arm, spec);
        AntennaGraph.blockRun(b, new AntennaGraph.Point(1, y, 0.5), 1, 0, 0, arm, spec);
        if (ground != null) b.ground(0, ground, "average soil");
        return b;
    }

    @Test
    void blockDipoleResonatesNearItsHalfWave() {
        AntennaGraph g = dipole(10, 10, RfDefaults.COPPER_WIRE, Ground.AVERAGE).build();
        assertEquals(20, g.pathLength(), 1e-9, "2 x 10 blocks + the feed block, centre to centre");
        AntennaReport r = AntennaAnalysis.solve(g);
        System.out.println(r.summary());
        System.out.println(r.details());
        assertEquals(AntennaReport.Status.SOLVED, r.status());
        double halfWave = C / 40;   // 7.49 MHz
        assertTrue(r.resonantHz() > 0.90 * halfWave && r.resonantHz() < 1.0 * halfWave,
                "resonance " + r.resonantHz() + " should sit a few % under the half-wave frequency");
        assertTrue(r.swr() < 2.0, "SWR at resonance " + r.swr());
        assertTrue(r.swrBandLowHz() < r.resonantHz() && r.swrBandHighHz() > r.resonantHz());
        // Over real ground η includes the ground-reflection loss; in free space a copper dipole is ~all radiation.
        assertTrue(r.efficiency() > 0.6, "over average ground: " + r.efficiency());
        assertTrue(AntennaAnalysis.solve(dipole(10, 10, RfDefaults.COPPER_WIRE, null).build()).efficiency() > 0.95);
        // Power: wire limit ~ I² R ≈ 0.85² · 73 ≈ 50 W, named.
        assertTrue(r.wireLimitW() > 30 && r.wireLimitW() < 90, "wire limit " + r.wireLimitW());
        assertEquals("copper wire", r.wireLimitLabel());
        assertEquals("copper wire", r.weakestLink());
        assertEquals("wire_current", r.limitCause());
        // A resonant dipole's current peaks at the feed: the hottest segment is next to it.
        assertTrue(r.weakestLinkAt().distance(new AntennaGraph.Point(0.5, 10.5, 0.5)) < 1.5, "weakest link at " + r.weakestLinkAt());
        assertTrue(r.summary().matches("Resonant at \\d+\\.\\d MHz · 2:1 SWR band \\d+\\.\\d–\\d+\\.\\d MHz · rated \\d+ W \\(copper wire\\) / .*"),
                r.summary());
        // Sweep tables: SWR is read back at resonance and worse off resonance.
        assertEquals(r.swr(), r.swrAt(r.resonantHz()), 0.15);
        assertTrue(r.swrAt(r.resonantHz() * 1.15) > 2 * r.swrAt(r.resonantHz()));
        assertTrue(Double.isInfinite(r.swrAt(5e9)), "far outside the sweeps reads as unusable");
        assertTrue(r.efficiencyAt(14e6) > 0, "HF band swept");
    }

    @Test
    void heavierTierRaisesThePowerRating() {
        AntennaReport thin = AntennaAnalysis.solve(dipole(10, 10, RfDefaults.COPPER_WIRE, Ground.AVERAGE).build());
        AntennaReport thick = AntennaAnalysis.solve(dipole(10, 10, RfDefaults.ANTENNA_WIRE, Ground.AVERAGE).build());
        assertTrue(thick.wireLimitW() > 3 * thin.wireLimitW(), thin.wireLimitW() + " vs " + thick.wireLimitW());
        assertTrue(thick.wireLimitW() > 120 && thick.wireLimitW() < 350, "antenna wire ~200 W: " + thick.wireLimitW());
    }

    @Test
    void touchingMetalExtendsAndDetunes() {
        AntennaReport clean = AntennaAnalysis.solve(dipole(10, 10, RfDefaults.COPPER_WIRE, Ground.AVERAGE).build());
        var b = dipole(10, 10, RfDefaults.COPPER_WIRE, Ground.AVERAGE);
        // An iron block touching the end of the +x arm (last wire block centre at x = 10.5).
        b.edge(new AntennaGraph.Point(10.5, 10.5, 0.5), new AntennaGraph.Point(11, 10.5, 0.5), RfDefaults.COPPER_WIRE, "copper wire");
        b.edge(new AntennaGraph.Point(11, 10.5, 0.5), new AntennaGraph.Point(11.5, 10.5, 0.5), RfDefaults.METAL_BLOCK, "metal block");
        AntennaReport detuned = AntennaAnalysis.solve(b.build());
        System.out.println("clean " + clean.resonantHz() + " detuned " + detuned.resonantHz());
        assertTrue(detuned.resonantHz() < clean.resonantHz() * 0.98, "touching metal lowers the resonance");
    }

    @Test
    void insulatorsSetTheVoltageLimitAndBareEndsUseCorona() {
        var bare = dipole(10, 10, RfDefaults.ANTENNA_WIRE, Ground.AVERAGE).build();
        var held = dipole(10, 10, RfDefaults.ANTENNA_WIRE, Ground.AVERAGE)
                .insulated(new AntennaGraph.Point(-9.5, 10.5, 0.5), "insulator", RfDefaults.INSULATOR_VOLTS)
                .insulated(new AntennaGraph.Point(10.5, 10.5, 0.5), "insulator", RfDefaults.INSULATOR_VOLTS).build();
        assertEquals(2, AntennaAnalysis.openEnds(bare).size());
        AntennaReport a = AntennaAnalysis.solve(bare), b = AntennaAnalysis.solve(held);
        System.out.println(a.summary() + "\n" + b.summary());
        assertEquals("bare end of antenna wire", a.voltageLimitLabel());
        assertEquals("insulators", b.voltageLimitLabel());
        assertTrue(b.voltageLimitAt().near(new AntennaGraph.Point(-9.5, 10.5, 0.5)) || b.voltageLimitAt().near(new AntennaGraph.Point(10.5, 10.5, 0.5)),
                "insulator position " + b.voltageLimitAt());
        assertTrue(a.voltageLimitAt().distance(new AntennaGraph.Point(0.5, 10.5, 0.5)) > 9, "bare end position " + a.voltageLimitAt());
        assertTrue(b.voltageLimitW() > a.voltageLimitW(), "a 4 kV insulator beats a 2.5 kV bare end");
        assertEquals(AntennaAnalysis.voltageLimitW(RfDefaults.INSULATOR_VOLTS, b.peakEndVoltagePerWatt()), b.voltageLimitW(), 1e-6 * b.voltageLimitW());
    }

    @Test
    void emptyFeedPointIsNoAntenna() {
        var g = AntennaGraph.builder(new AntennaGraph.Point(0, 5, 0), new AntennaGraph.Point(1, 5, 0)).build();
        AntennaReport r = AntennaAnalysis.solve(g);
        assertEquals(AntennaReport.Status.NO_ANTENNA, r.status());
        assertTrue(r.summary().startsWith("No antenna"));
        assertEquals(0, r.powerLimitW());
        assertFalse(r.present());
    }

    @Test
    void groundMountedVerticalIsAMonopole() {
        // Feed point on the ground (bottom face y = 64), 9 rods above it: 10 m above ground.
        var b = AntennaGraph.builder(new AntennaGraph.Point(0.5, 64, 0.5), new AntennaGraph.Point(0.5, 65, 0.5)).monopole(true);
        AntennaGraph.blockRun(b, new AntennaGraph.Point(0.5, 65, 0.5), 0, 1, 0, 9, RfDefaults.ANTENNA_ROD);
        b.ground(64, Ground.PERFECT, "metal");
        AntennaGraph g = b.build();
        assertEquals(19, g.pathLength(), 1e-9, "image doubles the 9.5 m height");
        AntennaReport r = AntennaAnalysis.solve(g);
        System.out.println(r.summary() + "\n" + r.details());
        double quarter = C / (4 * 9.5 + 0.0);
        assertTrue(r.resonantHz() > 0.85 * C / 39 && r.resonantHz() < quarter, "monopole resonance " + r.resonantHz());
        assertTrue(r.feedImpedance().re() > 25 && r.feedImpedance().re() < 50, "λ/4 monopole ≈ 36 Ω: " + r.feedImpedance());
        assertEquals("monopole", r.kind());
    }

    @Test
    void estimateIsAvailableBeforeTheSolve() {
        AntennaReport e = AntennaAnalysis.estimate(dipole(10, 10, RfDefaults.COPPER_WIRE, Ground.AVERAGE).build());
        assertEquals(AntennaReport.Status.ESTIMATE, e.status());
        assertTrue(e.summary().startsWith("≈ Resonant at 7.2 MHz"), e.summary());
        assertTrue(e.wireLimitW() > 30 && e.wireLimitW() < 90);
    }

    @Test
    void patternIsInTheMinecraftFrame() {
        // Free-space dipole along x (east): nulls off the ends, maximum broadside, horizontal polarization.
        AntennaReport r = AntennaAnalysis.solve(dipole(10, 10, RfDefaults.COPPER_WIRE, null).build());
        AntennaPattern p = r.patternAt(r.resonantHz());
        double broadside = p.gainDbi(0, 0, 1), endFire = p.gainDbi(1, 0, 0), up = p.gainDbi(0, 1, 0);
        System.out.printf("broadside %.2f end-fire %.2f up %.2f%n", broadside, endFire, up);
        assertEquals(2.15, broadside, 0.6, "half-wave dipole gain");
        assertTrue(endFire < broadside - 15, "end-fire null");
        assertEquals(broadside, up, 0.5, "symmetric about the wire axis");
        double[] pol = p.polarization(0, 0, 1);
        assertEquals(1, Math.abs(pol[0]), 0.02, "E-field along the wire (x): " + java.util.Arrays.toString(pol));
        double[] s = AntennaModelBuilder.toSolver(1, 2, 3), m = AntennaModelBuilder.toMinecraft(s[0], s[1], s[2]);
        assertArrayEquals(new double[] {1, 2, 3}, m, 1e-12);
        assertEquals(2, s[2], 1e-12, "y up maps to z up");
    }

    @Test
    void oxidationAddsLoss() {
        ConductorSpec clean = RfDefaults.COPPER_WIRE, green = clean.oxidized(3);
        assertEquals(clean.resistivity() * 4, green.resistivity(), 1e-20);
        assertSame(RfDefaults.ANTENNA_ROD, RfDefaults.ANTENNA_ROD.oxidized(3), "aluminium rods don't oxidize");
        // A short (lossy) dipole shows it clearly.
        double e0 = AntennaAnalysis.solve(dipole(3, 10, clean, null).build()).efficiency();
        double e3 = AntennaAnalysis.solve(dipole(3, 10, green, null).build()).efficiency();
        assertTrue(e3 < e0, e0 + " -> " + e3);
    }

    @Test
    void powerLimitFormulas() {
        // 0.85 A rms into a 73 Ω resonant feed: P = I²R ≈ 52.7 W.
        assertEquals(0.85 * 0.85 * 73, AntennaAnalysis.currentLimitW(0.85, Math.sqrt(2 / 73.0)), 1e-9);
        assertEquals(1e6 / 100.0, AntennaAnalysis.voltageLimitW(1000, Math.sqrt(100)), 1e-9);
        assertEquals("1.4 kW", AntennaReport.watts(1400));
        assertEquals("200 W", AntennaReport.watts(200));
        assertEquals("7.1 MHz", AntennaReport.hz(7.1e6));
    }

    @Test
    void coaxLossFollowsTheTwoPointFit() {
        CoaxSpec coax = RfDefaults.COAX_CABLE, hard = RfDefaults.HARDLINE;
        assertEquals(0.49, coax.lossPer10Db(10e6), 1e-9);
        assertEquals(6.6, coax.lossPer10Db(1e9), 1e-9);
        double l100 = coax.lossPer10Db(100e6);
        assertTrue(l100 > 1.2 && l100 < 2.5, "RG-58-like at 100 MHz: " + l100);
        assertTrue(coax.lossDb(450e6, 10) > 3, "several dB per 10 blocks at UHF");
        assertTrue(hard.lossDb(450e6, 10) < 0.3, "hardline barely loses anything");
        assertEquals(2 * coax.lossDb(146e6, 10), coax.lossDb(146e6, 20), 1e-12);
        assertTrue(coax.powerRatingW(1e9) < coax.powerRatingW(10e6));
    }
}
