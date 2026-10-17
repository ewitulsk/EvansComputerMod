package com.example.evanscomputermod.voice;

/**
 * The recent sound of one source in the world (a player's voice, a speaker's
 * cone), as it is in the air around it: mono samples at {@link #AIR_RATE} on
 * the {@link SoundField} clock.
 *
 * <p>Audio arrives in bursts (a client sends its microphone every tick, a speaker
 * is drained every server tick), so each burst is placed {@link #LATENCY_MS}
 * after it arrived, back to back with the previous one, like a VoIP jitter
 * buffer. A source that went quiet starts again {@link #LATENCY_MS} after its
 * next burst. Whatever reads the air (a microphone) reads up to "now", which is
 * always behind what was written. The last {@link #CAPACITY} samples are kept.
 *
 * <p>Thread-safe.
 */
public final class AirTrack {

    public static final int AIR_RATE = 48_000;
    /** How far after its arrival a burst is heard. */
    public static final int LATENCY_MS = 100;
    static final long LATENCY = (long) AIR_RATE * LATENCY_MS / 1000;
    /** Samples kept (2 s). */
    static final int CAPACITY = AIR_RATE * 2;

    private final float[] ring = new float[CAPACITY];
    /** Next air sample to write; {@link Long#MIN_VALUE} before the first burst. */
    private long head = Long.MIN_VALUE;
    /** Resampler state: position in the incoming block (input samples), and the sample before it. */
    private double phase = -1;
    private float last;
    private int lastRate;

    private volatile String dimension = "";
    private volatile double x, y, z;
    private volatile long lastWriteNanos;

    /** Where the sound is (an eye, a speaker cone). */
    public void moveTo(String dimension, double x, double y, double z) {
        this.dimension = dimension;
        this.x = x;
        this.y = y;
        this.z = z;
    }

    public String dimension() { return dimension; }
    public double x() { return x; }
    public double y() { return y; }
    public double z() { return z; }

    /** Clock time of the last burst (for dropping silent sources). */
    public long lastWriteNanos() {
        return lastWriteNanos;
    }

    /**
     * Add a burst of 16-bit mono samples at {@code rate} that arrived at air
     * sample {@code nowAir}, scaled by {@code gain}.
     */
    public synchronized void append(short[] pcm, int off, int n, int rate, long nowAir, long nowNanos, float gain) {
        if (n <= 0 || rate <= 0) return;
        lastWriteNanos = nowNanos;
        if (head == Long.MIN_VALUE || head < nowAir) {
            // First burst, or the source went quiet and the air caught up: start again after the latency.
            long start = nowAir + LATENCY;
            if (head != Long.MIN_VALUE) clear(head, start);
            head = start;
            phase = -1;
            last = 0;
        } else if (head > nowAir + CAPACITY / 2) {
            // Writing faster than real time (a stalled clock): drop this burst rather than run away.
            return;
        }
        if (rate != lastRate) {
            phase = -1;
            lastRate = rate;
        }
        double step = (double) rate / AIR_RATE;
        float scale = gain / 32768f;
        while (phase < n - 1) {
            int i = (int) Math.floor(phase);
            float f = (float) (phase - i);
            float a = i < 0 ? last : pcm[off + i] * scale;
            float b = pcm[off + i + 1] * scale;
            ring[(int) Math.floorMod(head, (long) CAPACITY)] = a + (b - a) * f;
            head++;
            phase += step;
        }
        phase -= n;
        last = pcm[off + n - 1] * scale;
    }

    /** {@link #append(short[], int, int, int, long, long, float)} for float samples (±1). */
    public void append(float[] samples, int rate, long nowAir, long nowNanos) {
        short[] pcm = new short[samples.length];
        for (int i = 0; i < pcm.length; i++) pcm[i] = (short) Math.round(Math.max(-1, Math.min(1, samples[i])) * 32767);
        append(pcm, 0, pcm.length, rate, nowAir, nowNanos, 1f);
    }

    private void clear(long from, long to) {
        long n = Math.min(CAPACITY, to - from);
        for (long k = 0; k < n; k++) ring[(int) Math.floorMod(from + k, (long) CAPACITY)] = 0;
    }

    /** The sample at air index {@code air}; 0 where nothing was written (not yet, or too long ago). */
    synchronized float at(long air) {
        return valid(air) ? ring[(int) Math.floorMod(air, (long) CAPACITY)] : 0;
    }

    private boolean valid(long air) {
        return head != Long.MIN_VALUE && air < head && air >= head - CAPACITY;
    }

    /**
     * Add {@code n} samples heard at {@code rate} (absolute sample index {@code start} on the same clock)
     * times {@code gain} into {@code out}, interpolating between air samples.
     */
    public synchronized void addTo(float[] out, long start, int n, int rate, float gain) {
        if (head == Long.MIN_VALUE || gain == 0) return;
        double ratio = (double) AIR_RATE / rate;
        for (int k = 0; k < n; k++) {
            double t = (start + k) * ratio;
            long i = (long) Math.floor(t);
            if (i + 1 < head - CAPACITY || i >= head) continue;
            float f = (float) (t - i);
            float a = valid(i) ? ring[(int) Math.floorMod(i, (long) CAPACITY)] : 0;
            float b = valid(i + 1) ? ring[(int) Math.floorMod(i + 1, (long) CAPACITY)] : 0;
            out[k] += (a + (b - a) * f) * gain;
        }
    }

    /** Next air sample to be written (tests). */
    public synchronized long head() {
        return head;
    }
}
