package com.example.evanscomputermod.voice;

import java.util.Locale;
import java.util.SplittableRandom;
import java.util.function.LongSupplier;

/**
 * The sound card of one microphone: the capture side of {@code /dev/audio}.
 * Programs read PCM from {@code /dev/mic} and set it up through
 * {@code /dev/micctl}, like a real capture device.
 *
 * <ul>
 *   <li><b>Format</b>: {@code rate} 8000-48000 Hz, {@code bits} 8 (unsigned)
 *       or 16 (signed little-endian), {@code channels} 1 or 2 (a microphone is
 *       one capsule, so both channels carry the same signal). The format and
 *       gain belong to the card, so they are shared by everything reading it.</li>
 *   <li><b>Gain</b> 0-100 (input volume): 50 is the capsule's own level,
 *       every 25 above or below is ±10 dB, 0 mutes.</li>
 *   <li><b>Streams.</b> Each open {@code /dev/mic} is its own capture stream
 *       ({@link Stream}), as a sound server gives every recording program the
 *       whole signal: two programs recording at once both hear everything.</li>
 *   <li><b>Clock.</b> A stream starts capturing with its first read and runs
 *       at {@code rate} samples per second from then on. A read waits until
 *       the samples it asks for have been captured (at most half a second's
 *       worth per read); in non-blocking mode it returns what is there, or
 *       "would block" when nothing is.</li>
 *   <li><b>Overruns.</b> Captured audio waits for one second. A program that
 *       reads less often than that loses the oldest audio and counts an
 *       overrun, as a sound card's buffer overflows.</li>
 *   <li><b>Noise.</b> The capsule hisses faintly (about -75 dBFS), so a
 *       quiet room reads as quiet, not as digital zero.</li>
 * </ul>
 *
 * What the microphone hears comes from its {@link Hearing} (the
 * {@link SoundField} at its position). Thread-safe.
 */
public final class MicAudio {

    public static final int MIN_RATE = 8000;
    public static final int MAX_RATE = 48000;
    public static final int DEFAULT_RATE = 48000;
    public static final int DEFAULT_GAIN = 50;
    /** Capsule self-noise, RMS relative to full scale (about -75 dBFS). */
    static final double SELF_NOISE = 1.8e-4;
    /** A program that read within this long is "listening" (the capture LED). */
    static final long LISTENING_NANOS = 1_500_000_000L;

    /** What the microphone picks up: add {@code n} samples at {@code rate} from absolute sample {@code start} to {@code out}. */
    @FunctionalInterface
    public interface Hearing {
        void render(long start, int n, int rate, float[] out);
    }

    /** Thrown to a reader when the microphone is gone (broken, unloaded, detached). */
    public static final class ClosedException extends Exception {
        public ClosedException() {
            super("microphone removed");
        }
    }

    /** One reader's place in the capture (one open {@code /dev/mic}). */
    public static final class Stream {
        /** Next sample to deliver (absolute index at {@link #rate}); MIN_VALUE until capture starts. */
        private long cursor = Long.MIN_VALUE;
        private int rate;
        private long overruns;
    }

    private final Hearing hearing;
    private final LongSupplier clockNanos;
    private final SplittableRandom noise;
    /** The stream of callers that don't open their own (a radio's microphone, tests). */
    private final Stream own = new Stream();

    private int rate = DEFAULT_RATE;
    private int bits = 16;
    private int channels = 1;
    private int gain = DEFAULT_GAIN;

    private long overruns;
    private long samplesRead;
    private long lastReadNanos = Long.MIN_VALUE;
    private double lastPeak;
    private Stream lastStream = own;
    private boolean closed;
    /** Clock time of the last card-wide flush: every stream drops what it captured before it. */
    private long flushedNanos = Long.MIN_VALUE;

    public MicAudio(Hearing hearing) {
        this(hearing, SoundField.get()::nanos, System.nanoTime());
    }

    public MicAudio(Hearing hearing, LongSupplier clockNanos, long seed) {
        this.hearing = hearing;
        this.clockNanos = clockNanos;
        this.noise = new SplittableRandom(seed);
    }

