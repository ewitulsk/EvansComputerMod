package com.example.evanscomputermod.speaker;

import java.util.Locale;

/**
 * The sound card of one Speaker: the PCM stream programs write to through
 * {@code /dev/audio}, with a playback clock like real audio hardware.
 *
 * <ul>
 *   <li><b>Format</b> ({@code /dev/audioctl}): {@code rate} 8000-48000 Hz,
 *       {@code bits} 8 (unsigned) or 16 (signed little-endian),
 *       {@code channels} 1 or 2 (interleaved; a speaker is one point source,
 *       so stereo is mixed down to mono). {@code volume} 0-100.</li>
 *   <li><b>Clock.</b> Once samples are queued the speaker plays them at
 *       {@code rate} samples per second. A write that would put more than
 *       {@code latency} ms of audio in the queue waits until enough has played,
 *       so a program that just writes is paced by the audio clock. In
 *       non-blocking mode the write takes what fits and returns.</li>
 *   <li><b>Underruns.</b> If the queue runs dry the speaker plays silence and
 *       counts an underrun; the next write starts the clock again.</li>
 * </ul>
 *
 * Samples are converted to 16-bit mono as they are written and kept until the
 * server tick sends them to listeners ({@link #drain()}).
 *
 * <p>Thread-safe: programs write on their own threads, the server drains.
 */
public final class SpeakerAudio {

    public static final int MIN_RATE = 8000;
    public static final int MAX_RATE = 48000;
    public static final int DEFAULT_RATE = 48000;
    public static final int DEFAULT_LATENCY_MS = 100;
    public static final int MIN_LATENCY_MS = 20;
    public static final int MAX_LATENCY_MS = 1000;

    /** Thrown to a writer when the speaker is gone (broken, unloaded, detached). */
    public static final class ClosedException extends Exception {
        public ClosedException() {
            super("speaker removed");
        }
    }

    private int rate = DEFAULT_RATE;
    private int bits = 16;
    private int channels = 1;
    private int volume = 100;
    private int latencyMs = DEFAULT_LATENCY_MS;

    // Playback clock, in samples at `rate`.
    private long written;
    private long baseWritten;
    private long baseNanos;
    private boolean running;
    private long underruns;

    /** Bytes of a frame left over from the previous write. */
    private final byte[] carry = new byte[4];
    private int carryLen;

    /** Converted samples waiting for the next server tick. */
    private short[] pending = new short[4096];
    private int pendingLen;

    private boolean closed;

    // ------------------------------------------------------------ settings

    public synchronized int rate() { return rate; }

    public synchronized int bits() { return bits; }

    public synchronized int channels() { return channels; }

    public synchronized int volume() { return volume; }

    public synchronized int latencyMs() { return latencyMs; }

    public synchronized long underruns() { return underruns; }

    /** Change the format. The queue is dropped, as a format change on real hardware would. */
    public synchronized void setFormat(int newRate, int newBits, int newChannels) {
        if (newRate < MIN_RATE || newRate > MAX_RATE) throw new IllegalArgumentException("rate must be 8000-48000");
        if (newBits != 8 && newBits != 16) throw new IllegalArgumentException("bits must be 8 or 16");
        if (newChannels != 1 && newChannels != 2) throw new IllegalArgumentException("channels must be 1 or 2");
        if (newRate == rate && newBits == bits && newChannels == channels) return;
        flushLocked();
        rate = newRate;
        bits = newBits;
        channels = newChannels;
    }

    public synchronized void setVolume(int v) {
        if (v < 0 || v > 100) throw new IllegalArgumentException("volume must be 0-100");
        volume = v;
    }

    public synchronized void setLatencyMs(int ms) {
        if (ms < MIN_LATENCY_MS || ms > MAX_LATENCY_MS) throw new IllegalArgumentException("latency must be 20-1000 ms");
        latencyMs = ms;
        notifyAll();
    }

    /** Drop everything queued: silence now. */
    public synchronized void flush() {
        flushLocked();
        notifyAll();
    }

    private void flushLocked() {
        long now = System.nanoTime();
        written = played(now);
        running = false;
        carryLen = 0;
    }

    public synchronized void close() {
        closed = true;
        notifyAll();
    }

    public synchronized boolean isClosed() {
        return closed;
    }

    // ------------------------------------------------------------ clock

    private long played(long now) {
        if (!running) return written;
        long p = baseWritten + (now - baseNanos) * rate / 1_000_000_000L;
        return Math.min(written, p);
    }

    /** Milliseconds of audio queued and not yet played. */
    public synchronized int bufferedMs() {
        return (int) ((written - played(System.nanoTime())) * 1000 / rate);
    }

    private long capacitySamples() {
        return (long) rate * latencyMs / 1000;
    }

    // ------------------------------------------------------------ writing

