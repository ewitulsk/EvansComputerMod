package com.example.evanscomputermod.speaker;

import java.util.Locale;

/**
 * One stream of a sound card ({@link SpeakerAudio}): what one open of {@code /dev/audio}
 * writes, in its own format, with its own playback clock and its own role
 * ({@code media}, {@code voice}, {@code phone}). The card resamples every stream to its
 * output rate and mixes them (like ALSA dmix), ducking lower roles while a higher one
 * carries sound (docs/gui/APPS.md "Audio").
 *
 * <ul>
 *   <li><b>Format</b> ({@code /dev/audioctl}): {@code rate} 8000-48000 Hz, {@code bits} 8
 *       (unsigned) or 16 (signed little-endian), {@code channels} 1 or 2 (interleaved; a
 *       speaker is one point source, so stereo is mixed down to mono), {@code volume}
 *       0-100 (on top of the speaker's own volume), {@code latency} 20-1000 ms,
 *       {@code role media|voice|phone}.</li>
 *   <li><b>Clock.</b> Once samples are queued the stream plays them at {@code rate} samples
 *       per second. A write that would put more than {@code latency} ms of audio in the
 *       queue waits until enough has played, so a program that just writes is paced by the
 *       audio clock. In non-blocking mode the write takes what fits and returns.</li>
 *   <li><b>Underruns.</b> If the queue runs dry the stream plays silence and counts an
 *       underrun; the next write starts the clock again.</li>
 * </ul>
 *
 * All state is guarded by the card's monitor: writers wait on it, the server tick mixes
 * under it.
 */
public final class AudioStream {

    /** Who a stream is for: higher roles duck lower ones while they carry sound. */
    public enum Role {
        MEDIA("media"), VOICE("voice"), PHONE("phone");

        private final String id;

        Role(String id) {
            this.id = id;
        }

        public String id() {
            return id;
        }

        public static Role parse(String s) {
            for (Role r : values()) if (r.id.equals(s.toLowerCase(Locale.ROOT))) return r;
            throw new IllegalArgumentException("role must be media, voice or phone");
        }
    }

    final SpeakerAudio card;
    final int pid;
    int rate = SpeakerAudio.DEFAULT_RATE;
    int bits = 16;
    int channels = 1;
    int volume = 100;
    int latencyMs = SpeakerAudio.DEFAULT_LATENCY_MS;
    Role role = Role.MEDIA;
    boolean closed;

    // Playback clock, in samples at `rate`: index anchorIndex plays at anchorNanos.
    long anchorNanos;
    long anchorIndex;
    /** Index of the next sample a write adds. */
    long written;
    boolean running;
    long underruns;

    /** Converted (mono, 16-bit, stream volume applied) samples not yet mixed: indices qStart .. written-1. */
    short[] queue = new short[4096];
    long qStart;
    int qLen;

    /** Bytes of a frame left over from the previous write. */
    final byte[] carry = new byte[4];
    int carryLen;

    // Mixing (card side): where in this stream the card's next output sample falls.
    boolean cursorValid;
    double cursor;
    /** Ducking gain now (1 = full) and until when this stream counts as carrying sound. */
    double gain = 1;
    long loudUntilNanos;
    boolean ducked;
    /** The Volume Mixer's strip for this stream's app: its volume (0-1.5) and mute. */
    double appGain = 1;
    boolean muted;
    /** Peak of what this stream put into the last mix (after its app volume), 0-32767. */
    int peak;

    AudioStream(SpeakerAudio card, int pid) {
        this.card = card;
        this.pid = pid;
    }

    /** The program that opened the stream (0 = the card's own default stream). */
    public int pid() {
        return pid;
    }

    // ------------------------------------------------------------ settings

    public int rate() {
        synchronized (card) {
            return rate;
        }
    }

    public int bits() {
        synchronized (card) {
            return bits;
        }
    }

    public int channels() {
        synchronized (card) {
            return channels;
        }
    }

    public int volume() {
        synchronized (card) {
            return volume;
        }
    }

    public int latencyMs() {
        synchronized (card) {
            return latencyMs;
        }
    }

    public Role role() {
        synchronized (card) {
            return role;
        }
    }

    public long underruns() {
        synchronized (card) {
            return underruns;
        }
    }

    /** True while a higher-role stream carries sound and this one plays quieter. */
    public boolean ducked() {
        synchronized (card) {
            return ducked;
        }
    }

