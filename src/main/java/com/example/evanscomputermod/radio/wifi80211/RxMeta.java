package com.example.evanscomputermod.radio.wifi80211;

/**
 * Receive metadata the radio medium attaches to a frame (radiotap-style).
 *
 * @param rssiDbm   received signal strength in dBm
 * @param rateKbps  PHY data rate the frame was sent at, in kbit/s
 * @param channel   channel number the frame was received on; 0 = unknown, no check
 */
public record RxMeta(int rssiDbm, int rateKbps, int channel) {

    public static final RxMeta NONE = new RxMeta(0, 0, 0);
}
