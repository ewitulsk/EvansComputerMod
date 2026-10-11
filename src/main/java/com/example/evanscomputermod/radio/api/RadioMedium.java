package com.example.evanscomputermod.radio.api;

/**
 * The shared radio medium beside {@code NetworkHub}'s cables. One per server.
 *
 * <p>Contract (spec, Radio medium engine): delivering a frame never raycasts —
 * it reads cached path loss, adds fading and interference and rolls once against
 * the packet error rate. Path loss is recomputed off the hot path when an
 * endpoint moves, its antenna changes or the world on the path changes.
 * {@link #transmit} is callable from any thread.
 */
public interface RadioMedium {

    /** World airtime clock in microseconds (game time, continuous across lag). */
    long nowMicros();

    void register(RadioEndpoint endpoint);

    void unregister(RadioEndpoint endpoint);

    /** The endpoint's pose, antenna or tuning changed: its cached links are stale. */
    void invalidate(RadioEndpoint endpoint);

    /**
     * Put an emission on the air from {@code from}. The emission's start time is
     * clamped to now; returns the emission as scheduled.
     */
    Emission transmit(RadioEndpoint from, Emission emission);

    /** Total in-channel power (dBm) heard at {@code at} right now — carrier sense / RSSI. */
    double channelPowerDbm(RadioEndpoint at, Channel channel);

    /** Cached path gain (dB, negative = loss) between two endpoints at a frequency, or NaN if unknown yet. */
    double pathGainDb(RadioEndpoint a, RadioEndpoint b, double freqHz);

    /** One emission as heard at a receiver: power at the receiver's antenna port and propagation delay. */
    record Heard(RadioEndpoint from, Emission emission, double rxPowerDbm, double delayMicros) {}

    /**
     * Every emission (any kind) overlapping {@code [fromMicros, toMicros)} and the
     * channel {@code within}, as heard at {@code rx} — what an SDR synthesises its
     * IQ from. Excludes the receiver's own emissions. Called from worker threads.
     */
    default void forEachHeard(RadioEndpoint rx, Channel within, long fromMicros, long toMicros,
                              java.util.function.Consumer<Heard> sink) {
    }
}
