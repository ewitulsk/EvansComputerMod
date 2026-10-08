package com.example.evanscomputermod.radio.wifi.mac;

import java.util.random.RandomGenerator;

/**
 * DCF binary exponential backoff: a contention window that starts at
 * {@link WifiPhy#CW_MIN}, doubles (2·CW + 1) after every failed attempt up to
 * {@link WifiPhy#CW_MAX}, and resets after a success or a dropped frame. Each
 * attempt waits a uniform number of slots in [0, CW]. Pure and deterministic
 * given its random generator.
 */
public final class Backoff {

    private final RandomGenerator rng;
    private int cw = WifiPhy.CW_MIN;
    private int attempts;

    public Backoff(RandomGenerator rng) {
        this.rng = rng;
    }

    public int cw() {
        return cw;
    }

    /** Attempts made for the current frame. */
    public int attempts() {
        return attempts;
    }

    /** Slots to wait before the next attempt, in [0, cw]. */
    public int drawSlots() {
        attempts++;
        return rng.nextInt(cw + 1);
    }

    /** No ACK: widen the window. Returns false once the retry limit is used up. */
    public boolean onFailure() {
        cw = Math.min(WifiPhy.CW_MAX, cw * 2 + 1);
        return attempts <= WifiPhy.RETRY_LIMIT;
    }

    /** ACKed, or given up: back to CWmin for the next frame. */
    public void reset() {
        cw = WifiPhy.CW_MIN;
        attempts = 0;
    }
}
