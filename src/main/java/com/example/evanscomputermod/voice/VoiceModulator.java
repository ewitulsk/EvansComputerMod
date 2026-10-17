package com.example.evanscomputermod.voice;

/**
 * A voice transmitter's modulator, pure Java: microphone audio in, complex
 * baseband (interleaved I/Q, RMS 1 = the transmitter's power) out, the way an
 * analogue two-way radio does it.
 *
 * <ul>
 *   <li><b>FM</b>: the audio is band-limited (3 kHz for a narrow channel,
 *       15 kHz for broadcast), pre-emphasised (75 µs, matched to the
 *       receivers' de-emphasis), clipped at full deviation and integrated into
 *       the carrier's phase. Constant envelope.</li>
 *   <li><b>AM</b>: band-limited to 4 kHz and put on the carrier's envelope at
 *       {@link #AM_DEPTH} modulation depth.</li>
 * </ul>
 * State carries from block to block, so a stream modulated in pieces has no seams.
 */
public final class VoiceModulator {

    public enum Mode { FM, AM }

    /** AM modulation depth at full-scale audio. */
    public static final double AM_DEPTH = 0.8;

    private final Mode mode;
    private final int audioRate;
    private final int iqRate;
    private final double deviationHz;

    private final Biquad lowPass1, lowPass2;
    private final double preAlpha;
    private double prevAudio;
    private double prevSample;
    private double phase;

    /**
     * @param audioRate   rate of the audio given to {@link #modulate}
     * @param iqRate      rate of the I/Q out (a whole multiple of {@code audioRate})
     * @param deviationHz FM peak deviation (5 kHz narrow, 75 kHz broadcast); ignored for AM
     */
    public VoiceModulator(Mode mode, int audioRate, int iqRate, double deviationHz) {
        if (iqRate % audioRate != 0) throw new IllegalArgumentException("I/Q rate must be a multiple of the audio rate");
        this.mode = mode;
        this.audioRate = audioRate;
        this.iqRate = iqRate;
        this.deviationHz = deviationHz;
        double cutoff = mode == Mode.AM ? 4000 : deviationHz > 20_000 ? 15_000 : 3000;
        cutoff = Math.min(cutoff, audioRate * 0.45);
        lowPass1 = Biquad.lowPass(cutoff, audioRate);
        lowPass2 = Biquad.lowPass(cutoff, audioRate);
        // Inverse of the receivers' de-emphasis y += (x - y)·α at the audio rate.
        preAlpha = 1 - Math.exp(-1 / (audioRate * 75e-6));
    }

    public Mode mode() { return mode; }

    public int iqRate() { return iqRate; }

    /** Modulate {@code audio} (±1 at the audio rate); returns {@code audio.length · iqRate / audioRate} I/Q samples. */
    public float[] modulate(float[] audio) {
        int up = iqRate / audioRate;
        float[] iq = new float[2 * audio.length * up];
        int k = 0;
        for (float a : audio) {
            double x = lowPass2.step(lowPass1.step(a));
            double v;
            if (mode == Mode.FM) {
                double pre = (x - (1 - preAlpha) * prevAudio) / preAlpha;
                prevAudio = x;
                // Pre-emphasis lifts the treble; keep the boost from overshooting the deviation.
                v = Math.max(-1, Math.min(1, pre * preAlpha * 1.6));
            } else {
                v = Math.max(-1, Math.min(1, x));
            }
            for (int u = 1; u <= up; u++) {
                double s = prevSample + (v - prevSample) * u / up;
                if (mode == Mode.FM) {
                    phase += 2 * Math.PI * deviationHz * s / iqRate;
                    if (phase > Math.PI) phase -= 2 * Math.PI;
                    else if (phase < -Math.PI) phase += 2 * Math.PI;
                    iq[k++] = (float) Math.cos(phase);
                    iq[k++] = (float) Math.sin(phase);
                } else {
                    // Carrier plus sidebands, scaled so a full-depth tone keeps RMS near 1.
                    iq[k++] = (float) ((1 + AM_DEPTH * s) / Math.sqrt(1 + AM_DEPTH * AM_DEPTH / 2));
                    iq[k++] = 0;
                }
            }
            prevSample = v;
        }
        return iq;
    }

    /** A second-order low-pass section (RBJ cookbook, Q = 0.707). */
    static final class Biquad {
        final double b0, b1, b2, a1, a2;
        double z1, z2;

        Biquad(double b0, double b1, double b2, double a1, double a2) {
            this.b0 = b0;
            this.b1 = b1;
            this.b2 = b2;
            this.a1 = a1;
            this.a2 = a2;
        }

        static Biquad lowPass(double cutoff, double rate) {
            double w = 2 * Math.PI * cutoff / rate;
            double alpha = Math.sin(w) / (2 * 0.7071);
            double cos = Math.cos(w);
            double a0 = 1 + alpha;
            return new Biquad((1 - cos) / 2 / a0, (1 - cos) / a0, (1 - cos) / 2 / a0, -2 * cos / a0, (1 - alpha) / a0);
        }

        double step(double x) {
            double y = b0 * x + z1;
            z1 = b1 * x - a1 * y + z2;
            z2 = b2 * x - a2 * y;
            return y;
        }
    }
}
