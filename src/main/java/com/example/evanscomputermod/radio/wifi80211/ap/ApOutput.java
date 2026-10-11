package com.example.evanscomputermod.radio.wifi80211.ap;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

/**
 * Where an {@link AccessPointCore} sends what it produces. Called synchronously
 * from {@code onReceive}, {@code tick} and {@code onWiredFrame}; implementations
 * should queue rather than call back into the core.
 */
public interface ApOutput {

    /** An MPDU (no FCS) to put on the radio medium on the AP's channel. */
    void transmitRadio(byte[] frame80211);

    /**
     * An Ethernet frame (no FCS) to send into the cable network through the
     * bridge port; {@code srcMac} is the wireless client it came from.
     */
    void transmitWired(byte[] ethernetFrame, MacAddress srcMac);

    /** A client finished association (open) or the 4-way handshake (WPA2) and may now pass data. */
    default void clientAuthorized(MacAddress mac) {}

    /** A previously authorized client left (deauth, disassoc, timeout). */
    default void clientRemoved(MacAddress mac) {}
}
