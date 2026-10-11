package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class FreeSpaceFresnelTest {
    @Test
    void friisConstantIsMinus147_55() {
        assertEquals(-147.55, FreeSpace.FRIIS_CONSTANT_DB, 0.01);
    }

    @Test
    void friisKnownValues() {
        // 1 km at 2.4 GHz: 60 + 187.60 - 147.55 = 100.05 dB
        assertEquals(100.05, FreeSpace.lossDb(1000, 2.4e9), 0.02);
        // 1 km at 1 GHz: 92.45 dB
        assertEquals(92.45, FreeSpace.lossDb(1000, 1e9), 0.02);
        // 100 m at 2.4 GHz: 80.05 dB
        assertEquals(80.05, FreeSpace.lossDb(100, 2.4e9), 0.02);
        // 20 log(4 pi d / lambda) by first principles at 10 km, 7 MHz
        double lambda = Units.wavelengthM(7e6);
        assertEquals(20 * Math.log10(4 * Math.PI * 1e4 / lambda), FreeSpace.lossDb(1e4, 7e6), 1e-9);
        // 6 dB per doubling of distance, 20 dB per decade of frequency
        assertEquals(6.02, FreeSpace.lossDb(200, 2.4e9) - FreeSpace.lossDb(100, 2.4e9), 0.01);
        assertEquals(20.0, FreeSpace.lossDb(100, 24e9) - FreeSpace.lossDb(100, 2.4e9), 1e-9);
    }

    @Test
    void friisInverse() {
        assertEquals(1000, FreeSpace.distanceForLossM(FreeSpace.lossDb(1000, 2.4e9), 2.4e9), 1e-6);
    }

    @Test
    void friisControlZeroAndNearField() {
        assertEquals(0, FreeSpace.lossDb(0, 2.4e9));
        assertEquals(0, FreeSpace.lossDb(-5, 2.4e9));
        assertEquals(0, FreeSpace.lossDb(1e-3, 2.4e9)); // inside lambda/4pi: clamped
    }

    @Test
    void unitConversions() {
        assertEquals(0.125, Units.wavelengthM(2.398339664e9), 1e-9);
        assertEquals(30, Units.wToDbm(1), 1e-12);
        assertEquals(1, Units.dbmToMw(0), 1e-12);
        assertEquals(100, Units.dbToLinear(20), 1e-9);
        assertEquals(-20, Units.linearToDb(0.01), 1e-9);
        assertEquals(Double.NEGATIVE_INFINITY, Units.mwToDbm(0));
        assertEquals(-174, Units.thermalNoiseDbm(290, 1), 0.05);
        assertEquals(8.6859, Units.DB_PER_NEPER, 1e-4);
        assertEquals(8.854e-12, Units.EPS0_F_PER_M, 1e-15);
        assertEquals(1e-6 / 3, Units.propagationDelayS(100), 1e-9);
    }

    @Test
    void fresnelRadius100mAt2_4GHz() {
        // spec: a 100-block 2.4 GHz link needs ~1.8 blocks of clearance
        assertEquals(1.77, Fresnel.midpathRadiusM(2.4e9, 100), 0.01);
        assertEquals(1.77, Fresnel.radiusM(2.4e9, 50, 50, 1), 0.01);
        // zone n scales as sqrt(n)
        assertEquals(Math.sqrt(2) * 1.767, Fresnel.radiusM(2.4e9, 50, 50, 2), 0.01);
        // HF (lambda ~ 40 m at 7.5 MHz): tens of metres
        assertTrue(Fresnel.midpathRadiusM(7.5e6, 100) > 30);
    }

    @Test
    void fresnelClearance() {
        double r = Fresnel.radiusM(2.4e9, 50, 50, 1);
        // a full first zone clear: no loss
        assertEquals(0, Fresnel.clearanceLossDb(r, 2.4e9, 50, 50));
        // grazing (edge on the line): 6 dB
        assertEquals(6.0, Fresnel.clearanceLossDb(0, 2.4e9, 50, 50), 0.1);
        // blocked by one zone radius: v = sqrt(2) -> about 16.4 dB
        assertEquals(KnifeEdge.lossDb(Math.sqrt(2)), Fresnel.clearanceLossDb(-r, 2.4e9, 50, 50), 1e-9);
        assertEquals(1.0, Fresnel.clearanceRatio(r, 2.4e9, 50, 50), 1e-12);
    }

    @Test
    void fresnelControlEndpoint() {
        assertEquals(0, Fresnel.radiusM(2.4e9, 0, 100, 1));
        assertEquals(0, Fresnel.radiusM(2.4e9, 50, 50, 0));
    }
}