    /**
     * Queue {@code len} bytes of PCM in the current format. Blocking: waits for
     * room and returns {@code len}. Non-blocking: queues what fits now and
     * returns how many bytes were taken (0 = the queue is full, try again).
     *
     * @throws InterruptedException the program is being killed
     * @throws ClosedException      the speaker is gone
     */
    public synchronized int write(byte[] data, int off, int len, boolean nonBlocking)
            throws InterruptedException, ClosedException {
        int consumed = 0;
        int frameBytes = (bits / 8) * channels;
        while (consumed < len) {
            if (closed) throw new ClosedException();
            long now = System.nanoTime();
            if (!running || played(now) >= written) {
                if (running) underruns++;
                running = true;
                baseNanos = now;
                baseWritten = written;
            }
            long room = capacitySamples() - (written - played(now));
            if (room <= 0) {
                if (nonBlocking) return consumed;
                // Sleep until about one 10 ms slice has played.
                long waitMs = Math.max(1, Math.min(10, (1 - room) * 1000 / rate + 1));
                wait(waitMs);
                if (Thread.currentThread().isInterrupted()) throw new InterruptedException();
                continue;
            }
            // Whole frames we can take: carried bytes first.
            int availFrames = (carryLen + (len - consumed)) / frameBytes;
            int frames = (int) Math.min(room, availFrames);
            if (frames == 0) {
                // Less than a frame left: keep it for the next write.
                int n = len - consumed;
                System.arraycopy(data, off + consumed, carry, carryLen, n);
                carryLen += n;
                consumed = len;
                break;
            }
            ensurePending(frames);
            for (int f = 0; f < frames; f++) {
                pending[pendingLen++] = (short) (readFrame(data, off, consumed, frameBytes) * volume / 100);
                consumed += frameBytes - carryLen;
                carryLen = 0;
            }
            written += frames;
        }
        return consumed;
    }

    /** One frame as a 16-bit mono sample; uses carried bytes first. */
    private int readFrame(byte[] data, int off, int consumed, int frameBytes) {
        int sum = 0;
        int bytesPerSample = bits / 8;
        for (int c = 0; c < channels; c++) {
            int s;
            if (bytesPerSample == 1) {
                s = ((byteAt(data, off, consumed, c) & 0xFF) - 128) << 8;
            } else {
                int lo = byteAt(data, off, consumed, c * 2) & 0xFF;
                int hi = byteAt(data, off, consumed, c * 2 + 1);
                s = (short) (lo | (hi << 8));
            }
            sum += s;
        }
        return sum / channels;
    }

    private byte byteAt(byte[] data, int off, int consumed, int i) {
        return i < carryLen ? carry[i] : data[off + consumed + i - carryLen];
    }

    private void ensurePending(int more) {
        if (pendingLen + more > pending.length) {
            // The server drains every tick; this only grows if it stalls.
            int cap = Math.max(pending.length * 2, pendingLen + more);
            if (cap > MAX_RATE * 4) {
                // Server stalled for seconds: keep the newest audio.
                int keep = Math.max(0, MAX_RATE * 2 - more);
                System.arraycopy(pending, pendingLen - keep, pending, 0, keep);
                pendingLen = keep;
                cap = Math.max(pending.length, keep + more);
            }
            short[] n = new short[cap];
            System.arraycopy(pending, 0, n, 0, pendingLen);
            pending = n;
        }
    }

    // ------------------------------------------------------------ server side

    /** Samples written since the last drain, as 16-bit mono at {@link #rate()}. */
    public synchronized short[] drain() {
        short[] out = new short[pendingLen];
        System.arraycopy(pending, 0, out, 0, pendingLen);
        pendingLen = 0;
        notifyAll();
        return out;
    }

    /** {@code /dev/audioctl} contents. */
    public synchronized String status() {
        return String.format(Locale.ROOT,
                "rate %d\nbits %d\nchannels %d\nvolume %d\nlatency %d\nbuffered_ms %d\nunderruns %d\n",
                rate, bits, channels, volume, latencyMs, bufferedMs(), underruns);
    }

    /**
     * Apply one {@code /dev/audioctl} command: {@code rate N}, {@code bits N},
     * {@code channels N}, {@code volume N}, {@code latency N}, {@code flush}.
     *
     * @throws IllegalArgumentException for a bad command or value (EINVAL)
     */
    public void control(String line) {
        String[] parts = line.trim().split("\\s+");
        if (parts.length == 0 || parts[0].isEmpty()) return;
        String cmd = parts[0].toLowerCase(Locale.ROOT);
        if (cmd.equals("flush")) {
            flush();
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
                case "volume" -> setVolume(v);
                case "latency" -> setLatencyMs(v);
                default -> throw new IllegalArgumentException("unknown command: " + cmd);
            }
        }
    }
}
