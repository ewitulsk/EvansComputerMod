package com.example.evanscomputermod.radio.antenna.solver;

import static com.example.evanscomputermod.radio.antenna.solver.TestAntennas.*;
import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/** Losses, loads, junctions, meshing, hazards outputs, validation, determinism and speed. */
public class AntennaSolverBehaviourTest {
    private static final double F40 = 7.1e6;

    private static AntennaModel hfDipole(double resistivity, double radius) {
        double l = AntennaSolver.C0 / F40 / 2 * 0.95;
        return new AntennaModel(List.of(Wire.of(0, 0, 10, l, 0, 10, radius, resistivity)), Feed.center(0), Ground.NONE);
    }

    @Test
    void lossyIronHasLowerEfficiencyThanCopper() {
        double a = 2.5e-4; // thin wire makes the difference visible
        AntennaResult perfect = AntennaSolver.solve(hfDipole(Wire.PERFECT, a), F40);
        AntennaResult copper = AntennaSolver.solve(hfDipole(Wire.COPPER, a), F40);
        AntennaResult gold = AntennaSolver.solve(hfDipole(Wire.GOLD, a), F40);
        AntennaResult iron = AntennaSolver.solve(hfDipole(Wire.IRON, a), F40);
        System.out.println("40 m dipole η: perfect " + perfect.efficiency() + ", copper " + copper.efficiency() + ", gold "
                + gold.efficiency() + ", iron " + iron.efficiency());
        assertEquals(1, perfect.efficiency(), 1e-9);
        assertTrue(copper.efficiency() < 1 && copper.efficiency() > 0.9, "copper: a few % loss");
        assertTrue(gold.efficiency() < copper.efficiency(), "gold is slightly lossier than copper");
        assertTrue(iron.efficiency() < gold.efficiency(), "iron is the lossiest");
        assertTrue(iron.peakGainDbi() < copper.peakGainDbi(), "loss shows up as gain");
        // R_in splits into radiation and loss resistance; loss scales with √ρ (skin effect).
        assertEquals(iron.feedImpedance().re(), iron.radiationResistance() + iron.lossResistance(), 1e-9);
        double ratio = iron.lossResistance() / copper.lossResistance();
        assertEquals(Math.sqrt(Wire.IRON / Wire.COPPER), ratio, 0.15 * ratio, "skin-effect loss ∝ √ρ");
        assertEquals(1 - iron.efficiency(), iron.lossPowerFraction(), 1e-6);
    }

    @Test
    void seriesLoadAddsToFeedImpedance() {
        Complex bare = AntennaSolver.solve(dipole(0.5, 1e-4, 20), F).feedImpedance();
        var loaded = new AntennaModel(List.of(new Wire(0, 0, -0.25, 0, 0, 0.25, 1e-4, Wire.PERFECT, 20)), Feed.center(0),
                List.of(Load.resistor(0, 0.5, 50)), Ground.NONE);
        AntennaResult r = AntennaSolver.solve(loaded, F);
        assertEquals(bare.re() + 50, r.feedImpedance().re(), 1e-6, "a resistor in series with the feed");
        assertEquals(bare.im(), r.feedImpedance().im(), 1e-6);
        assertEquals(bare.re() / (bare.re() + 50), r.efficiency(), 1e-6, "the resistor burns its share");
        // A loading coil halfway up each arm lowers the resonant frequency (emergent, not special-cased).
        var coil = new AntennaModel(List.of(new Wire(0, 0, -0.2, 0, 0, 0.2, 1e-3, Wire.PERFECT, 40)), Feed.center(0),
                List.of(Load.series(0, 0.25, 0, 300), Load.series(0, 0.75, 0, 300)), Ground.NONE);
        double plain = FrequencySweep.resonantFrequency(dipole(0.4, 1e-3, 40), 0.5 * F, 2 * F, 31).orElseThrow();
        double loadedRes = FrequencySweep.resonantFrequency(coil, 0.5 * F, 2 * F, 31).orElseThrow();
        assertTrue(loadedRes < plain * 0.95, "inductive loading shortens the electrical length: " + loadedRes + " vs " + plain);
    }