    /** The Volume Mixer's setting for this stream's app: volume in percent (0-150) and mute. */
    public void setMixer(int percent, boolean mute) {
        synchronized (card) {
            appGain = Math.max(0, Math.min(150, percent)) / 100.0;
            muted = mute;
        }
    }

    /** Give {@code other} (the same program's stream on another speaker) this stream's settings and mixer strip. */
    void copyTo(AudioStream other) {
        int r, b, c, v, l;
        Role ro;
        double g;
        boolean m;
        synchronized (card) {
            r = rate;
            b = bits;
            c = channels;
            v = volume;
            l = latencyMs;
            ro = role;
            g = appGain;
            m = muted;
        }
        synchronized (other.card) {
            other.rate = r;
            other.bits = b;
            other.channels = c;
            other.volume = v;
            other.latencyMs = l;
            other.role = ro;
            other.appGain = g;
            other.muted = m;
        }
    }

    /** When {@link #peak} was measured. */
    long peakAtNanos;

    /** Peak of this stream in the last mix (0-32767; the Volume Mixer's meter, held for 300 ms). */
    public int peak() {
        synchronized (card) {
            return System.nanoTime() - peakAtNanos < SpeakerAudio.METER_HOLD_NANOS ? peak : 0;
        }
    }

    public boolean isClosed() {
        synchronized (card) {
            return closed || card.closed;
        }
    }

    /** Change the format. The queue is dropped, as a format change on real hardware would. */
    public void setFormat(int newRate, int newBits, int newChannels) {
        if (newRate < SpeakerAudio.MIN_RATE || newRate > SpeakerAudio.MAX_RATE) throw new IllegalArgumentException("rate must be 8000-48000");
        if (newBits != 8 && newBits != 16) throw new IllegalArgumentException("bits must be 8 or 16");
        if (newChannels != 1 && newChannels != 2) throw new IllegalArgumentException("channels must be 1 or 2");
        synchronized (card) {
            if (newRate == rate && newBits == bits && newChannels == channels) return;
            flushLocked();
            rate = newRate;
            bits = newBits;
            channels = newChannels;
            card.notifyAll();
        }
    }

    public void setVolume(int v) {
        if (v < 0 || v > 100) throw new IllegalArgumentException("volume must be 0-100");
        synchronized (card) {
            volume = v;
        }
    }

    public void setLatencyMs(int ms) {
        if (ms < SpeakerAudio.MIN_LATENCY_MS || ms > SpeakerAudio.MAX_LATENCY_MS) throw new IllegalArgumentException("latency must be 20-1000 ms");
        synchronized (card) {
            latencyMs = ms;
            card.notifyAll();
        }
    }

    public void setRole(Role r) {
        synchronized (card) {
            role = r;
        }
    }

    /** Drop everything queued: silence now. */
    public void flush() {
        synchronized (card) {
            flushLocked();
            card.notifyAll();
        }
    }

    void flushLocked() {
        qStart = written;
        qLen = 0;
        running = false;
        carryLen = 0;
        cursorValid = false;
        loudUntilNanos = 0;
    }

    /** The stream is done (its fd closed): what is queued still plays out, nothing more is taken. */
    public void close() {
        synchronized (card) {
            closed = true;
            card.notifyAll();
        }
    }

    // ------------------------------------------------------------ clock

    long played(long now) {
        if (!running) return written;
        long p = anchorIndex + (now - anchorNanos) * rate / 1_000_000_000L;
        return Math.min(written, p);
    }

    /** Milliseconds of audio queued and not yet played. */
    public int bufferedMs() {
        synchronized (card) {
            return (int) ((written - played(System.nanoTime())) * 1000 / rate);
        }
    }

    private long capacitySamples() {
        return (long) rate * latencyMs / 1000;
    }

    /** Wall time (nanos) at which sample {@code index} plays. */
    long timeOf(long index) {
        return anchorNanos + (index - anchorIndex) * 1_000_000_000L / rate;
    }

    /** (Re)start the clock after an underrun or at the first write: never before what the card already sent. */
    private void startClock(long now) {
        long t0 = Math.max(now, card.mixStartFloor(now));
        if (running && qLen > 0) {
            // Samples nobody mixed yet are still queued: keep the old timeline and pad the gap with silence.
            long pad = anchorIndex + (t0 - anchorNanos) * rate / 1_000_000_000L - written;
            if (pad >= 0 && pad <= rate) {
                ensureQueue((int) pad);
                for (int i = 0; i < pad; i++) queue[qLen++] = 0;
                written += pad;
                return;
            }
            qStart = written;
            qLen = 0;
            cursorValid = false;
        }
        anchorNanos = t0;
        anchorIndex = written;
    }

