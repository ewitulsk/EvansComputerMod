package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class SpectralMaskTest {
    private static final SpectralMask B = SpectralMask.DSSS_22MHZ;

    private static double w24(int a, int b) {
        return B.weight(Band.wifi24ChannelHz(a), Band.wifi24ChannelHz(b), 22e6);
    }

    @Test
    void channelCentres() {
        assertEquals(2412e6, Band.wifi24ChannelHz(1));
        assertEquals(2437e6, Band.wifi24ChannelHz(6));
        assertEquals(2484e6, Band.wifi24ChannelHz(14));
        assertEquals(5180e6, Band.wifi5ChannelHz(36));
        assertEquals(Band.WIFI_24, Band.of(2.437e9));
        assertEquals(Band.HF, Band.of(7e6));
        assertNull(Band.of(1.5e9));
        assertThrows(IllegalArgumentException.class, () -> Band.wifi24ChannelHz(15));
    }

    @Test
    void dsssCoChannelAndOverlap() {
        assertTrue(w24(6, 6) > 0.99);
        // channel 1 vs 2 (5 MHz apart): 17 of 22 MHz in band at 0 dBr -> -1.1 dB
        assertEquals(-1.12, Units.linearToDb(w24(1, 2)), 0.1);
        // channel 1 vs 6 (25 MHz apart): only mask skirts -> about -34 dB
        assertEquals(-34.3, Units.linearToDb(w24(1, 6)), 1.0);
        // 1/6/11 is the right plan: 1 vs 11 is negligible
        assertTrue(Units.linearToDb(w24(1, 11)) < -50);
    }

    @Test
    void overlapFallsWithSeparationAndIsSymmetric() {
        double prev = 1.01;
        for(int c = 1; c <= 13; c++) {
            double w = w24(1, c);
            assertTrue(w < prev || w == 0, "channel " + c);
            assertEquals(w, w24(c, 1), 1e-9);
            prev = w;
        }
    }

    @Test
    void ofdmMasks() {
        double adj = SpectralMask.OFDM_20MHZ.weightDb(5180e6, 20e6, 5200e6, 20e6);
        assertTrue(adj < -15 && adj > -35, "adjacent 20 MHz OFDM " + adj);
        assertTrue(SpectralMask.OFDM_20MHZ.weight(5180e6, 5180e6, 20e6) > 0.99);
        // a 40 MHz transmitter covering a 20 MHz receiver puts about half its power in it
        assertEquals(0.5, SpectralMask.OFDM_40MHZ.weight(5190e6, 40e6, 5180e6, 20e6), 0.05);
        // a narrow receiver inside a wide signal gets the bandwidth ratio
        assertEquals(0.1, SpectralMask.GENERIC.weight(145e6, 25e3, 145e6, 2.5e3), 0.01);
    }

    @Test
    void controls() {
        assertEquals(0, B.weight(2412e6, 0, 2412e6, 22e6));
        assertEquals(0, B.weight(2412e6, 22e6, 5180e6, 20e6));
        assertEquals(Double.NEGATIVE_INFINITY, B.weightDb(2412e6, 22e6, 5180e6, 20e6));
        assertEquals(1.0, B.psd(0), 1e-12);
        assertEquals(1e-3, B.psd(0.75), 1e-9);
        assertEquals(22e6, B.nominalBandwidthHz());
    }
}
