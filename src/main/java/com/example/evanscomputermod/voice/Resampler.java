package com.example.evanscomputermod.voice;

/**
 * Streaming mono sample-rate converter: linear interpolation, with a low-pass
 * in front when the rate goes down so nothing above the new Nyquist folds back.
 * Keeps its place between blocks, so a stream converted in pieces has no seams.
 */
public final class Resampler {

    private final int inRate;
    private final int outRate;
    private final VoiceModulator.Biquad aa1, aa2;
    private double phase = -1;
    private float last;

    public Resampler(int inRate, int outRate) {
        this.inRate = inRate;
        this.outRate = outRate;
        if (outRate < inRate) {
            aa1 = VoiceModulator.Biquad.lowPass(outRate * 0.45, inRate);
            aa2 = VoiceModulator.Biquad.lowPass(outRate * 0.45, inRate);
        } else {
            aa1 = aa2 = null;
        }
    }

    public int inRate() { return inRate; }

    public int outRate() { return outRate; }

    /** Convert one block of 16-bit samples. */
    public short[] process(short[] in, int off, int n) {
        if (n <= 0) return new short[0];
        float[] x = new float[n];
        for (int i = 0; i < n; i++) {
            double v = in[off + i];
            if (aa1 != null) v = aa2.step(aa1.step(v));
            x[i] = (float) v;
        }
        if (inRate == outRate) {
            short[] out = new short[n];
            for (int i = 0; i < n; i++) out[i] = clamp(x[i]);
            return out;
        }
        double step = (double) inRate / outRate;
        short[] out = new short[(int) Math.ceil((n - 1 - phase) / step) + 1];
        int k = 0;
        while (phase < n - 1) {
            int i = (int) Math.floor(phase);
            float f = (float) (phase - i);
            float a = i < 0 ? last : x[i];
            float b = x[i + 1];
            out[k++] = clamp(a + (b - a) * f);
            phase += step;
        }
        phase -= n;
        last = x[n - 1];
        return k == out.length ? out : java.util.Arrays.copyOf(out, k);
    }

    private static short clamp(double v) {
        return (short) Math.max(-32768, Math.min(32767, Math.round(v)));
    }
}