    // ------------------------------------------------------------ writing

    /**
     * Queue {@code len} bytes of PCM in the current format. Blocking: waits for room and
     * returns {@code len}. Non-blocking: queues what fits now and returns how many bytes
     * were taken (0 = the queue is full, try again).
     *
     * @throws InterruptedException            the program is being killed
     * @throws SpeakerAudio.ClosedException the speaker is gone
     */
    public int write(byte[] data, int off, int len, boolean nonBlocking) throws InterruptedException, SpeakerAudio.ClosedException {
        // Told outside the card's lock: the listener may start a background mode (and that may ask the player).
        SpeakerAudio.WriteListener l = card.writeListener;
        if (l != null) l.written(pid, role());
        synchronized (card) {
            int consumed = 0;
            int frameBytes = (bits / 8) * channels;
            while (consumed < len) {
                if (closed || card.closed) throw new SpeakerAudio.ClosedException();
                long now = System.nanoTime();
                if (!running || played(now) >= written) {
                    if (running) underruns++;
                    startClock(now);
                    running = true;
                }
                long room = capacitySamples() - (written - played(now));
                if (room <= 0) {
                    if (nonBlocking) return consumed;
                    // Sleep until about one 10 ms slice has played.
                    long waitMs = Math.max(1, Math.min(10, (1 - room) * 1000 / rate + 1));
                    card.wait(waitMs);
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
                ensureQueue(frames);
                for (int f = 0; f < frames; f++) {
                    queue[qLen++] = (short) (readFrame(data, off, consumed, frameBytes) * volume / 100);
                    consumed += frameBytes - carryLen;
                    carryLen = 0;
                }
                written += frames;
            }
            return consumed;
        }
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

    private void ensureQueue(int more) {
        if (qLen + more <= queue.length) return;
        int cap = Math.max(queue.length * 2, qLen + more);
        if (cap > SpeakerAudio.MAX_RATE * 4) {
            // The server stalled for seconds: keep the newest audio.
            int keep = Math.max(0, SpeakerAudio.MAX_RATE * 2 - more);
            int drop = qLen - Math.min(keep, qLen);
            System.arraycopy(queue, drop, queue, 0, qLen - drop);
            qLen -= drop;
            qStart += drop;
            cursorValid = false;
            cap = Math.max(queue.length, qLen + more);
        }
        short[] n = new short[cap];
        System.arraycopy(queue, 0, n, 0, qLen);
        queue = n;
    }

    /** Sample {@code i} of the stream (0 outside what is queued). */
    int sampleAt(long i) {
        long k = i - qStart;
        return k < 0 || k >= qLen ? 0 : queue[(int) k];
    }

    /** Forget queued samples before {@code index}. */
    void consumeBefore(long index) {
        long drop = Math.min(qLen, index - qStart);
        if (drop <= 0) return;
        System.arraycopy(queue, (int) drop, queue, 0, qLen - (int) drop);
        qLen -= (int) drop;
        qStart += drop;
    }

    // ------------------------------------------------------------ ctl

    /** {@code /dev/audioctl} contents for this stream. */
    public String status() {
        synchronized (card) {
            return String.format(Locale.ROOT,
                    "rate %d\nbits %d\nchannels %d\nvolume %d\nlatency %d\nbuffered_ms %d\nunderruns %d\nrole %s\nducked %d\n",
                    rate, bits, channels, volume, latencyMs, (int) ((written - played(System.nanoTime())) * 1000 / rate), underruns,
                    role.id(), ducked ? 1 : 0);
        }
    }

    /**
     * Apply one {@code /dev/audioctl} command: {@code rate N}, {@code bits N},
     * {@code channels N}, {@code volume N}, {@code latency N}, {@code role R}, {@code flush}.
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
        if (parts.length != 2) throw new IllegalArgumentException("usage: " + cmd + (cmd.equals("role") ? " media|voice|phone" : " <number>"));
        if (cmd.equals("role")) {
            setRole(Role.parse(parts[1]));
            return;
        }
        int v;
        try {
            v = Integer.parseInt(parts[1]);
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("not a number: " + parts[1]);
        }
        synchronized (card) {
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

    @Override
    public String toString() {
        synchronized (card) {
            return "AudioStream[pid " + pid + ", " + role.id() + ", " + rate + " Hz" + (closed ? ", closed" : "") + "]";
        }
    }
}
