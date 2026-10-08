package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class IonosphereTest {
    private static final Ionosphere ION = Ionosphere.DEFAULT;
    private static final long SUNRISE = 0, NOON = 6000, SUNSET = 12000, MIDNIGHT = 18000;

    @Test
    void solarCycle() {
        assertEquals(0, Ionosphere.cosSolarZenith(SUNRISE), 1e-12);
        assertEquals(1, Ionosphere.cosSolarZenith(NOON), 1e-12);
        assertEquals(-1, Ionosphere.cosSolarZenith(MIDNIGHT), 1e-12);
        assertEquals(Ionosphere.cosSolarZenith(NOON), Ionosphere.cosSolarZenith(NOON + 24000L * 7), 1e-12);
        assertEquals(10e6, ION.foF2Hz(NOON), 1);
        assertEquals(4e6, ION.foF2Hz(MIDNIGHT), 1);
    }

    @Test
    void hopCompressionDefaultsTo8kBlocks() {
        // the longest single hop maps to 8000 blocks; real geometry gives ~3400 km at 2 degrees
        assertEquals(3414e3, ION.maxHopRealM(), 30e3);
        assertEquals(8000, ION.maxHopRealM() / ION.metresPerBlock(), 1e-6);
        // typical skip distances land in the low thousands of blocks
        double skip = ION.skipDistanceBlocks(20e6, NOON);
        assertTrue(skip > 1500 && skip < 4000, "skip " + skip);
    }

    @Test
    void daytimeDLayerAbsorbsMf() {
        SkywavePath day = ION.skywave(1e6, 1000, NOON, true);
        SkywavePath night = ION.skywave(1e6, 1000, MIDNIGHT, true);
        assertTrue(day.available() && night.available());
        assertTrue(day.absorptionDb() > 40, "day absorption " + day.absorptionDb());
        assertEquals(0, night.absorptionDb(), 1e-12);
        assertTrue(day.lossDb() > night.lossDb() + 40);
        // HF is absorbed far less than MF by day (1/f^2)
        assertTrue(ION.dLayerAbsorptionDb(14e6, NOON, 0.5) < ION.dLayerAbsorptionDb(1e6, NOON, 0.5) / 10);
    }

    @Test
    void nightAllowsSkywave() {
        // MF below night foF2: reflects at every distance (no skip), small loss over thousands of blocks
        SkywavePath p = ION.skywave(1e6, 5000, MIDNIGHT, true);
        assertTrue(p.available());
        assertEquals(1, p.hops());
        assertEquals(0, p.skipDistanceBlocks());
        assertTrue(p.lossDb() < 90, "night MF loss " + p.lossDb());
    }

    @Test
    void skipZoneAndMuf() {
        double skip = ION.skipDistanceBlocks(20e6, NOON);
        assertFalse(ION.skywave(20e6, skip * 0.8, NOON, true).available(), "inside the skip zone");
        assertTrue(ION.skywave(20e6, skip * 1.2, NOON, true).available(), "beyond the skip zone");
        // MUF follows the day/night cycle
        assertTrue(ION.mufHz(4000, NOON) > ION.mufHz(4000, MIDNIGHT) * 2);
        // MUF grows with hop length (secant law), and equals foF2 at vertical incidence
        assertTrue(ION.mufHz(6000, NOON) > ION.mufHz(2000, NOON));
        assertEquals(ION.foF2Hz(NOON), ION.mufHz(0, NOON), 1);
        // 20 MHz closes at night: above the MUF of even the longest hop
        assertEquals(Double.POSITIVE_INFINITY, ION.skipDistanceBlocks(20e6, MIDNIGHT));
        assertFalse(ION.skywave(20e6, 7000, MIDNIGHT, true).available());
    }

    @Test
    void multiHop() {
        SkywavePath p = ION.skywave(3e6, 20_000, MIDNIGHT, true);
        assertTrue(p.available());
        assertEquals(3, p.hops());
        assertFalse(ION.skywave(3e6, 60_000, MIDNIGHT, true).available(), "more than maxHops");
        // compression is configurable
        Ionosphere wide = ION.withMaxHopBlocks(2000);
        assertEquals(10, (int) Math.ceil(20_000 / 2000.0));
        assertFalse(wide.skywave(3e6, 20_000, MIDNIGHT, true).available());
        assertEquals(2, wide.skywave(3e6, 3000, MIDNIGHT, true).hops());
    }

    @Test
    void noSkywaveWithoutSky() {
        assertFalse(ION.skywave(3e6, 5000, MIDNIGHT, false).available());
        assertEquals(Double.POSITIVE_INFINITY, ION.lossDb(3e6, 5000, MIDNIGHT, false));
    }

    @Test
    void controlZeroDistanceAndDeterminism() {
        SkywavePath p = ION.skywave(3e6, 0, MIDNIGHT, true);
        assertTrue(p.available(), "NVIS straight up and down");
        assertEquals(Math.PI / 2, p.elevationRad(), 1e-9);
        assertEquals(p, ION.skywave(3e6, 0, MIDNIGHT, true));
        assertFalse(ION.skywave(0, 1000, MIDNIGHT, true).available());
        // sunset is symmetric to sunrise
        assertEquals(ION.foF2Hz(SUNRISE), ION.foF2Hz(SUNSET), 1);
    }
}