    /** A new capture stream (what opening {@code /dev/mic} gets). */
    public Stream openStream() {
        return new Stream();
    }

    // ------------------------------------------------------------ settings

    public synchronized int rate() { return rate; }

    public synchronized int bits() { return bits; }

    public synchronized int channels() { return channels; }

    public synchronized int gain() { return gain; }

    /** Overruns of every stream so far. */
    public synchronized long overruns() { return overruns; }

    public synchronized long samplesRead() { return samplesRead; }

    /** Change the format. Capture restarts, as a format change on real hardware would. */
    public synchronized void setFormat(int newRate, int newBits, int newChannels) {
        if (newRate < MIN_RATE || newRate > MAX_RATE) throw new IllegalArgumentException("rate must be 8000-48000");
        if (newBits != 8 && newBits != 16) throw new IllegalArgumentException("bits must be 8 or 16");
        if (newChannels != 1 && newChannels != 2) throw new IllegalArgumentException("channels must be 1 or 2");
        rate = newRate;
        bits = newBits;
        channels = newChannels;
    }

    public synchronized void setGain(int g) {
        if (g < 0 || g > 100) throw new IllegalArgumentException("gain must be 0-100");
        gain = g;
    }

    /** Linear amplitude factor of a gain setting. */
    public static double gainFactor(int g) {
        return g <= 0 ? 0 : Math.pow(10, (g - 50) / 50.0);
    }

    /** Drop what {@code s} captured and did not read: its next read gets audio from now on. */
    public synchronized void flush(Stream s) {
        if (s.cursor != Long.MIN_VALUE) s.cursor = now();
        notifyAll();
    }

    /** Flush every stream ({@code flush} written to {@code /dev/micctl}). */
    public synchronized void flushAll() {
        flushedNanos = clockNanos.getAsLong();
        notifyAll();
    }

    /** {@link #flush(Stream)} of the shared stream. */
    public void flush() {
        flush(own);
    }

    public synchronized void close() {
        closed = true;
        notifyAll();
    }

    public synchronized boolean isClosed() {
        return closed;
    }

    /** A program read this microphone recently (it is recording). */
    public synchronized boolean listening() {
        return !closed && lastReadNanos != Long.MIN_VALUE && clockNanos.getAsLong() - lastReadNanos < LISTENING_NANOS;
    }

    // ------------------------------------------------------------ clock

    private long now() {
        return Math.floorDiv(clockNanos.getAsLong(), 1000L) * rate / 1_000_000L;
    }

    /** Frames {@code s} captured and has not read yet. */
    public synchronized long available(Stream s) {
        if (s.cursor == Long.MIN_VALUE || s.rate != rate) return 0;
        return Math.max(0, Math.min(rate, now() - s.cursor));
    }

    /** Milliseconds captured and not read yet by the stream that read last. */
    public synchronized int bufferedMs() {
        return (int) (available(lastStream) * 1000 / rate);
    }

    // ------------------------------------------------------------ reading

    /** Result of a non-blocking read that found nothing. */
    public static final int WOULD_BLOCK = -2;

    /** {@link #read(Stream, byte[], int, int, boolean)} on the shared stream. */
    public int read(byte[] buf, int off, int len, boolean nonBlocking) throws InterruptedException, ClosedException {
        return read(own, buf, off, len, nonBlocking);
    }

