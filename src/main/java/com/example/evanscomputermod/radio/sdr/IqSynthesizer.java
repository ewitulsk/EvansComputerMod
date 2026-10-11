package com.example.evanscomputermod.radio.sdr;

import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.RadioMedium;

import java.util.List;
import java.util.SplittableRandom;

/**
 * Builds an SDR's receive samples from what the medium says it hears (spec,
 * SDR block): {@code rx[n] = Σ g·shift(tx[n − τ], Δf) + w[n]}.
 *
 * <ul>
 *   <li>SDR transmitters ({@link Emission.Kind#IQ}) are resampled to the receive rate,
 *       delayed by the propagation delay and shifted to the receiver's centre.</li>
 *   <li>Packet radios and energy ({@code FRAME}/{@code ENERGY}) appear as band-limited
 *       noise bursts at the right power, bandwidth, offset and airtime: correct on a
 *       waterfall, not decodable.</li>
 *   <li>Thermal noise at {@code −174 + 10log(rate) + NF} dBm.</li>
 * </ul>
 * Amplitudes are √mW internally; gain sets the ADC full scale
 * ({@code −10 dBm − gain}); output is clipped to ±1 and quantised to the ADC depth.
 * Pure Java and deterministic for a given seed.
 */
public final class IqSynthesizer {

    /** ADC full scale with 0 dB gain, dBm. */
    public static final double FULL_SCALE_DBM_AT_0DB = -10;

    private final double sampleRate;
    private final double centerHz;
    private final int adcBits;
    private final double noiseFigureDb;
    private final long seed;

    public IqSynthesizer(double sampleRate, double centerHz, int adcBits, double noiseFigureDb, long seed) {
        this.sampleRate = sampleRate;
        this.centerHz = centerHz;
        this.adcBits = adcBits;
        this.noiseFigureDb = noiseFigureDb;
        this.seed = seed;
    }

    /** Absolute sample index at a world-clock time (µs). */
    public static long sampleAt(long micros, double rate) {
        return (long) Math.floor(micros * rate / 1e6);
    }

    /** World-clock time (µs) of an absolute sample index. */
    public static double microsAt(long sample, double rate) {
        return sample * 1e6 / rate;
    }

    /**
     * Synthesise {@code n} samples starting at absolute sample {@code s0} into
     * {@code out} (interleaved I/Q, length ≥ 2n) at the given RF gain.
     *
     * @return the peak pre-ADC amplitude relative to full scale (for AGC; &gt; 1 means clipping)
     */
    public double synthesize(long s0, int n, List<RadioMedium.Heard> heard, double gainDb, float[] out) {
        return synthesize(s0, n, heard, gainDb, out, 0);
    }

    /**
     * {@link #synthesize(long, int, List, double, float[])} through a channel filter: with
     * {@code bandwidthHz} between 0 and the sample rate, everything (signals and noise) is
     * low-pass filtered to ±bandwidth/2 before the ADC, as the SDR's channel filter would.
     * The filter's history is synthesised too (the samples just before {@code s0}), so a
     * stream read in blocks is filtered without seams.
     */
    public double synthesize(long s0, int n, List<RadioMedium.Heard> heard, double gainDb, float[] out, double bandwidthHz) {
        double[] taps = bandwidthHz > 0 && bandwidthHz < sampleRate * 0.98 ? lowPass(bandwidthHz / sampleRate) : null;
        int hist = taps == null ? 0 : taps.length - 1;
        double[] raw = analog(s0 - hist, n + hist, heard);
        double[] acc;
        if (taps == null) acc = raw;
        else {
            acc = new double[2 * n];
            for (int k = 0; k < n; k++) {
                double re = 0, im = 0;
                for (int t = 0; t < taps.length; t++) {
                    int i = k + hist - t;
                    re += taps[t] * raw[2 * i];
                    im += taps[t] * raw[2 * i + 1];
                }
                acc[2 * k] = re;
                acc[2 * k + 1] = im;
            }
        }
        double fsAmp = Math.sqrt(Math.pow(10, (FULL_SCALE_DBM_AT_0DB - gainDb) / 10));
        double levels = Math.pow(2, adcBits - 1) - 1;
        double peak = 0;
        for (int k = 0; k < 2 * n; k++) {
            double v = acc[k] / fsAmp;
            peak = Math.max(peak, Math.abs(v));
            v = Math.max(-1, Math.min(1, v));
            out[k] = (float) (Math.round(v * levels) / levels);
        }
        return peak;
    }

