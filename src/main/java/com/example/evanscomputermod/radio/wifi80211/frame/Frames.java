package com.example.evanscomputermod.radio.wifi80211.frame;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

/** Builders for complete MPDUs (no FCS). */
public final class Frames {

    private Frames() {}

    public static byte[] mgmt(int subtype, MacAddress a1, MacAddress a2, MacAddress a3, int seq, byte[] body) {
        return new Frame80211(MacHeader.mgmt(subtype, a1, a2, a3, seq), body).encode();
    }

    /** Non-QoS data frame with an LLC/SNAP body. {@code dsFlags} is ToDS and/or FromDS. */
    public static byte[] data(int dsFlags, MacAddress a1, MacAddress a2, MacAddress a3, int seq, int etherType, byte[] payload) {
        return new Frame80211(MacHeader.data(dsFlags, a1, a2, a3, seq), Llc.encap(etherType, payload)).encode();
    }

    public static byte[] deauth(MacAddress a1, MacAddress a2, MacAddress bssid, int seq, int reason) {
        return mgmt(FrameControl.DEAUTH, a1, a2, bssid, seq, Mgmt.reason(reason));
    }

    public static byte[] disassoc(MacAddress a1, MacAddress a2, MacAddress bssid, int seq, int reason) {
        return mgmt(FrameControl.DISASSOC, a1, a2, bssid, seq, Mgmt.reason(reason));
    }

    /** ACK control frame (10 bytes). */
    public static byte[] ack(MacAddress receiver) {
        return new MacHeader(FrameControl.of(FrameControl.TYPE_CTRL, FrameControl.ACK), 0, receiver, null, null, -1, null, -1, -1).encode();
    }
}
