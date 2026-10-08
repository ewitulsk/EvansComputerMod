package com.example.evanscomputermod.radio.wifi.mac;

import com.example.evanscomputermod.radio.api.Channel;

/**
 * 802.11 b/g/n PHY numbers the low MAC needs: the modulation name the medium
 * uses for packet error rates, airtime from rate and length, and the
 * interframe spaces. Rates are kb/s as on the host ABI ({@code wifi_tx_frame}):
 * DSSS/CCK 1000, 2000, 5500, 11000; OFDM 6000..54000; HT MCS0-7 6500..65000
 * (20 MHz, long GI, one stream). Pure Java.
 */
public final class WifiPhy {

    private WifiPhy() {}

    public static final int[] DSSS_KBPS = {1000, 2000, 5500, 11000};
    public static final int[] OFDM_KBPS = {6000, 9000, 12000, 18000, 24000, 36000, 48000, 54000};
    public static final int[] HT_KBPS = {6500, 13000, 19500, 26000, 39000, 52000, 58500, 65000};

    /** Short slot (ERP / OFDM), µs. */
    public static final int SLOT_US = 9;
    public static final int CW_MIN = 15;
    public static final int CW_MAX = 1023;
    /** 802.11 long retry limit: 1 + 7 retransmissions. */
    public static final int RETRY_LIMIT = 7;
    /** ACK / CTS length on the air with FCS. */
    public static final int ACK_LEN = 14;

    public static boolean isDsss(int kbps) {
        for (int r : DSSS_KBPS) if (r == kbps) return true;
        return false;
    }

    public static int htMcs(int kbps) {
        for (int i = 0; i < HT_KBPS.length; i++) if (HT_KBPS[i] == kbps) return i;
        return -1;
    }

    public static boolean isOfdm(int kbps) {
        for (int r : OFDM_KBPS) if (r == kbps) return true;
        return false;
    }

    /** True if {@code kbps} is one of the rates above. */
    public static boolean valid(int kbps) {
        return isDsss(kbps) || isOfdm(kbps) || htMcs(kbps) >= 0;
    }

    /** Modulation name for {@code BasicRadioMedium.requiredSinrDb}. */
    public static String modulation(int kbps) {
        return switch (kbps) {
            case 1000 -> "DSSS-1";
            case 2000 -> "DSSS-2";
            case 5500 -> "CCK-5.5";
            case 11000 -> "CCK-11";
            default -> {
                int mcs = htMcs(kbps);
                if (mcs >= 0) yield "HT-MCS" + mcs;
                if (isOfdm(kbps)) yield "OFDM-" + (kbps / 1000);
                yield "DSSS-1";
            }
        };
    }

    /** SIFS: 10 µs on 2.4 GHz, 16 µs on 5 GHz. */
    public static int sifsUs(Channel ch) {
        return ch != null && ch.centerHz() > 4e9 ? 16 : 10;
    }

    /** DIFS = SIFS + 2 slots. */
    public static int difsUs(Channel ch) {
        return sifsUs(ch) + 2 * SLOT_US;
    }

    /**
     * Airtime of a PPDU carrying {@code bytes} (MPDU incl. FCS) at {@code kbps}, µs:
     * DSSS long preamble 192 µs + payload; OFDM 20 µs preamble + 4 µs symbols of
     * (16 service + 8·len + 6 tail) bits (+6 µs signal extension on 2.4 GHz);
     * HT mixed format 36 µs preamble + symbols.
     */
    public static long airtimeUs(int kbps, int bytes, Channel ch) {
        if (!valid(kbps)) kbps = 1000;
        if (isDsss(kbps)) {
            return 192 + (long) Math.ceil(bytes * 8 * 1000.0 / kbps);
        }
        int ndbps = kbps * 4 / 1000;              // data bits per 4 µs symbol
        long symbols = (16 + 8L * bytes + 6 + ndbps - 1) / ndbps;
        boolean g24 = ch == null || ch.centerHz() < 4e9;
        long ext = g24 ? 6 : 0;
        return (htMcs(kbps) >= 0 ? 36 : 20) + 4 * symbols + ext;
    }

    /** The rate an ACK to a frame sent at {@code dataKbps} goes out at (a basic rate). */
    public static int ackRateKbps(int dataKbps, Channel ch) {
        boolean g5 = ch != null && ch.centerHz() > 4e9;
        if (g5) return 6000;
        return isDsss(dataKbps) ? 1000 : 6000;
    }

    /** The radio channel for an 802.11 channel number, or null if invalid. */
    public static Channel channel(int number) {
        if (number >= 1 && number <= 13) return Channel.wifi24(number);
        if (number >= 32 && number <= 177) return Channel.wifi5(number, 20);
        return null;
    }
}
