package com.example.evanscomputermod.radio.handheld;

/**
 * The handheld's demodulators, pure Java. Works on blocks of interleaved IQ
 * and keeps the little state AM/FM need between blocks (previous phase, DC
 * level, de-emphasis), so audio is continuous across server ticks.
 */
public final class HandheldDemod {

    private double prevI = 1, prevQ = 0;
    private double dc;
    private double deemph;
    private double avgPower;

    /** Mean power of an IQ block relative to ADC full scale (for the S-meter / squelch). */
    public static double power(float[] iq, int n) {
        double p = 0;
        for (int k = 0; k < 2 * n; k++) p += iq[k] * iq[k];
        return p / Math.max(1, n);
    }

    /**
     * Envelope AM: |iq| with the carrier (DC) removed, decimated by {@code decim},
     * scaled to ±1-ish. Output length n / decim.
     */
    public float[] am(float[] iq, int n, int decim, double gain) {
        int m = n / decim;
        float[] out = new float[m];
        for (int j = 0; j < m; j++) {
            double acc = 0;
            for (int d = 0; d < decim; d++) {
                int k = j * decim + d;
                acc += Math.hypot(iq[2 * k], iq[2 * k + 1]);
            }
            double env = acc / decim;
            dc += (env - dc) * 0.002;
            out[j] = (float) clamp((env - dc) / Math.max(1e-6, dc) * gain);
        }
        return out;
    }

    /**
     * FM discriminator: phase difference between successive samples scaled by
     * the peak deviation, with 50 µs-ish de-emphasis, decimated by {@code decim}.
     */
    public float[] fm(float[] iq, int n, int decim, double sampleRate, double deviationHz) {
        int m = n / decim;
        float[] out = new float[m];
        double scale = sampleRate / (2 * Math.PI * deviationHz);
        double alpha = 1 - Math.exp(-1 / (sampleRate / decim * 75e-6));
        for (int j = 0; j < m; j++) {
            double acc = 0;
            for (int d = 0; d < decim; d++) {
                int k = j * decim + d;
                double i = iq[2 * k], q = iq[2 * k + 1];
                // arg(x[k] * conj(x[k-1]))
                double re = i * prevI + q * prevQ, im = q * prevI - i * prevQ;
                acc += Math.atan2(im, re) * scale;
                prevI = i;
                prevQ = q;
            }
            double v = acc / decim;
            deemph += (v - deemph) * alpha;
            out[j] = (float) clamp(deemph);
        }
        return out;
    }

    /** Smoothed in-channel power, dBFS, for the meter. */
    public double meterDbfs(float[] iq, int n) {
        double p = power(iq, n);
        avgPower = avgPower == 0 ? p : avgPower * 0.7 + p * 0.3;
        return 10 * Math.log10(Math.max(1e-12, avgPower));
    }

    private static double clamp(double v) {
        return Math.max(-1, Math.min(1, v));
    }

    /** Scale ±1 audio to 16-bit PCM at a volume 0..100. */
    public static short[] toPcm(float[] audio, int volume) {
        short[] pcm = new short[audio.length];
        double g = volume / 100.0 * 32000;
        for (int i = 0; i < audio.length; i++) pcm[i] = (short) Math.round(audio[i] * g);
        return pcm;
    }
}