    @Test
    void wiresJoinAtSharedEndpointsWithCurrentContinuity() {
        // A dipole drawn as two wires meeting at the feed equals the single wire.
        var split = new AntennaModel(List.of(new Wire(0, 0, -0.25, 0, 0, 0, 1e-4, Wire.PERFECT, 10),
                new Wire(0, 0, 0, 0, 0, 0.25, 1e-4, Wire.PERFECT, 10)), Feed.at(0, 1), Ground.NONE);
        Complex one = AntennaSolver.solve(dipole(0.5, 1e-4, 20), F).feedImpedance();
        Complex two = AntennaSolver.solve(split, F).feedImpedance();
        assertEquals(one.re(), two.re(), 1e-6);
        assertEquals(one.im(), two.im(), 1e-6);
        // Feeding the second wire's start at the same node with the same polarity is the same port.
        var split2 = new AntennaModel(split.wires(), Feed.base(1), Ground.NONE);
        assertEquals(one.re(), AntennaSolver.solve(split2, F).feedImpedance().re(), 1e-6);
        // A top hat (T junction, three wires at one node) loads a vertical: lower resonance than without.
        var b = AntennaModel.builder();
        int mast = b.wire(new Wire(0, 0, 0, 0, 0, 0.15, 1e-3, Wire.PERFECT, 6));
        b.wire(new Wire(0, 0, 0.15, 0.1, 0, 0.15, 1e-3, Wire.PERFECT, 4));
        b.wire(new Wire(0, 0, 0.15, -0.1, 0, 0.15, 1e-3, Wire.PERFECT, 4));
        AntennaModel hat = b.feed(Feed.base(mast)).ground(Ground.PERFECT).build();
        double withHat = FrequencySweep.resonantFrequency(hat, 0.5 * F, 2 * F, 31).orElseThrow();
        double without = FrequencySweep.resonantFrequency(monopole(0.15, 1e-3, 6, Ground.PERFECT), 0.5 * F, 2.5 * F, 41).orElseThrow();
        System.out.println("vertical 0.15λ resonance " + without / F + ", with top hat " + withHat / F);
        assertTrue(withHat < 0.8 * without, "top hat lowers resonance");
        AntennaMesh hatMesh = AntennaMesh.build(hat, F);
        // Unknowns: 1 ground half-basis + 5 mast joints + 2 at the 3-way junction (k − 1) + 3 + 3 arm joints.
        assertEquals(14, hatMesh.basisCount(), "Kirchhoff: one unknown fewer than wires at each junction");
    }

    @Test
    void blockWiresMergeIntoLongRuns() {
        // A 20 m dipole built from 1 m block wires (7 MHz), fed at the joint between blocks 10 and 11.
        var blocks = AntennaModel.builder();
        for (int i = 0; i < 20; i++) blocks.wire(i, 0, 10, i + 1, 0, 10, 0.002, Wire.COPPER);
        AntennaModel model = blocks.feed(Feed.at(9, 1.0)).build();
        AntennaModel single = new AntennaModel(List.of(Wire.of(0, 0, 10, 20, 0, 10, 0.002, Wire.COPPER)), Feed.center(0), Ground.NONE);
        AntennaMesh mesh = AntennaMesh.build(model, F40);
        assertTrue(mesh.segmentCount() < 20, "merged into fewer segments than blocks: " + mesh.segmentCount());
        assertEquals(AntennaMesh.build(single, F40).segmentCount(), mesh.segmentCount());
        Complex a = AntennaSolver.solve(mesh, F40).feedImpedance(), b = AntennaSolver.solve(single, F40).feedImpedance();
        assertEquals(b.re(), a.re(), 1e-6);
        assertEquals(b.im(), a.im(), 1e-6);
        // Different materials do not merge.
        var mixed = AntennaModel.builder();
        mixed.wire(0, 0, 10, 10, 0, 10, 0.002, Wire.COPPER);
        mixed.wire(10, 0, 10, 20, 0, 10, 0.002, Wire.IRON);
        AntennaMesh mixedMesh = AntennaMesh.build(mixed.feed(Feed.at(0, 1)).build(), F40);
        assertTrue(mixedMesh.segmentWire(0) != mixedMesh.segmentWire(mixedMesh.segmentCount() - 1));
    }

    @Test
    void segmentationFollowsWavelength() {
        AntennaModel auto = new AntennaModel(List.of(Wire.of(0, 0, -0.25, 0, 0, 0.25, 1e-4, 0)), Feed.center(0), Ground.NONE);
        AntennaMesh mesh = AntennaMesh.build(auto, F);
        for (int i = 0; i < mesh.segmentCount(); i++)
            assertTrue(mesh.segmentLength(i) <= 1.0 / AntennaMesh.MIN_SEGMENTS_PER_WAVELENGTH + 1e-12, "segments ≤ λ/10");
        assertTrue(AntennaMesh.build(auto, 4 * F).segmentCount() > mesh.segmentCount(), "higher frequency, finer mesh");
        // A long wire that would exceed the cap at λ/20 falls back to λ/10, then gives up.
        AntennaModel longWire = new AntennaModel(List.of(Wire.of(0, 0, -7.5, 0, 0, 7.5, 1e-3, 0)), Feed.center(0), Ground.NONE);
        assertTrue(AntennaMesh.build(longWire, F).segmentCount() <= AntennaMesh.MAX_SEGMENTS);
        AntennaModel tooLong = new AntennaModel(List.of(Wire.of(0, 0, -15, 0, 0, 15, 1e-3, 0)), Feed.center(0), Ground.NONE);
        assertThrows(AntennaMesh.SegmentCapException.class, () -> AntennaMesh.build(tooLong, F));
        assertEquals(-1, AntennaMesh.estimateSegments(tooLong, F));
    }