    /**
     * Read PCM in the current format into {@code buf}: whole frames only.
     * Blocking: waits until the frames asked for (at most half a second) are
     * captured. Non-blocking: returns the frames captured so far, or
     * {@link #WOULD_BLOCK}.
     *
     * @throws IllegalArgumentException {@code len} is smaller than one frame
     * @throws InterruptedException     the program is being killed
     * @throws ClosedException          the microphone is gone
     */
    public synchronized int read(Stream s, byte[] buf, int off, int len, boolean nonBlocking)
            throws InterruptedException, ClosedException {
        int frameBytes = bits / 8 * channels;
        int want = len / frameBytes;
        if (want <= 0) throw new IllegalArgumentException("buffer smaller than one frame");
        want = Math.min(want, rate / 2);
        lastReadNanos = clockNanos.getAsLong();
        lastStream = s;
        while (true) {
            if (closed) throw new ClosedException();
            long now = now();
            if (s.cursor == Long.MIN_VALUE || s.rate != rate) {
                // Capture starts (or restarts after a format change).
                s.cursor = now;
                s.rate = rate;
            }
            if (flushedNanos != Long.MIN_VALUE) {
                long f = Math.floorDiv(flushedNanos, 1000L) * rate / 1_000_000L;
                if (s.cursor < f) s.cursor = f;
            }
            long behind = now - s.cursor;
            if (behind > rate) {
                overruns++;
                s.overruns++;
                s.cursor = now - rate;
                behind = rate;
            }
            if (behind >= want || (nonBlocking && behind > 0)) break;
            if (nonBlocking) return WOULD_BLOCK;
            long waitMs = Math.max(1, Math.min(20, (want - behind) * 1000 / rate + 1));
            wait(waitMs);
            if (Thread.currentThread().isInterrupted()) throw new InterruptedException();
            frameBytes = bits / 8 * channels;   // another reader may have changed the format
            want = Math.min(Math.max(1, len / frameBytes), rate / 2);
        }
        int frames = (int) Math.min(want, now() - s.cursor);
        float[] heard = new float[frames];
        hearing.render(s.cursor, frames, rate, heard);
        double g = gainFactor(gain);
        double peak = 0;
        int p = off;
        for (int k = 0; k < frames; k++) {
            double v = (heard[k] + SELF_NOISE * gaussian()) * g;
            v = Math.max(-1, Math.min(1, v));
            peak = Math.max(peak, Math.abs(v));
            for (int c = 0; c < channels; c++) {
                if (bits == 8) {
                    buf[p++] = (byte) (128 + (int) Math.round(v * 127));
                } else {
                    int x = (int) Math.round(v * 32767);
                    buf[p++] = (byte) x;
                    buf[p++] = (byte) (x >> 8);
                }
            }
        }
        lastPeak = peak;
        s.cursor += frames;
        samplesRead += frames;
        lastReadNanos = clockNanos.getAsLong();
        return frames * frameBytes;
    }

    private double gaussian() {
        double u = Math.max(1e-12, noise.nextDouble());
        return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * noise.nextDouble());
    }

    /** Peak level of the last read, dBFS (-99 before the first). */
    public synchronized int levelDb() {
        return lastPeak <= 0 ? -99 : (int) Math.max(-99, Math.round(20 * Math.log10(lastPeak)));
    }

    /** {@code /dev/micctl} contents. */
    public synchronized String status() {
        return String.format(Locale.ROOT,
                "rate %d\nbits %d\nchannels %d\ngain %d\nbuffered_ms %d\noverruns %d\nlevel_db %d\nlistening %d\n",
                rate, bits, channels, gain, bufferedMs(), overruns, levelDb(), listening() ? 1 : 0);
    }

    /**
     * Apply one {@code /dev/micctl} command: {@code rate N}, {@code bits N},
     * {@code channels N}, {@code gain N}, {@code flush} (of {@code stream}; null: every stream).
     *
     * @throws IllegalArgumentException for a bad command or value (EINVAL)
     */
    public void control(String line, Stream stream) {
        String[] parts = line.trim().split("\\s+");
        if (parts.length == 0 || parts[0].isEmpty()) return;
        String cmd = parts[0].toLowerCase(Locale.ROOT);
        if (cmd.equals("flush")) {
            if (stream == null) flushAll();
            else flush(stream);
            return;
        }
        if (parts.length != 2) throw new IllegalArgumentException("usage: " + cmd + " <number>");
        int v;
        try {
            v = Integer.parseInt(parts[1]);
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("not a number: " + parts[1]);
        }
        synchronized (this) {
            switch (cmd) {
                case "rate" -> setFormat(v, bits, channels);
                case "bits" -> setFormat(rate, v, channels);
                case "channels" -> setFormat(rate, bits, v);
                case "gain", "volume" -> setGain(v);
                default -> throw new IllegalArgumentException("unknown command: " + cmd);
            }
        }
    }

    /** {@link #control(String, Stream)} for the shared stream. */
    public void control(String line) {
        control(line, own);
    }
}
