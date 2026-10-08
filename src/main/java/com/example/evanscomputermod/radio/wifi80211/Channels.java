package com.example.evanscomputermod.radio.wifi80211;

/** Channel numbering and rate sets for the 2.4 GHz and 5 GHz bands. */
public final class Channels {

    private Channels() {}

    /** 802.11b/g basic+supported rates (Supported Rates element, 500 kbit/s units, 0x80 = basic). */
    public static final byte[] RATES_24 = {(byte) 0x82, (byte) 0x84, (byte) 0x8b, (byte) 0x96, 0x0c, 0x12, 0x18, 0x24};
    /** 802.11g rates that overflow into the Extended Supported Rates element. */
    public static final byte[] EXT_RATES_24 = {0x30, 0x48, 0x60, 0x6c};
    /** 802.11a rates (5 GHz, OFDM only). */
    public static final byte[] RATES_5 = {(byte) 0x8c, 0x12, (byte) 0x98, 0x24, (byte) 0xb0, 0x48, 0x60, 0x6c};

    public static boolean isValid(int channel) {
        return (channel >= 1 && channel <= 14) || (channel >= 32 && channel <= 177);
    }

    public static boolean is5GHz(int channel) {
        return channel > 14;
    }

    /** Centre frequency in MHz. */
    public static int frequencyMhz(int channel) {
        if (channel == 14) return 2484;
        if (channel >= 1 && channel <= 13) return 2407 + 5 * channel;
        if (channel >= 32 && channel <= 177) return 5000 + 5 * channel;
        throw new IllegalArgumentException("unknown channel " + channel);
    }

    /** Channel number for a centre frequency, or 0 if it is not a known channel. */
    public static int channelFor(int mhz) {
        if (mhz == 2484) return 14;
        if (mhz >= 2412 && mhz <= 2472 && (mhz - 2407) % 5 == 0) return (mhz - 2407) / 5;
        if (mhz >= 5160 && mhz <= 5885 && mhz % 5 == 0) return (mhz - 5000) / 5;
        return 0;
    }

    public static byte[] supportedRates(int channel) {
        return is5GHz(channel) ? RATES_5.clone() : RATES_24.clone();
    }

    /** Extended rates for the channel; empty on 5 GHz. */
    public static byte[] extendedRates(int channel) {
        return is5GHz(channel) ? new byte[0] : EXT_RATES_24.clone();
    }
}
