package com.example.evanscomputermod.radio.wifi80211.crypto;

import java.util.Arrays;

/**
 * CCMP replay detection (802.11-2016 12.5.3.4.4): one counter per TID plus one
 * for non-QoS frames. A frame is accepted only if its PN is strictly greater
 * than the last accepted PN for that counter. Call {@link #accept} only after
 * the MIC has verified, so forged frames cannot advance the counter.
 */
public final class PnReplayWindow {

    /** Counter index used for non-QoS data frames and group frames. */
    public static final int NON_QOS = 16;

    private final long[] last = new long[17];

    public PnReplayWindow() {
        reset(0);
    }

    /** Resets every counter so the next accepted PN must exceed {@code initial} (e.g. a GTK's RSC). */
    public void reset(long initial) {
        Arrays.fill(last, initial);
    }

    public boolean isReplay(int index, long pn) {
        return pn <= last[index];
    }

    /** Returns true and records the PN if it is fresh; false if it is a replay. */
    public boolean accept(int index, long pn) {
        if (pn <= last[index]) return false;
        last[index] = pn;
        return true;
    }

    public long last(int index) {
        return last[index];
    }
}
