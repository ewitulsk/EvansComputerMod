package com.example.evanscomputermod.radio.api;

import java.util.UUID;

/**
 * Anything that sends or hears radio: an access point, a Wi-Fi or controller
 * receiver module, an SDR, a handheld, a dish. The medium reads these through
 * this interface only; implementations live in the hardware packages.
 *
 * <p>Methods may be called from worker threads; implementations return
 * snapshots (immutable values) and must not touch the world.
 */
public interface RadioEndpoint {

    UUID id();

    /** Current world pose of the antenna (Sable endpoints report the projected pose). */
    Pose pose();

    /** The antenna's pattern in its local frame. */
    AntennaPattern antenna();

    /** Channel the receiver is tuned to (null when not listening). */
    Channel tunedChannel();

    /** Receiver noise figure, dB. */
    default double noiseFigureDb() {
        return 6;
    }

    /** Minimum received power to attempt decoding, dBm. */
    default double sensitivityDbm() {
        return -95;
    }

    /** Highest transmit power this endpoint can use, dBm (for range culling). */
    double maxTxPowerDbm();

    /** Speed of the endpoint in m/s (moving players, ships) — sets fading coherence time. */
    default double speedMps() {
        return 0;
    }

    /** True if this endpoint wants frames delivered (receiver on). */
    default boolean listening() {
        return tunedChannel() != null;
    }

    /**
     * How finely the pattern depends on orientation: the medium refreshes this antenna's gain
     * and polarization towards each peer once it has turned by the configured Sable turn
     * threshold times this. A narrow microwave beam uses a fraction; 1 for everything else.
     * (Endpoints never invalidate the medium for movement: it watches poses itself.)
     */
    default double turnThresholdScale() {
        return 1;
    }

    /** A frame decoded at this receiver. Called off-thread; queue it, don't touch the world. */
    void onReceive(Reception reception);
}
