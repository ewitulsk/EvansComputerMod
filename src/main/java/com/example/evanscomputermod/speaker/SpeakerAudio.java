package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.EcmConfig;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The sound card of one speaker (a Speaker block, an ePhone, AirPods through the phone): a
 * mixer of {@link AudioStream}s. Every open of {@code /dev/audio} is its own stream with its
 * own format and clock ({@link #openStream}); the server tick ({@link #drain()}) resamples
 * them to one output rate and mixes them, like ALSA's dmix.
 *
 * <ul>
 *   <li><b>Per-stream control.</b> {@code /dev/audioctl} opened by a program sets that
 *       program's streams ({@link #controlFor}): format, volume, latency and
 *       {@code role media|voice|phone}. Reads report the stream's settings, {@code role}
 *       and {@code ducked}.</li>
 *   <li><b>Ducking.</b> While a stream carries sound (louder than -50 dBFS over the tick)
 *       every stream with a lower role ({@code media < voice < phone}) is lowered to
 *       {@code audio.duckLevelPercent} (25 %) with a 150 ms fade, and brought back 400 ms
 *       after it goes quiet. A Discord call is {@code voice}, so music only dips while
 *       someone in the call talks; cellular calls and ringtones are {@code phone}.</li>
 *   <li><b>Timeline.</b> Streams are aligned on wall time. A stream that starts while others
 *       play joins at the point the card has already sent (never in the past), so it is
 *       never cut; the card sends at most {@link #MIX_LEAD_MS} ahead of real time, which
 *       keeps a voice stream's latency low while music writes far ahead.</li>
 *   <li><b>The card's own stream.</b> The single-stream methods ({@link #write},
 *       {@link #setFormat}, {@link #control}, ...) act on a default stream for callers that
 *       own the card outright (tests, older hosts).</li>
 * </ul>
 * The speaker's own {@link #volume()} (the peripheral's, saved with a Speaker block) scales
 * the mix.
 *
 * <p>Thread-safe: programs write on their own threads, the server drains; one monitor.
 */
public final class SpeakerAudio {

    public static final int MIN_RATE = 8000;
    public static final int MAX_RATE = 48000;
    public static final int DEFAULT_RATE = 48000;
    public static final int DEFAULT_LATENCY_MS = 100;
    public static final int MIN_LATENCY_MS = 20;
    public static final int MAX_LATENCY_MS = 1000;
    /** How far ahead of real time the card sends mixed audio. */
    public static final long MIX_LEAD_MS = 200;
    /** Meters show a peak this long after it was measured. */
    static final long METER_HOLD_NANOS = 300_000_000L;
    private long peakAtNanos;
    /** A stream "carries sound" above this RMS (-50 dBFS). */
    static final double LOUD_RMS = 32768 * Math.pow(10, -50 / 20.0);
    /** Ducking fades over this long, and lets go this long after the louder stream went quiet. */
    static final long DUCK_FADE_MS = 150, DUCK_HOLD_MS = 400;

    /** Thrown to a writer when the speaker is gone (broken, unloaded, detached). */
    public static final class ClosedException extends Exception {
        public ClosedException() {
            super("speaker removed");
        }
    }

    /** Told on every write, on the writer's thread, outside the card's lock: who wrote, as what. */
    @FunctionalInterface
    public interface WriteListener {
        void written(int pid, AudioStream.Role role);
    }

    private int volume = 100;
    private boolean muted;
    /** Peak of the last mix (0-32767; the Volume Mixer's master meter). */
    private int peak;
    boolean closed;
    volatile WriteListener writeListener;

    private final List<AudioStream> streams = new ArrayList<>();
    private AudioStream defaultStream;
    /** Settings a program gave its {@code /dev/audioctl} before (or while) it has streams: new streams start with them. */
    private final Map<Integer, AudioStream> templates = new LinkedHashMap<>(16, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<Integer, AudioStream> e) {
            return size() > 64;
        }
    };

    // Output timeline: output sample `emitBaseSamples` plays at `emitBaseNanos`, at `outRate`.
    private int outRate = DEFAULT_RATE;
    private long emitBaseNanos, emitBaseSamples, emitSamples;
    /** Samples handed to {@link #drain()} so far (a speaker with no listeners still counts). */
    private long drained;
    /** Duck level override for tests (-1 = the config). */
    private volatile int duckPercent = -1;

    // ------------------------------------------------------------ streams

    /** A new stream for program {@code pid} (an open of {@code /dev/audio}), with the settings its audioctl gave. */
    public synchronized AudioStream openStream(int pid) {
        AudioStream s = new AudioStream(this, pid);
        AudioStream t = pid == 0 ? null : templates.get(pid);
        if (t != null) copySettings(t, s);
        streams.add(s);
        return s;
    }

    private static void copySettings(AudioStream from, AudioStream to) {
        to.rate = from.rate;
        to.bits = from.bits;
        to.channels = from.channels;
        to.volume = from.volume;
        to.latencyMs = from.latencyMs;
        to.role = from.role;
    }

    /** Open streams (tests, diagnostics). */
    public synchronized List<AudioStream> streams() {
        return List.copyOf(streams);
    }

    private synchronized AudioStream defaultStream() {
        if (defaultStream == null || defaultStream.closed) defaultStream = openStream(0);
        return defaultStream;
    }

    /** The newest open stream of {@code pid}, or null. */
    private AudioStream latestOf(int pid) {
        for (int i = streams.size() - 1; i >= 0; i--) {
            AudioStream s = streams.get(i);
            if (s.pid == pid && !s.closed) return s;
        }
        return null;
    }

    /**
     * {@code /dev/audioctl} written by program {@code pid}: applies to every open stream of
     * it and to the streams it opens later. Pid 0 = the card's own default stream.
     *
     * @throws IllegalArgumentException for a bad command or value (EINVAL)
     */
    public void controlFor(int pid, String line) {
        if (pid == 0) {
            defaultStream().control(line);
            return;
        }
        List<AudioStream> targets = new ArrayList<>();
        synchronized (this) {
            AudioStream t = templates.computeIfAbsent(pid, p -> new AudioStream(this, p));
            targets.add(t);
            for (AudioStream s : streams) if (s.pid == pid && !s.closed) targets.add(s);
        }
        // The template first: a bad value fails before any stream changed.
        for (AudioStream s : targets) s.control(line);
    }

    /** {@code /dev/audioctl} read by program {@code pid}: its newest stream's settings and counters. */
    public String statusFor(int pid) {
        AudioStream s;
        synchronized (this) {
            s = pid == 0 ? defaultStream() : latestOf(pid);
            if (s == null) s = templates.get(pid);
        }
        return (s != null ? s.status() : new AudioStream(this, pid).status()) + statusExtra.get();
    }

    // ------------------------------------------------------------ the card

    public synchronized int volume() {
        return volume;
    }

    /** The speaker's own volume (the peripheral's), on top of each stream's. */
    public synchronized void setVolume(int v) {
        if (v < 0 || v > 100) throw new IllegalArgumentException("volume must be 0-100");
        volume = v;
    }

    /** The master strip's mute (the Volume Mixer): the speaker plays silence, programs keep running. */
    public synchronized void setMuted(boolean m) {
        muted = m;
    }

    public synchronized boolean muted() {
        return muted;
    }

    /** Peak of the last mix, 0-32767 (held for 300 ms). */
    public synchronized int peak() {
        return System.nanoTime() - peakAtNanos < METER_HOLD_NANOS ? peak : 0;
    }

    /** Drop everything queued on every stream: silence now. */
    public synchronized void flush() {
        for (AudioStream s : streams) s.flushLocked();
        notifyAll();
    }

    public synchronized void close() {
        closed = true;
        notifyAll();
    }

    public synchronized boolean isClosed() {
        return closed;
    }

    /** The rate of what {@link #drain()} returned last. */
    public synchronized int outputRate() {
        return outRate;
    }

    public void setWriteListener(WriteListener l) {
        writeListener = l;
    }

    /** A listener that only wants to know that someone wrote. */
    public void setWriteListener(Runnable l) {
        writeListener = l == null ? null : (pid, role) -> l.run();
    }

    /** Samples played so far (handed to the server tick by {@link #drain()}). */
    public synchronized long samplesDrained() {
        return drained;
    }

    /** Tests: duck to {@code percent} instead of {@code audio.duckLevelPercent} (-1 = the config). */
    public void setDuckPercent(int percent) {
        duckPercent = percent;
    }

    private double duckLevel() {
        int p = duckPercent >= 0 ? duckPercent : EcmConfig.audioDuckLevelPercent();
        return Math.max(0, Math.min(100, p)) / 100.0;
    }

    private volatile java.util.function.Supplier<String> statusExtra = () -> "";
    private volatile java.util.function.Supplier<String> label = () -> "Speaker";

    /** What the Volume Mixer calls this speaker ("Speaker", "ePhone speaker", "AirPods" while they play). */
    public String label() {
        String l = label.get();
        return l == null || l.isBlank() ? "Speaker" : l;
    }

    public void setLabel(java.util.function.Supplier<String> l) {
        label = l == null ? () -> "Speaker" : l;
    }

    /**
     * More {@code key value} lines for {@code /dev/audioctl}, from the device that owns this
     * sound card (an ePhone adds {@code headphones 1} while its audio goes to AirPods).
     */
    public void setStatusExtra(java.util.function.Supplier<String> extra) {
        statusExtra = extra == null ? () -> "" : extra;
    }

    // ------------------------------------------------------------ mixing (server side)

    private long emitTime() {
        return emitBaseNanos + (emitSamples - emitBaseSamples) * 1_000_000_000L / outRate;
    }

    private void rebase(long nanos, int rate) {
        emitBaseNanos = nanos;
        emitBaseSamples = emitSamples;
        outRate = rate;
    }

    /** Where a stream starting now joins: the end of what the card already sent, while it is mixing. */
    long mixStartFloor(long now) {
        for (AudioStream s : streams) if (s.cursorValid) return emitTime();
        return now;
    }

    /** Stream {@code s}'s position (in its samples) at wall time {@code t}. */
    private static double positionAt(AudioStream s, long t) {
        double p = s.anchorIndex + (double) (t - s.anchorNanos) * s.rate / 1e9;
        double r = Math.rint(p);
        return Math.abs(p - r) < 0.01 ? r : p;
    }

    /**
     * Everything mixed since the last drain, as 16-bit mono at {@link #outputRate()}: each
     * stream resampled, ducked by louder higher roles, summed and scaled by the volume.
     */
    public synchronized short[] drain() {
        long now = System.nanoTime();
        lastLive = false;
        streams.removeIf(s -> s.closed && s.qLen == 0);
        List<AudioStream> live = new ArrayList<>();
        for (AudioStream s : streams) {
            if (s.qLen > 0) live.add(s);
            else s.cursorValid = false;
        }
        if (live.isEmpty()) {
            updateDucking(now, List.of());
            notifyAll();
            return new short[0];
        }
        int rate = 0;
        boolean mixing = false;
        for (AudioStream s : live) {
            rate = Math.max(rate, s.rate);
            mixing |= s.cursorValid;
        }
        if (!mixing) {
            // Nothing was playing: start at the earliest queued sample.
            long first = Long.MAX_VALUE;
            for (AudioStream s : live) first = Math.min(first, s.timeOf(s.qStart));
            rebase(first, rate);
        } else if (rate > outRate) {
            // Only up while mixing: listeners restart their stream on a rate change, so it stays put when a fast stream ends.
            rebase(emitTime(), rate);
        }
        long e = emitTime();
        lastDrainStartNanos = e;
        long n = (now + MIX_LEAD_MS * 1_000_000L - e) * outRate / 1_000_000_000L;
        List<AudioStream> active = new ArrayList<>();
        for (AudioStream s : live) {
            if (!s.cursorValid) {
                s.cursor = positionAt(s, e);
                s.cursorValid = true;
            }
            double step = (double) s.rate / outRate;
            double last = s.written - 1;
            if (s.cursor > last + 1e-9) {
                // Run dry: what is left lies behind the card's position.
                s.consumeBefore((long) Math.floor(s.cursor));
                s.cursorValid = false;
                continue;
            }
            long avail = (long) Math.floor((last - s.cursor) / step + 1e-9) + 1;
            n = Math.min(n, avail);
            active.add(s);
        }
        if (n <= 0 || active.isEmpty()) {
            updateDucking(now, List.of());
            return new short[0];
        }
        int count = (int) Math.min(n, MAX_RATE);
        double[][] parts = new double[active.size()][];
        for (int a = 0; a < active.size(); a++) {
            AudioStream s = active.get(a);
            double step = (double) s.rate / outRate;
            double fader = s.muted ? 0 : s.appGain;
            double[] out = new double[count];
            double sq = 0, pk = 0;
            for (int k = 0; k < count; k++) {
                double p = s.cursor + k * step;
                long i0 = (long) Math.floor(p);
                double f = p - i0;
                double v = s.sampleAt(i0);
                if (f > 1e-9) v = v * (1 - f) + s.sampleAt(i0 + 1) * f;
                v *= fader;
                out[k] = v;
                sq += v * v;
                pk = Math.max(pk, Math.abs(v));
            }
            s.cursor += count * step;
            s.consumeBefore((long) Math.floor(s.cursor));
            // What the listener hears counts: a muted call doesn't duck the music.
            if (Math.sqrt(sq / count) > LOUD_RMS) s.loudUntilNanos = now + DUCK_HOLD_MS * 1_000_000L;
            s.peak = (int) Math.min(32767, pk);
            s.peakAtNanos = now;
            if (pk > 0 && isLiveRole(s.role.name().toLowerCase(java.util.Locale.ROOT))) lastLive = true;
            parts[a] = out;
        }
        updateDucking(now, active);
        double fadeStep = (1 - duckLevel()) / Math.max(1, DUCK_FADE_MS * outRate / 1000.0);
        double master = muted ? 0 : volume / 100.0;
        short[] mix = new short[count];
        double[] sums = new double[count];
        for (int a = 0; a < active.size(); a++) {
            AudioStream s = active.get(a);
            double target = s.ducked ? duckLevel() : 1;
            double g = s.gain;
            double[] part = parts[a];
            for (int k = 0; k < count; k++) {
                if (g < target) g = Math.min(target, g + fadeStep);
                else if (g > target) g = Math.max(target, g - fadeStep);
                sums[k] += part[k] * g;
            }
            s.gain = g;
        }
        int pk = 0;
        for (int k = 0; k < count; k++) {
            long v = Math.round(sums[k] * master);
            mix[k] = (short) Math.max(Short.MIN_VALUE, Math.min(Short.MAX_VALUE, v));
            pk = Math.max(pk, Math.abs(mix[k]));
        }
        peak = Math.min(32767, pk);
        peakAtNanos = now;
        emitSamples += count;
        drained += count;
        notifyAll();
        return mix;
    }

    /** Did a live stream ({@link #isLiveRole}: a call) put sound into the last {@link #drain()}? */
    private boolean lastLive;
    /** When the first sample of the last {@link #drain()} plays (System.nanoTime): the mix's own clock. */
    private long lastDrainStartNanos;

    /**
     * Did a live stream (a role for which {@link #isLiveRole} is true: a call)
     * put sound into the last {@link #drain()}? Live audio is sent to play at
     * once, not on the picture's playout delay (SpeakerAudioPacket.FLAG_LIVE).
     */
    public synchronized boolean lastDrainLive() {
        return lastLive;
    }

    /** Roles whose sound is live: {@code phone} (cellular calls) and {@code voice} (Discord calls). */
    public static boolean isLiveRole(String role) {
        return "phone".equals(role) || "voice".equals(role);
    }

    /**
     * The pts of the last {@link #drain()}'s first sample, on the display clock
     * ({@code DisplayClock}, microseconds): when the mixer scheduled it to play,
     * after mixing and ducking. Senders stamp their blocks from it.
     */
    public synchronized long lastDrainPtsUs() {
        return com.example.evanscomputermod.computer.pipeline.DisplayClock.ofNanoTime(lastDrainStartNanos);
    }

    /** Which streams a louder, higher-role stream ducks now. Streams not mixed this tick fade with the rest. */
    private void updateDucking(long now, List<AudioStream> mixed) {
        for (AudioStream s : streams) {
            boolean d = false;
            for (AudioStream h : streams) {
                if (h != s && h.role.ordinal() > s.role.ordinal() && h.loudUntilNanos > now) {
                    d = true;
                    break;
                }
            }
            s.ducked = d;
            if (!mixed.contains(s)) s.gain = d ? duckLevel() : 1;
        }
    }

    // ------------------------------------------------------------ the card's own stream

    public int rate() {
        return defaultStream().rate();
    }

    public int bits() {
        return defaultStream().bits();
    }

    public int channels() {
        return defaultStream().channels();
    }

    public int latencyMs() {
        return defaultStream().latencyMs();
    }

    public long underruns() {
        return defaultStream().underruns();
    }

    /** Milliseconds queued on the card's own stream. */
    public int bufferedMs() {
        return defaultStream().bufferedMs();
    }

    public void setFormat(int newRate, int newBits, int newChannels) {
        defaultStream().setFormat(newRate, newBits, newChannels);
    }

    public void setLatencyMs(int ms) {
        defaultStream().setLatencyMs(ms);
    }

    /** Queue PCM on the card's own stream (see {@link AudioStream#write}). */
    public int write(byte[] data, int off, int len, boolean nonBlocking) throws InterruptedException, ClosedException {
        return defaultStream().write(data, off, len, nonBlocking);
    }

    /** {@code /dev/audioctl} contents of the card's own stream. */
    public String status() {
        return statusFor(0);
    }

    /** One {@code /dev/audioctl} command for the card's own stream. */
    public void control(String line) {
        controlFor(0, line);
    }
}
