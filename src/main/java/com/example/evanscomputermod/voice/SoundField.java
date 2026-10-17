package com.example.evanscomputermod.voice;

import java.util.Iterator;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.LongSupplier;

/**
 * Sound in the air, for microphones: every source that is making sound now
 * (players talking, speakers playing) as an {@link AirTrack} at its position.
 * A microphone hears the sum of the sources around it, each fainter with
 * distance (the inverse-distance law: half as loud at twice the distance,
 * full level within {@link #REFERENCE_DISTANCE}), and nothing past
 * {@link #range()}.
 *
 * <p>The clock is wall-clock time, the same clock a speaker plays on, because
 * people talk in real time. One field per server ({@link #get()}); tests make
 * their own with a fake clock.
 */
public final class SoundField {

    /** Within this distance (blocks) a source is heard at its own level. */
    public static final double REFERENCE_DISTANCE = 1.0;
    /** Sources silent for this long are forgotten. */
    private static final long EXPIRE_NANOS = 10_000_000_000L;

    private static final SoundField SERVER = new SoundField(System::nanoTime);

    private final LongSupplier clockNanos;
    private final Map<String, AirTrack> tracks = new ConcurrentHashMap<>();
    private volatile double range = 24;

    public SoundField(LongSupplier clockNanos) {
        this.clockNanos = clockNanos;
    }

    /** The server's sound field. */
    public static SoundField get() {
        return SERVER;
    }

    public long nanos() {
        return clockNanos.getAsLong();
    }

    /** Absolute sample index on this field's clock at {@code rate}. */
    public long sampleAt(int rate) {
        return Math.floorDiv(nanos(), 1000L) * rate / 1_000_000L;
    }

    /** Air sample index now. */
    public long nowAir() {
        return sampleAt(AirTrack.AIR_RATE);
    }

    /** Farthest a microphone hears anything, blocks. */
    public double range() {
        return range;
    }

    public void setRange(double blocks) {
        range = Math.max(1, blocks);
    }

    /** The track of source {@code id} (created on first use). */
    public AirTrack track(String id) {
        return tracks.computeIfAbsent(id, k -> new AirTrack());
    }

    /** The track of {@code id} if it exists. */
    public AirTrack find(String id) {
        return tracks.get(id);
    }

    public void remove(String id) {
        tracks.remove(id);
    }

    /** Sources that made sound recently. */
    public int size() {
        return tracks.size();
    }

    /**
     * Put a burst of sound from source {@code id} at a position into the air.
     * {@code gain} scales it (1 = as given).
     */
    public void emit(String id, String dimension, double x, double y, double z, short[] pcm, int off, int n, int rate, float gain) {
        AirTrack t = track(id);
        t.moveTo(dimension, x, y, z);
        long now = nanos();
        t.append(pcm, off, n, rate, Math.floorDiv(now, 1000L) * AirTrack.AIR_RATE / 1_000_000L, now, gain);
    }

    /**
     * Add what a microphone at a position hears into {@code out}: {@code n}
     * samples at {@code rate} starting at absolute sample {@code start}.
     */
    public void render(String dimension, double x, double y, double z, long start, int n, int rate, float[] out) {
        long now = nanos();
        double r = range;
        for (Iterator<Map.Entry<String, AirTrack>> it = tracks.entrySet().iterator(); it.hasNext(); ) {
            AirTrack t = it.next().getValue();
            if (now - t.lastWriteNanos() > EXPIRE_NANOS) {
                it.remove();
                continue;
            }
            float g = gainAt(t, dimension, x, y, z, r);
            if (g > 0) t.addTo(out, start, n, rate, g);
        }
    }

    /** How loud {@code t} is at a position, 0 when out of range or in another dimension. */
    static float gainAt(AirTrack t, String dimension, double x, double y, double z, double range) {
        if (!dimension.equals(t.dimension())) return 0;
        double dx = t.x() - x, dy = t.y() - y, dz = t.z() - z;
        double d = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (d > range) return 0;
        return (float) (REFERENCE_DISTANCE / Math.max(REFERENCE_DISTANCE, d));
    }
}