    @Test
    void currentsAndEndVoltagesForHazards() {
        AntennaResult half = AntennaSolver.solve(dipole(0.5, 1e-5, 40), F);
        double[] current = half.segmentCurrentPerWatt();
        double feedCurrent = Math.sqrt(2 / half.feedImpedance().re());
        System.out.println("λ/2 dipole at 1 W: I_feed " + feedCurrent + " A, peak " + half.peakCurrentPerWatt() + " A, end "
                + half.peakEndVoltagePerWatt() + " V; segment currents " + Arrays.toString(Arrays.copyOf(current, 3)) + "...");
        assertEquals(feedCurrent, half.peakCurrentPerWatt(), feedCurrent * 0.01, "current peaks at the centre feed");
        assertTrue(current[0] < 0.1 * feedCurrent, "current falls to ~0 at the open ends");
        // Transmission-line estimate of a λ/2 dipole's end voltage: I_feed·Z0/2, Z0 = 120(ln(L/a) − 1) ≈ 1180 Ω.
        double lineEstimate = feedCurrent * 120 * (Math.log(0.5 / 1e-5) - 1) / 2;
        assertEquals(lineEstimate, half.peakEndVoltagePerWatt(), lineEstimate * 0.35, "end voltage per watt");
        assertEquals(half.peakEndVoltagePerWatt() * 10, half.peakEndVoltage(100), 1e-9, "V ∝ √P");
        AntennaResult shortWhip = AntennaSolver.solve(dipole(0.1, 1e-5, 20), F);
        System.out.println("0.1λ dipole end voltage at 1 W: " + shortWhip.peakEndVoltagePerWatt() + " V");
        assertTrue(shortWhip.peakEndVoltagePerWatt() > 5 * half.peakEndVoltagePerWatt(), "short antennas have huge end voltages");
        assertTrue(shortWhip.peakCurrentPerWatt() > 3 * half.peakCurrentPerWatt(), "and big feed currents");
    }

