package com.example.evanscomputermod.radio.antenna.solver;

import static com.example.evanscomputermod.radio.antenna.solver.TestAntennas.*;
import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

import java.util.List;

public class FrequencySweepTest {
    @Test
    void sweepFindsDipoleResonanceAndBandwidth() {
        AntennaModel model = dipole(0.4757, 1e-3, 31); // resonant at F for this radius
        List<AntennaResult> sweep = FrequencySweep.sweep(model, 0.8 * F, 1.2 * F, 21);
        assertEquals(21, sweep.size());
        assertEquals(0.8 * F, sweep.get(0).frequencyHz(), 1e-6);
        assertEquals(1.2 * F, sweep.get(20).frequencyHz(), 1e-6);
        assertNull(sweep.get(0).pattern(), "sweeps skip the pattern by default");
        assertTrue(sweep.get(0).feedImpedance().im() < 0 && sweep.get(20).feedImpedance().im() > 0, "capacitive below, inductive above");

        double res = FrequencySweep.resonantFrequency(model, 0.8 * F, 1.2 * F, 21).orElseThrow();
        System.out.println("resonance " + res / F + " F");
        assertEquals(F, res, 0.01 * F);

        FrequencySweep.Band band = FrequencySweep.swrBandwidth(model, 0.8 * F, 1.2 * F, 41, 72, 2).orElseThrow();
        System.out.printf("2:1 SWR (72 Ω) band %.4f–%.4f F (%.1f%%)%n", band.lowHz() / F, band.highHz() / F, 100 * band.widthHz() / F);
        assertTrue(band.lowHz() < res && band.highHz() > res);
        assertTrue(band.widthHz() > 0.03 * F && band.widthHz() < 0.25 * F, "a λ/2 dipole has a 2:1 bandwidth of several %");
        // Edges really are at SWR 2.
        assertEquals(2, AntennaSolver.solve(AntennaMesh.build(model, 1.2 * F), band.lowHz(), false).swr(72), 0.01);
        assertEquals(2, AntennaSolver.solve(AntennaMesh.build(model, 1.2 * F), band.highHz(), false).swr(72), 0.01);
        assertTrue(FrequencySweep.swr2to1Bandwidth(model, 0.8 * F, 1.2 * F, 41).isPresent(), "72 Ω dipole is under 2:1 on 50 Ω");
    }

    @Test
    void hfBlockWireDipoleResonatesWhereExpected() {
        // 20 m of copper block wire 10 m up over average ground: a 40 m band (7 MHz) dipole.
        var b = AntennaModel.builder();
        for (int i = 0; i < 20; i++) b.wire(i, 0, 10, i + 1, 0, 10, 0.002, Wire.COPPER);
        AntennaModel model = b.feed(Feed.at(9, 1.0)).ground(Ground.AVERAGE).build();
        double res = FrequencySweep.resonantFrequency(model, 5e6, 9e6, 21).orElseThrow();
        System.out.println("20 m block dipole resonance " + res / 1e6 + " MHz, 2:1 band " + FrequencySweep.swr2to1Bandwidth(model, 5e6, 9e6, 41));
        assertTrue(res > 6.9e6 && res < 7.4e6, "≈ 0.96 × 7.49 MHz");
    }

    @Test
    void noResonanceOrMatchGivesEmpty() {
        AntennaModel tiny = dipole(0.05, 1e-4, 10);
        assertTrue(FrequencySweep.resonantFrequency(tiny, 0.5 * F, 1.5 * F, 11).isEmpty());
        assertTrue(FrequencySweep.swr2to1Bandwidth(tiny, 0.5 * F, 1.5 * F, 11).isEmpty());
        assertThrows(IllegalArgumentException.class, () -> FrequencySweep.sweep(tiny, F, 0.5 * F, 3));
    }
}
