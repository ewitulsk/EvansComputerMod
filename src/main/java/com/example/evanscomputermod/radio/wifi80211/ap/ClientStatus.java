package com.example.evanscomputermod.radio.wifi80211.ap;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

/**
 * Snapshot of one station known to the AP, for the status page.
 *
 * @param aid           association ID, 0 if not associated
 * @param lastRssiDbm   RSSI of the last frame received from the station
 * @param lastRateKbps  PHY rate of the last frame received from the station
 * @param lastError     most recent protocol error for this station, or null
 */
public record ClientStatus(MacAddress mac, int aid, State state, Handshake handshake, int lastRssiDbm, int lastRateKbps,
                           String lastError, long associatedAtMs, long lastActivityMs, long rxFrames, long txFrames,
                           long replayDrops, long micFailures) {

    public enum State {
        /** Open System authentication done, not associated. */
        AUTHENTICATED,
        /** Associated; data is blocked until the 4-way handshake completes (WPA2). */
        ASSOCIATED,
        /** Associated and keys installed (or open network): data flows. */
        AUTHORIZED
    }

    public enum Handshake {
        NONE,
        /** M1 sent, waiting for a valid M2. */
        PTK_START,
        /** M3 sent, waiting for M4. */
        PTK_NEGOTIATING,
        /** PTK and GTK installed. */
        DONE,
        /** Group key message 1 sent, waiting for group message 2. */
        GTK_REKEYING
    }
}