    @Test
    void realGroundSitsBetweenFreeSpaceAndPerfect() {
        double h = AntennaSolver.C0 / F40 / 4 * 0.97;
        AntennaResult[] r = new AntennaResult[3];
        Ground[] grounds = {Ground.PERFECT, Ground.SEA_WATER, Ground.AVERAGE};
        for (int i = 0; i < 3; i++)
            r[i] = AntennaSolver.solve(new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, h, 0.01, Wire.COPPER)), Feed.base(0), grounds[i]), F40);
        System.out.println("40 m vertical gain: perfect " + r[0].peakGainDbi() + " / sea " + r[1].peakGainDbi() + " / average "
                + r[2].peakGainDbi() + " dBi at θ=" + r[2].peakThetaDeg());
        assertEquals(5.15, r[0].peakGainDbi(), 0.3);
        assertTrue(r[0].peakGainDbi() > r[1].peakGainDbi() && r[1].peakGainDbi() > r[2].peakGainDbi());
        assertTrue(r[2].peakThetaDeg() < 90, "real ground lifts the vertical's beam off the horizon");
        assertEquals(0, r[2].pattern().gain(18, 0), 1e-12, "vertical polarisation vanishes at grazing over real ground");
        // The no-pattern path gives the same impedance and (coarsely integrated) efficiency.
        AntennaResult fast = AntennaSolver.solve(AntennaMesh.build(new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, h, 0.01, Wire.COPPER)),
                Feed.base(0), Ground.AVERAGE), F40), F40, false);
        assertNull(fast.pattern());
        assertTrue(Double.isNaN(fast.peakGainDbi()));
        assertEquals(r[2].feedImpedance().re(), fast.feedImpedance().re(), 1e-9);
        assertEquals(r[2].efficiency(), fast.efficiency(), 0.05);
    }

    @Test
    void invalidGeometryIsRejected() {
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(), Feed.center(0), Ground.NONE), "no wires");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(1, 1, 1, 1, 1, 1, 1e-3, 0)), Feed.center(0), Ground.NONE), "zero length");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, 1, 0, 0)), Feed.center(0), Ground.NONE), "zero radius");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, 1, 0.6, 0)), Feed.center(0), Ground.NONE), "too thick");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, Double.NaN, 1e-3, 0)), Feed.center(0), Ground.NONE), "NaN");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, 1, 1e-3, -1)), Feed.center(0), Ground.NONE), "negative resistivity");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, 0, 0, 0, 1, 1e-3, 0)), Feed.center(3), Ground.NONE), "feed on missing wire");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, -1, 0, 0, 1, 1e-3, 0)), Feed.center(0), Ground.PERFECT), "below ground");
        assertThrows(AntennaGeometryException.class, () -> new AntennaModel(List.of(Wire.of(0, 0, 0, 1, 0, 0, 1e-3, 0)), Feed.center(0), Ground.PERFECT), "lying on the ground");
        assertThrows(AntennaGeometryException.class, () -> new Feed(0, 1.5, Complex.ONE));
        assertThrows(AntennaGeometryException.class, () -> new Feed(0, 0.5, Complex.ZERO));
        assertThrows(AntennaGeometryException.class, () -> new Wire(0, 0, 0, 0, 0, 1, 1e-3, 0, -2));
        // Feeding an open end has nothing to drive.
        assertThrows(AntennaGeometryException.class, () -> AntennaSolver.solve(new AntennaModel(List.of(Wire.of(0, 0, -0.25, 0, 0, 0.25, 1e-4, 0)),
                Feed.base(0), Ground.NONE), F));
        assertThrows(AntennaGeometryException.class, () -> AntennaSolver.solve(dipole(0.5, 1e-4, 20), 0));
        assertThrows(AntennaGeometryException.class, () -> AntennaSolver.solve(dipole(0.5, 1e-4, AntennaMesh.MAX_SEGMENTS + 1), F));
        // Control: the same wire fed in the middle solves.
        assertTrue(AntennaSolver.solve(dipole(0.5, 1e-4, 20), F).feedImpedance().re() > 0);
    }

    @Test
    void deterministicAndThreadSafe() throws Exception {
        AntennaModel model = yagi();
        AntennaResult a = AntennaSolver.solve(model, F), b = AntennaSolver.solve(model, F);
        assertEquals(a.feedImpedance(), b.feedImpedance(), "bit-identical impedance");
        assertArrayEquals(a.segmentCurrentPerWatt(), b.segmentCurrentPerWatt());
        AntennaMesh mesh = AntennaMesh.build(model, F);
        ExecutorService pool = Executors.newFixedThreadPool(4);
        try {
            List<Future<AntennaResult>> futures = new ArrayList<>();
            for (int i = 0; i < 8; i++) futures.add(pool.submit(() -> AntennaSolver.solve(mesh, F)));
            for (Future<AntennaResult> f : futures) {
                AntennaResult r = f.get();
                assertEquals(a.feedImpedance(), r.feedImpedance());
                assertEquals(a.peakGainDbi(), r.peakGainDbi());
                assertEquals(a.peakEndVoltagePerWatt(), r.peakEndVoltagePerWatt());
            }
        } finally {
            pool.shutdownNow();
        }
    }

    @Test
    void twoHundredSegmentsSolveFast() {
        AntennaModel model = new AntennaModel(List.of(new Wire(0, 0, -2.5, 0, 0, 2.5, 1e-3, Wire.COPPER, AntennaMesh.MAX_SEGMENTS)),
                Feed.center(0), Ground.NONE);
        AntennaMesh mesh = AntennaMesh.build(model, F);
        assertEquals(AntennaMesh.MAX_SEGMENTS, mesh.segmentCount());
        for (int i = 0; i < 3; i++) AntennaSolver.solve(mesh, F); // warm up the JIT
        long[] full = new long[5], bare = new long[5];
        for (int i = 0; i < 5; i++) {
            long t0 = System.nanoTime();
            AntennaSolver.solve(mesh, F, true);
            long t1 = System.nanoTime();
            AntennaSolver.solve(mesh, F, false);
            full[i] = t1 - t0;
            bare[i] = System.nanoTime() - t1;
        }
        Arrays.sort(full);
        Arrays.sort(bare);
        double fullMs = full[2] / 1e6, bareMs = bare[2] / 1e6;
        System.out.printf("N=200 solve: %.1f ms with 5° pattern, %.1f ms without (median of 5)%n", fullMs, bareMs);
        assertTrue(fullMs < 100, "N=200 with pattern under 100 ms, took " + fullMs);
        // Over ground the image doubles the matrix fill.
        AntennaMesh grounded = AntennaMesh.build(new AntennaModel(List.of(new Wire(0, 0, 0, 0, 0, 5, 1e-3, Wire.COPPER, AntennaMesh.MAX_SEGMENTS)),
                Feed.base(0), Ground.AVERAGE), F);
        AntennaSolver.solve(grounded, F);
        long t0 = System.nanoTime();
        AntennaSolver.solve(grounded, F);
        double groundMs = (System.nanoTime() - t0) / 1e6;
        System.out.printf("N=200 over real ground: %.1f ms%n", groundMs);
        assertTrue(groundMs < 200, "N=200 over real ground under 200 ms, took " + groundMs);
    }
}
