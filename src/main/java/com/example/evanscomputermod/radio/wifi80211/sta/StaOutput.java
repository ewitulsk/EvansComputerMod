package com.example.evanscomputermod.radio.wifi80211.sta;

/** Where a {@link StationCore} sends what it produces. Called synchronously; implementations should queue. */
public interface StaOutput {

    /** An MPDU (no FCS) to put on the radio medium. */
    void transmitRadio(byte[] frame80211);

    /** A received, decrypted data frame converted to Ethernet (no FCS) for the host's network stack. */
    void deliverEthernet(byte[] ethernetFrame);
}
