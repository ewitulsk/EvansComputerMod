package com.example.evanscomputermod.radio.wifi.ap;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;
import com.example.evanscomputermod.radio.phys.WifiMcs;
import com.example.evanscomputermod.radio.wifi80211.Channels;

/**
 * How Wi-Fi frames go on the air: which channel object, which PHY rate and how
 * long the frame occupies the channel. Pure (no Minecraft), shared by the
 * Access Point block and the virtual test station.
 *
 * <p>Management and group-addressed frames use the lowest basic rate (DSSS
 * 1 Mbit/s on 2.4 GHz, OFDM 6 Mbit/s on 5 GHz) so every station can hear them.
 * Unicast data picks the fastest 802.11a/g OFDM rate whose SINR threshold the
 * peer's last RSSI clears with a {@value #RATE_MARGIN_DB} dB margin, which keeps
 * the rates consistent with the Supported Rates the beacons advertise (the AP
 * has no HT capabilities element, so no HT rates).
 */
public final class ApRadioPlan {

    /** One PHY rate: the modulation name the medium uses for its PER curve, and kbit/s. */
    public record Rate(String modulation, int kbps) {
        public boolean ofdm() {
            return modulation.startsWith("OFDM");
        }

        public double mbps() {
            return kbps / 1000.0;
        }
    }

    public static final Rate DSSS_1 = new Rate("DSSS-1", 1000);
    public static final Rate OFDM_6 = new Rate("OFDM-6", 6000);

    /** Extra SINR asked for above a rate's threshold before using it. */
    public static final double RATE_MARGIN_DB = 3;
    /** Receiver noise figure assumed when turning RSSI into SINR for rate choice. */
    public static final double NOISE_FIGURE_DB = 6;
    public static final int SIFS_US = 10;
    public static final int SLOT_US = 9;
    /** FCS bytes added on the air (frames in this code carry none). */
    public static final int FCS_BYTES = 4;

    private ApRadioPlan() {}

    /** The medium channel of an 802.11 channel number (2.4 GHz: 22 MHz wide; 5 GHz: 20 MHz). */
    public static Channel channel(int wifiChannel) {
        if (!Channels.isValid(wifiChannel) || wifiChannel == 14) throw new IllegalArgumentException("channel " + wifiChannel);
        return Channels.is5GHz(wifiChannel) ? Channel.wifi5(wifiChannel, 20) : Channel.wifi24(wifiChannel);
    }

    /** Rate for beacons, probe responses, auth/assoc and group-addressed data. */
    public static Rate basicRate(int wifiChannel) {
        return Channels.is5GHz(wifiChannel) ? OFDM_6 : DSSS_1;
    }

    /** Rate for unicast data to a peer last heard at {@code rssiDbm} (NaN = unknown: basic rate). */
    public static Rate dataRate(int wifiChannel, double rssiDbm) {
        if (Double.isNaN(rssiDbm)) return basicRate(wifiChannel);
        double sinr = rssiDbm - BasicRadioMedium.noiseDbm(20e6, NOISE_FIGURE_DB) - RATE_MARGIN_DB;
        WifiMcs best = WifiMcs.best(WifiMcs.OFDM_11AG, sinr);
        if (best == null) return basicRate(wifiChannel);
        int mbps = (int) Math.round(best.rateMbps());
        return new Rate("OFDM-" + mbps, mbps * 1000);
    }

    /**
     * Airtime of an MPDU of {@code bytes} (without FCS) in microseconds: the
     * long DSSS preamble + PLCP header (192 us) then the PSDU at the bit rate;
     * OFDM is 20 us of preamble and SIGNAL then 4 us symbols carrying
     * SERVICE (16 bits) + PSDU + tail (6 bits), plus the 6 us ERP signal
     * extension on 2.4 GHz.
     */
    public static long airtimeUs(Rate rate, int bytes, int wifiChannel) {
        int psduBits = (bytes + FCS_BYTES) * 8;
        if (!rate.ofdm()) {
            return 192 + (long) Math.ceil(psduBits / rate.mbps());
        }
        int bitsPerSymbol = (int) Math.round(rate.mbps() * 4);
        long symbols = (16 + psduBits + 6 + bitsPerSymbol - 1) / bitsPerSymbol;
        return 20 + symbols * 4 + (Channels.is5GHz(wifiChannel) ? 0 : 6);
    }

    /** DIFS (SIFS + 2 slots) used as the gap between back-to-back transmissions from one radio. */
    public static int difsUs() {
        return SIFS_US + 2 * SLOT_US;
    }
}
