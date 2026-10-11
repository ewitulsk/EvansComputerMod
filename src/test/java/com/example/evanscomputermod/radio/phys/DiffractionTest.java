package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class DiffractionTest {
    @Test
    void knifeEdgeJvItuPoints() {
        // ITU-R P.526 Fig. 7 / eq. (31)
        assertEquals(6.0, KnifeEdge.lossDb(0), 0.05);
        assertEquals(0.0, KnifeEdge.lossDb(-0.78), 0.05);
        assertEquals(13.9, KnifeEdge.lossDb(1), 0.1);
        assertEquals(20.5, KnifeEdge.lossDb(2.4), 0.1);
        assertEquals(0.54, KnifeEdge.lossDb(-0.7), 0.05);
        // large v: J ~ 13 + 20 log v
        assertEquals(13 + 20 * Math.log10(10), KnifeEdge.lossDb(10), 0.3);
    }

    @Test
    void knifeEdgeMonotonicAndNonNegative() {
        double prev = 0;
        for(double v = -3; v <= 10; v += 0.01) {
            double j = KnifeEdge.lossDb(v);
            assertTrue(j >= 0);
            assertTrue(j >= prev - 1e-12, "J must not decrease at v=" + v);
            prev = j;
        }
    }

    @Test
    void knifeEdgeControlClearPath() {
        assertEquals(0, KnifeEdge.lossDb(-5));
        assertEquals(0, KnifeEdge.lossDb(Double.NEGATIVE_INFINITY));
        assertEquals(Double.NEGATIVE_INFINITY, KnifeEdge.v(-1, 0, 10, 0.1));
    }

    @Test
    void vParameter() {
        // h = 10 m, d1 = d2 = 1 km, lambda = 0.125 m -> v = 10 sqrt(2/0.125 * 0.002) = 1.789
        assertEquals(1.789, KnifeEdge.v(10, 1000, 1000, 0.125), 0.001);
    }

    @Test
    void deygoutSingleEdgeEqualsKnifeEdge() {
        double f = 2.4e9;
        // tx and rx at 0 m ground, 2 m antennas; a 10 m hill at mid-path of 1 km
        TerrainProfile p = TerrainProfile.uniform(1000, 0, 0, 10, 0, 0);
        double expected = KnifeEdge.lossDb(8, 500, 500, f);
        assertEquals(expected, Deygout.lossDb(p, f, 2, 2), 1e-9);
        assertTrue(expected > 15);
    }

    @Test
    void deygoutFrequencyDependence() {
        // HF bends around a hill, 2.4 GHz does not
        TerrainProfile p = TerrainProfile.uniform(1000, 0, 0, 10, 0, 0);
        double hf = Deygout.lossDb(p, 7e6, 2, 2);
        double wifi = Deygout.lossDb(p, 2.4e9, 2, 2);
        assertTrue(hf < 10, "HF loss " + hf);
        assertEquals(KnifeEdge.lossDb(8, 500, 500, 7e6), hf, 1e-9, "flat ground beside the hill is not an edge");
        assertEquals(0, Deygout.lossDb(TerrainProfile.flat(1000, 0, 41), 7e6, 2, 2), "open plain at HF");
        assertTrue(wifi > 15, "2.4 GHz loss " + wifi);
    }

    @Test
    void deygoutThreeEdges() {
        double f = 900e6;
        TerrainProfile one = TerrainProfile.uniform(3000, 0, 0, 0, 30, 0, 0, 0);
        TerrainProfile three = TerrainProfile.uniform(3000, 0, 20, 0, 30, 0, 20, 0);
        double l1 = Deygout.lossDb(one, f, 5, 5);
        double l3 = Deygout.lossDb(three, f, 5, 5);
        assertTrue(l3 > l1 + 5, "secondary edges add loss: " + l1 + " vs " + l3);
        // never more than three J terms, each bounded by J of the steepest edge
        double principal = KnifeEdge.lossDb(KnifeEdge.v(25, 1500, 1500, Units.wavelengthM(f)));
        assertEquals(principal, l1, 1e-9);
        assertTrue(l3 < 3 * principal);
    }

    @Test
    void deygoutControlFlatAndShort() {
        assertEquals(0, Deygout.lossDb(TerrainProfile.flat(1000, 64, 50), 2.4e9, 10, 10));
        assertEquals(0, Deygout.lossDb(TerrainProfile.uniform(100, 0, 0), 2.4e9, 1, 1));
        // terrain far below the line
        assertEquals(0, Deygout.lossDb(TerrainProfile.uniform(1000, 0, -50, 0), 2.4e9, 10, 10));
    }

    @Test
    void terrainProfileValidation() {
        assertThrows(IllegalArgumentException.class, () -> new TerrainProfile(new double[] {0, 2, 1}, new double[3]));
        assertThrows(IllegalArgumentException.class, () -> new TerrainProfile(new double[] {0}, new double[1]));
        assertEquals(1000, TerrainProfile.flat(1000, 0, 11).lengthM(), 1e-12);
    }
}