    /** Windowed-sinc (Blackman) low-pass taps, unity gain at DC, for a two-sided bandwidth {@code frac} of the rate. */
    static double[] lowPass(double frac) {
        int len = (int) Math.min(255, Math.max(31, Math.ceil(8 / frac))) | 1;
        double fc = frac / 2;   // cutoff, cycles per sample
        double[] h = new double[len];
        int mid = len / 2;
        double sum = 0;
        for (int i = 0; i < len; i++) {
            int m = i - mid;
            double sinc = m == 0 ? 2 * fc : Math.sin(2 * Math.PI * fc * m) / (Math.PI * m);
            double w = 0.42 - 0.5 * Math.cos(2 * Math.PI * i / (len - 1)) + 0.08 * Math.cos(4 * Math.PI * i / (len - 1));
            h[i] = sinc * w;
            sum += h[i];
        }
        for (int i = 0; i < len; i++) h[i] /= sum;
        return h;
    }

    /** The pre-ADC signal (√mW, interleaved I/Q) for {@code n} samples from absolute sample {@code s0}. */
    private double[] analog(long s0, int n, List<RadioMedium.Heard> heard) {
        double[] acc = new double[2 * n];
        SplittableRandom rng = new SplittableRandom(seed ^ (s0 * 0x9E3779B97F4A7C15L));
        // Thermal noise over the sample bandwidth.
        double noiseMw = Math.pow(10, (-174 + 10 * Math.log10(sampleRate) + noiseFigureDb) / 10);
        double sigma = Math.sqrt(noiseMw / 2);
        for (int k = 0; k < 2 * n; k++) acc[k] = sigma * gaussian(rng);

        for (RadioMedium.Heard h : heard) {
            Emission e = h.emission();
            double df = e.channel().centerHz() - centerHz;
            double halfBw = e.channel().bandwidthHz() / 2;
            if (Math.abs(df) - halfBw >= sampleRate / 2) continue;     // entirely outside the passband
            double rxMw = Math.pow(10, h.rxPowerDbm() / 10);
            if (e.kind() == Emission.Kind.IQ && e.iq() != null) addIq(acc, s0, n, e, rxMw, h.delayMicros(), df);
            else addBurst(acc, s0, n, e, rxMw, h.delayMicros(), df, rng);
        }
        return acc;
    }

    private void addIq(double[] acc, long s0, int n, Emission e, double rxMw, double delayMicros, double df) {
        float[] tx = e.iq();
        int len = tx.length / 2;
        double txRate = e.sampleRateHz();
        double amp = Math.sqrt(rxMw);   // tx waveform RMS 1 = emission power
        for (int k = 0; k < n; k++) {
            double tMicros = microsAt(s0 + k, sampleRate);
            double tau = (tMicros - delayMicros - e.startMicros()) * 1e-6 * txRate;
            if (tau < 0 || tau >= len - 1) continue;
            int i0 = (int) tau;
            double f = tau - i0;
            double re = tx[2 * i0] * (1 - f) + tx[2 * i0 + 2] * f;
            double im = tx[2 * i0 + 1] * (1 - f) + tx[2 * i0 + 3] * f;
            double ph = 2 * Math.PI * df * tMicros * 1e-6;
            double c = Math.cos(ph), s = Math.sin(ph);
            acc[2 * k] += amp * (re * c - im * s);
            acc[2 * k + 1] += amp * (re * s + im * c);
        }
    }

    private void addBurst(double[] acc, long s0, int n, Emission e, double rxMw, double delayMicros, double df,
                          SplittableRandom rng) {
        double bw = e.channel().bandwidthHz();
        // Fraction of the burst's power inside the sampled band.
        double lo = Math.max(df - bw / 2, -sampleRate / 2), hi = Math.min(df + bw / 2, sampleRate / 2);
        double inBand = Math.max(0, hi - lo) / bw;
        if (inBand <= 0) return;
        double centre = (lo + hi) / 2;
        int avg = (int) Math.max(1, Math.round(sampleRate / Math.max(1, hi - lo)));
        double sigma = Math.sqrt(rxMw * inBand * avg / 2);
        double runRe = 0, runIm = 0;
        double[] histRe = new double[avg], histIm = new double[avg];
        int hp = 0;
        for (int k = 0; k < n; k++) {
            double tMicros = microsAt(s0 + k, sampleRate) - delayMicros;
            double wr = sigma * gaussian(rng), wi = sigma * gaussian(rng);
            runRe += wr - histRe[hp];
            runIm += wi - histIm[hp];
            histRe[hp] = wr;
            histIm[hp] = wi;
            hp = (hp + 1) % avg;
            if (tMicros < e.startMicros() || tMicros >= e.endMicros()) continue;
            double re = runRe / avg, im = runIm / avg;
            double ph = 2 * Math.PI * centre * (tMicros + delayMicros) * 1e-6;
            double c = Math.cos(ph), s = Math.sin(ph);
            acc[2 * k] += re * c - im * s;
            acc[2 * k + 1] += re * s + im * c;
        }
    }

    private static double gaussian(SplittableRandom r) {
        // Box–Muller (one value per call keeps it stateless).
        double u = Math.max(1e-12, r.nextDouble());
        return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * r.nextDouble());
    }
}
