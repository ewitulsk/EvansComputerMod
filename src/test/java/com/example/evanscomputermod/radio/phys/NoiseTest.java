package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class NoiseTest {
    private static final long NOON = 6000, MIDNIGHT = 18000;

    @Test
    void thermalFloor() {
        // -174 + 10 log(20 MHz) + 0 = -100.99 dBm; 22 MHz with NF 6 = -94.58
        assertEquals(-100.99, Noise.thermalDbm(20e6, 0), 0.01);
        assertEquals(-94.58, Noise.thermalDbm(22e6, 6), 0.01);
    }

    @Test
    void microwaveFloorIsThermal() {
        double floor = Noise.floorDbm(20e6, 6, 2.4e9, Noise.Environment.NONE, NOON, false);
        assertEquals(Noise.thermalDbm(20e6, 6), floor, 0.05);
        // thunder makes no difference at 2.4 GHz
        double storm = Noise.floorDbm(20e6, 6, 2.4e9, Noise.Environment.NONE, NOON, true);
        assertEquals(floor, storm, 0.1);
    }

    @Test
    void p372ManMadeAndGalactic() {
        // residential at 10 MHz: 72.5 - 27.7 = 44.8 dB above kT0B
        assertEquals(44.8, Noise.manMadeFaDb(10e6, Noise.Environment.RESIDENTIAL), 1e-9);
        assertEquals(52 - 23 * 2, Noise.galacticFaDb(100e6), 1e-9);
        assertEquals(Double.NEGATIVE_INFINITY, Noise.galacticFaDb(1e6), "screened by the ionosphere");
    }

    @Test
    void hfNoiseIsHighAndFollowsDayNight() {
        double night = Noise.floorDbm(3e3, 10, 1e6, Noise.Environment.QUIET_RURAL, MIDNIGHT, false);
        double day = Noise.floorDbm(3e3, 10, 1e6, Noise.Environment.QUIET_RURAL, NOON, false);
        assertTrue(night > day + 10, "night " + night + " day " + day);
        assertTrue(night > Noise.thermalDbm(3e3, 10) + 50, "atmospheric noise dominates at MF");
        // atmospheric noise falls steeply with frequency
        assertTrue(Noise.atmosphericNightFaDb(1e6) > Noise.atmosphericNightFaDb(30e6) + 40);
    }

    @Test
    void thunderAddsImpulsiveNoiseAtHf() {
        double calm = Noise.floorDbm(3e3, 10, 1e6, Noise.Environment.RURAL, MIDNIGHT, false);
        double storm = Noise.floorDbm(3e3, 10, 1e6, Noise.Environment.RURAL, MIDNIGHT, true);
        assertEquals(Noise.THUNDERSTORM_EXCESS_DB, storm - calm, 1.0);
        // a strike nearby is louder at low frequency and short range
        assertTrue(Noise.lightningImpulseDbm(1e6, 1e4, 500) > Noise.lightningImpulseDbm(10e6, 1e4, 500));
        assertTrue(Noise.lightningImpulseDbm(1e6, 1e4, 500) > Noise.lightningImpulseDbm(1e6, 1e4, 5000));
        assertEquals(-20, Noise.lightningImpulseDbm(1e6, 1e4, 1000), 1e-9);
    }

    @Test
    void rainFadeP838() {
        // P.838-3: 20 GHz, k = 0.09164, alpha = 1.0568 -> 2.75 dB/km at 25 mm/h
        assertEquals(2.75, Noise.rainSpecificAttenuationDbPerKm(20e9, 25), 0.02);
        // 30 GHz, k = 0.2403, alpha = 0.9485
        assertEquals(0.2403 * Math.pow(50, 0.9485), Noise.rainSpecificAttenuationDbPerKm(30e9, 50), 1e-9);
        assertEquals(2.75 * 0.5, Noise.rainFadeDb(20e9, 25, 500), 0.02);
        // grows with frequency over the microwave band
        double prev = 0;
        for(double f = 10e9; f <= 100e9; f += 1e9) {
            double g = Noise.rainSpecificAttenuationDbPerKm(f, 25);
            assertTrue(g > prev, "rain attenuation at " + f);
            prev = g;
        }
        // negligible below 10 GHz compared to above
        assertTrue(Noise.rainSpecificAttenuationDbPerKm(2.4e9, 25) < 0.01);
    }

    @Test
    void rainControls() {
        assertEquals(0, Noise.rainSpecificAttenuationDbPerKm(20e9, 0));
        assertEquals(0, Noise.rainSpecificAttenuationDbPerKm(100e6, 50));
        assertEquals(0, Noise.rainFadeDb(20e9, 25, 0));
        assertEquals(Noise.rainSpecificAttenuationDbPerKm(100e9, 10), Noise.rainSpecificAttenuationDbPerKm(200e9, 10));
        assertEquals(-3.0103 + 3.0103 * 2, Noise.addDbm(0, 0), 1e-3);
    }
}
