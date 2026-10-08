package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import java.util.Map;

import org.junit.jupiter.api.Test;

public class SkinDepthMaterialsTest {
    @Test
    void seaWaterAt10kHz() {
        // delta = sqrt(2 / (omega mu sigma)) with sigma = 4 S/m: 2.52 m
        assertEquals(2.5, SkinDepth.depthM(10e3, Ground.SEA_WATER), 0.1);
        assertEquals(SkinDepth.goodConductorDepthM(10e3, 4.0), SkinDepth.depthM(10e3, Ground.SEA_WATER), 0.01);
        // 8.686 / delta = 3.45 dB per metre
        assertEquals(3.45, SkinDepth.attenuationDbPerM(10e3, Ground.SEA_WATER), 0.1);
    }

    @Test
    void copperAt60Hz() {
        // textbook: 8.5 mm
        assertEquals(8.5e-3, SkinDepth.goodConductorDepthM(60, 5.8e7), 0.1e-3);
    }

    @Test
    void vlfPenetratesGhzDoesNot() {
        // VLF reaches tens of metres into earth; GHz is stopped in sea water within millimetres
        assertTrue(SkinDepth.depthM(10e3, Ground.AVERAGE_GROUND) > 50);
        assertTrue(SkinDepth.attenuationDbPerM(10e3, Ground.DRY_GROUND) < 0.1);
        assertTrue(SkinDepth.attenuationDbPerM(2.4e9, Ground.SEA_WATER) > 300);
        assertTrue(SkinDepth.depthM(2.4e9, Ground.SEA_WATER) < 0.05);
        // 100 m of rock above a mine at 10 kHz costs only a few dB
        assertTrue(GroundWave.penetrationLossDb(100, 10e3, Ground.DRY_GROUND) < 10);
    }

    @Test
    void lowLossDielectricLimit() {
        // sigma << omega eps: alpha -> (sigma/2) sqrt(mu0 / (eps0 er))
        Ground g = Ground.DRY_SAND;
        double expected = g.conductivitySPerM() / 2 * Math.sqrt(Units.MU0_H_PER_M / (Units.EPS0_F_PER_M * 3));
        assertEquals(expected, SkinDepth.attenuationNpPerM(10e9, g), expected * 1e-3);
    }

    @Test
    void skinDepthControls() {
        assertEquals(Double.POSITIVE_INFINITY, SkinDepth.depthM(1e6, new Ground(5, 0)));
        assertEquals(0, SkinDepth.depthM(1e6, Ground.METAL));
        assertEquals(Double.POSITIVE_INFINITY, SkinDepth.depthM(0, Ground.SEA_WATER));
        assertEquals(0, SkinDepth.penetrationLossDb(0, 1e6, Ground.SEA_WATER));
        assertThrows(IllegalArgumentException.class, () -> new Ground(0.5, 0));
    }

    @Test
    void materialTableMonotonicInFrequency() {
        for(Map.Entry<String, MaterialAttenuation> e : Materials.ALL.entrySet()) {
            double prev = -1;
            for(double f = 3e3; f <= 100e9; f *= 1.25) {
                double a = e.getValue().dbPerBlock(f);
                assertTrue(a >= prev - 1e-12, e.getKey() + " decreases at " + f);
                assertTrue(a >= 0 && Double.isFinite(a), e.getKey());
                prev = a;
            }
        }
    }

    @Test
    void materialOrderingAt2_4GHz() {
        double f = 2.4e9;
        double glass = Materials.GLASS.dbPerBlock(f), wood = Materials.WOOD.dbPerBlock(f);
        double concrete = Materials.CONCRETE.dbPerBlock(f), water = Materials.WATER.dbPerBlock(f);
        assertTrue(glass < wood && wood < concrete && concrete < water, glass + " " + wood + " " + concrete + " " + water);
        assertTrue(glass < 10, "glass is cheap");
        assertTrue(water > 100, "water absorbs heavily at 2.4 GHz");
        assertTrue(Materials.IRON.dbPerBlock(f) >= 40 && Materials.COPPER.dbPerBlock(f) >= 40, "metal is opaque");
        assertTrue(Materials.LEAVES.dbPerBlock(f) > Materials.LEAVES.dbPerBlock(100e6), "foliage loss grows with f");
        assertTrue(Materials.DIRT.dbPerBlock(f) > 20);
        // the power law matches its definition
        assertEquals(5.45 * Math.pow(2.4, 1.07), wood, 1e-9);
    }

    @Test
    void materialLowFrequencyUsesSkinDepth() {
        // VLF into the sea: skin-depth loss (3.45 dB/m) dominates the dielectric power law
        assertEquals(3.45, Materials.SEA_WATER.dbPerBlock(10e3), 0.1);
        assertTrue(Materials.DIRT.dbPerBlock(10e3) < 1);
        assertTrue(Materials.STONE.dbPerBlock(10e3) < 0.1);
    }

    @Test
    void materialControls() {
        assertEquals(0, Materials.AIR.dbPerBlock(2.4e9));
        assertSame(Materials.AIR, Materials.getOrAir("unobtainium"));
        assertNull(Materials.get("unobtainium"));
        assertEquals(0, Materials.WATER.dbPerBlock(0));
        assertEquals(0, Materials.GLASS.lossDb(0, 2.4e9));
        assertEquals(2 * Materials.GLASS.dbPerBlock(5e9), Materials.GLASS.lossDb(2, 5e9), 1e-12);
        for(String n : new String[] {"air", "glass", "wood", "leaves", "stone", "dirt", "water", "iron", "copper",
                "concrete", "wool", "ice", "snow"})
            assertNotNull(Materials.get(n), n);
    }
}
