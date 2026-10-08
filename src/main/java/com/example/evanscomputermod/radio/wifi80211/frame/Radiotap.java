package com.example.evanscomputermod.radio.wifi80211.frame;

import com.example.evanscomputermod.radio.wifi80211.Channels;

import java.util.Arrays;

/**
 * Radiotap capture header (radiotap.org), as written in front of each frame in
 * a LINKTYPE_IEEE802_11_RADIO (127) pcap. The writer emits Flags, Rate,
 * Channel and dBm Antenna Signal; the reader understands the fields up to
 * dBm Antenna Signal and uses {@code it_len} to find the frame regardless.
 */
public final class Radiotap {

    public static final int PRESENT_TSFT = 1;
    public static final int PRESENT_FLAGS = 1 << 1;
    public static final int PRESENT_RATE = 1 << 2;
    public static final int PRESENT_CHANNEL = 1 << 3;
    public static final int PRESENT_FHSS = 1 << 4;
    public static final int PRESENT_DBM_SIGNAL = 1 << 5;

    public static final int FLAG_FCS = 0x10;

    public static final int CHAN_CCK = 0x0020;
    public static final int CHAN_OFDM = 0x0040;
    public static final int CHAN_2GHZ = 0x0080;
    public static final int CHAN_5GHZ = 0x0100;

    private static final int LEN = 15;

    private Radiotap() {}

    /** Parsed radiotap fields; absent values are -1 (signal: Integer.MIN_VALUE). */
    public record Info(int headerLength, int flags, int rateKbps, int frequencyMhz, int channelFlags, int signalDbm) {
        public boolean hasFcs() {
            return flags >= 0 && (flags & FLAG_FCS) != 0;
        }
    }

    /**
     * Prefixes {@code frame} with a radiotap header.
     *
     * @param hasFcs true if {@code frame} ends with its FCS
     */
    public static byte[] wrap(byte[] frame, int rateKbps, int channel, int signalDbm, boolean hasFcs) {
        byte[] b = new byte[LEN + frame.length];
        b[0] = 0; // version
        b[1] = 0; // pad
        b[2] = LEN;
        b[3] = 0;
        int present = PRESENT_FLAGS | PRESENT_RATE | PRESENT_CHANNEL | PRESENT_DBM_SIGNAL;
        for (int i = 0; i < 4; i++) b[4 + i] = (byte) (present >>> (8 * i));
        b[8] = (byte) (hasFcs ? FLAG_FCS : 0);
        b[9] = (byte) (rateKbps / 500);
        int freq = Channels.frequencyMhz(channel);
        MacHeader.le16(b, 10, freq);
        MacHeader.le16(b, 12, channelFlags(channel, rateKbps));
        b[14] = (byte) signalDbm;
        System.arraycopy(frame, 0, b, LEN, frame.length);
        return b;
    }

    static int channelFlags(int channel, int rateKbps) {
        if (Channels.is5GHz(channel)) return CHAN_5GHZ | CHAN_OFDM;
        boolean cck = rateKbps == 1000 || rateKbps == 2000 || rateKbps == 5500 || rateKbps == 11000;
        return CHAN_2GHZ | (cck ? CHAN_CCK : CHAN_OFDM);
    }

    public static Info parse(byte[] b) {
        if (b.length < 8 || b[0] != 0) throw new IllegalArgumentException("not a radiotap v0 header");
        int len = MacHeader.u16(b, 2);
        if (len < 8 || len > b.length) throw new IllegalArgumentException("bad radiotap length");
        long present = MacHeader.u32(b, 4);
        int p = 8;
        long word = present;
        while ((word & 0x8000_0000L) != 0) { // extended bitmaps
            if (p + 4 > len) throw new IllegalArgumentException("truncated radiotap present words");
            word = MacHeader.u32(b, p);
            p += 4;
        }
        int flags = -1, rate = -1, freq = -1, chFlags = -1, signal = Integer.MIN_VALUE;
        if ((present & PRESENT_TSFT) != 0) {
            p = (p + 7) & ~7;
            p += 8;
        }
        if ((present & PRESENT_FLAGS) != 0 && p < len) flags = b[p++] & 0xFF;
        if ((present & PRESENT_RATE) != 0 && p < len) rate = (b[p++] & 0xFF) * 500;
        if ((present & PRESENT_CHANNEL) != 0) {
            p = (p + 1) & ~1;
            if (p + 4 <= len) {
                freq = MacHeader.u16(b, p);
                chFlags = MacHeader.u16(b, p + 2);
            }
            p += 4;
        }
        if ((present & PRESENT_FHSS) != 0) p += 2;
        if ((present & PRESENT_DBM_SIGNAL) != 0 && p < len) signal = b[p];
        return new Info(len, flags, rate, freq, chFlags, signal);
    }

    /** The 802.11 frame after the radiotap header (FCS kept if present). */
    public static byte[] payload(byte[] b) {
        return Arrays.copyOfRange(b, MacHeader.u16(b, 2), b.length);
    }
}
