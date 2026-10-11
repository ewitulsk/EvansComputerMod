package com.example.evanscomputermod.radio.handheld;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class HandheldDemodTest {

    /** Correlation amplitude of {@code a} with a sine at {@code hz}. */
    static double toneLevel(float[] a, double hz, double rate) {
        double re = 0, im = 0;
        for (int k = 0; k < a.length; k++) {
            re += a[k] * Math.cos(2 * Math.PI * hz * k / rate);
            im += a[k] * Math.sin(2 * Math.PI * hz * k / rate);
        }
        return 2 * Math.hypot(re, im) / a.length;
    }

    @Test
    void fmDiscriminatorRecoversTone() {
        int rate = 48_000, n = 48_000;
        float[] iq = new float[2 * n];
        double phase = 0, dev = 5000;
        for (int k = 0; k < n; k++) {
            double audio = 0.8 * Math.sin(2 * Math.PI * 1000 * k / rate);
            phase += 2 * Math.PI * dev * audio / rate;
            iq[2 * k] = (float) (0.3 * Math.cos(phase));
            iq[2 * k + 1] = (float) (0.3 * Math.sin(phase));
        }
        float[] out = new HandheldDemod().fm(iq, n, 2, rate, dev);
        double at1k = toneLevel(out, 1000, rate / 2.0), at3k = toneLevel(out, 3000, rate / 2.0);
        assertTrue(at1k > 0.4, "1 kHz level " + at1k);
        assertTrue(at1k > 20 * at3k, "clean tone: " + at1k + " vs " + at3k);
    }

    @Test
    void amEnvelopeRecoversToneAndNoiseIsNotATone() {
        int rate = 48_000, n = 48_000;
        float[] iq = new float[2 * n];
        java.util.SplittableRandom r = new java.util.SplittableRandom(3);
        for (int k = 0; k < n; k++) {
            double env = 0.2 * (1 + 0.6 * Math.sin(2 * Math.PI * 800 * k / rate));
            double ph = 2 * Math.PI * 300 * k / rate;   // small tuning offset
            iq[2 * k] = (float) (env * Math.cos(ph));
            iq[2 * k + 1] = (float) (env * Math.sin(ph));
        }
        HandheldDemod d = new HandheldDemod();
        d.am(iq, n, 2, 1);   // settle the carrier estimate
        float[] out = d.am(iq, n, 2, 1);
        assertTrue(toneLevel(out, 800, rate / 2.0) > 0.3, "800 Hz AM tone");
        float[] noise = new float[2 * n];
        for (int k = 0; k < 2 * n; k++) noise[k] = (float) (0.01 * r.nextGaussian());
        float[] hiss = new HandheldDemod().am(noise, n, 2, 1);
        assertTrue(toneLevel(hiss, 800, rate / 2.0) < 0.05, "control: noise has no 800 Hz tone");
    }

    @Test
    void bandsClampAndWideFm() {
        assertEquals(30e6, HandheldBand.SW.clamp(40e6));
        assertTrue(HandheldBand.VHF.wide(98.1e6));
        assertFalse(HandheldBand.VHF.wide(146.52e6));
        assertEquals(200e3, HandheldBand.VHF.bandwidth(100e6));
    }
}
