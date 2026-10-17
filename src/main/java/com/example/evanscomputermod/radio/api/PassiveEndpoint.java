package com.example.evanscomputermod.radio.api;

/**
 * An energise-to-answer endpoint (an RFID/NFC tag, a passive card): it has no transmitter
 * of its own and only backscatters the field that illuminates it.
 *
 * <p>The medium delivers {@link Emission.Kind#ENERGY} emissions on the tuned channel to
 * passive endpoints: a reception at or above {@link #wakeThresholdDbm()} calls
 * {@link #energise} with the field power and the time the field holds for. A simple
 * implementation stores {@code fieldPowerDbm} and {@code energisedUntil} and answers
 * {@link #energised()} from the medium clock ({@link RadioMedium#nowMicros()}).
 *
 * <p>{@link #maxTxPowerDbm()} contract: the field power currently energising the endpoint
 * minus {@link #backscatterLossDb()} while {@link #energised()}, else {@code -Infinity}. The
 * medium enforces it too: a transmit from a cold passive endpoint is dropped (and counted),
 * and an energised one is clamped to that power.
 */
public interface PassiveEndpoint extends RadioEndpoint {

    /** Least field power that wakes the endpoint, dBm. */
    double wakeThresholdDbm();

    /** How far below the illuminating field the reply is, dB. */
    default double backscatterLossDb() {
        return 20;
    }

    /** True while a field at or above the wake threshold holds the endpoint powered. */
    boolean energised();

    /**
     * An energising field arrived (medium hot path, any thread): record its power and
     * stay energised until {@code untilMicros} on the medium clock.
     */
    void energise(double fieldPowerDbm, long untilMicros);
}
