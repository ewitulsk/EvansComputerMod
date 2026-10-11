package com.example.evanscomputermod.radio.phys;

import java.util.ArrayList;
import java.util.List;

/**
 * 802.11 rate table for rate adaptation. Rates are the standard PHY rates;
 * HT rates are computed from the OFDM numerology
 * R = N<sub>SD</sub> &middot; N<sub>BPSCS</sub> &middot; R<sub>code</sub> &middot; N<sub>SS</sub> / T<sub>sym</sub>
 * (N<sub>SD</sub> = 52 at 20 MHz, 108 at 40 MHz; T<sub>sym</sub> = 4 &micro;s, 3.6 &micro;s short GI),
 * so HT20 MCS0 is 6.5 Mbps, HT40 SGI MCS7 is 150 Mbps and two streams reach 300 Mbps.
 *
 * <p>{@code minSinrDb} is the SINR at which the rate reaches ~10% PER on a 1000-byte
 * frame: typical receiver thresholds consistent with the 802.11 minimum
 * sensitivities (e.g. 6 Mbps at &minus;82 dBm, 54 Mbps at &minus;65 dBm in 20 MHz)
 * less the noise floor. 802.11b DSSS/CCK gains from spreading so 1 Mbps works at
 * ~0 dB SINR. N spatial streams need 10 log10 N dB more (power split between streams).
 */
public record WifiMcs(Standard standard, int index, String name, double rateMbps, double minSinrDb,
        Modulation modulation, double codingRate, int bandwidthMHz, int streams, boolean shortGi) {

    public enum Standard { DSSS_11B, OFDM_11AG, HT_11N }

    /** Waterfall steepness of coded OFDM: PER falls ~4.5x per dB above threshold. */
    private static final double WATERFALL_PER_DB = 1.5;

    public static final List<WifiMcs> DSSS_11B = List.of(
            new WifiMcs(Standard.DSSS_11B, 0, "1M DBPSK", 1, 0, Modulation.DBPSK, 1, 22, 1, false),
            new WifiMcs(Standard.DSSS_11B, 1, "2M DQPSK", 2, 3, Modulation.DQPSK, 1, 22, 1, false),
            new WifiMcs(Standard.DSSS_11B, 2, "5.5M CCK", 5.5, 6, Modulation.DQPSK, 1, 22, 1, false),
            new WifiMcs(Standard.DSSS_11B, 3, "11M CCK", 11, 9, Modulation.DQPSK, 1, 22, 1, false));

    public static final List<WifiMcs> OFDM_11AG = List.of(
            ofdm(0, 6, 4, Modulation.BPSK, 0.5),
            ofdm(1, 9, 5, Modulation.BPSK, 0.75),
            ofdm(2, 12, 7, Modulation.QPSK, 0.5),
            ofdm(3, 18, 9, Modulation.QPSK, 0.75),
            ofdm(4, 24, 12, Modulation.QAM16, 0.5),
            ofdm(5, 36, 16, Modulation.QAM16, 0.75),
            ofdm(6, 48, 20, Modulation.QAM64, 2.0 / 3),
            ofdm(7, 54, 21, Modulation.QAM64, 0.75));

    private static final Modulation[] HT_MOD = {Modulation.BPSK, Modulation.QPSK, Modulation.QPSK, Modulation.QAM16,
            Modulation.QAM16, Modulation.QAM64, Modulation.QAM64, Modulation.QAM64};
    private static final double[] HT_RATE = {0.5, 0.5, 0.75, 0.5, 0.75, 2.0 / 3, 0.75, 5.0 / 6};
    private static final double[] HT_SINR = {4, 7, 9, 12, 16, 20, 21, 23};

    private static WifiMcs ofdm(int i, double mbps, double sinr, Modulation m, double r) {
        return new WifiMcs(Standard.OFDM_11AG, i, mbps + "M OFDM", mbps, sinr, m, r, 20, 1, false);
    }

    /** HT (802.11n) MCS 0..(8&middot;maxStreams&minus;1) for one channel width and guard interval. */
    public static List<WifiMcs> ht(int bandwidthMHz, int maxStreams, boolean shortGi) {
        if(bandwidthMHz != 20 && bandwidthMHz != 40) throw new IllegalArgumentException("HT is 20 or 40 MHz");
        int nsd = bandwidthMHz == 20 ? 52 : 108;
        double tsym = shortGi ? 3.6 : 4.0;
        List<WifiMcs> out = new ArrayList<>();
        for(int ss = 1; ss <= Math.max(1, Math.min(4, maxStreams)); ss++)
            for(int i = 0; i < 8; i++) {
                Modulation m = HT_MOD[i];
                double rate = nsd * m.bitsPerSymbol() * HT_RATE[i] * ss / tsym;
                rate = Math.round(rate * 10) / 10.0;
                int mcs = (ss - 1) * 8 + i;
                out.add(new WifiMcs(Standard.HT_11N, mcs, "HT" + bandwidthMHz + " MCS" + mcs + (shortGi ? " SGI" : ""),
                        rate, HT_SINR[i] + 10.0 * Math.log10(ss), m, HT_RATE[i], bandwidthMHz, ss, shortGi));
            }
        return List.copyOf(out);
    }

    /** Highest-rate entry whose threshold the SINR meets, or null if none (link down). */
    public static WifiMcs best(List<WifiMcs> table, double sinrDb) {
        WifiMcs best = null;
        for(WifiMcs m : table)
            if(sinrDb >= m.minSinrDb && (best == null || m.rateMbps > best.rateMbps)) best = m;
        return best;
    }

    /**
     * PER of a frame of {@code bytes} at {@code sinrDb}: a coded-OFDM waterfall
     * PER<sub>1000</sub> = 1 / (1 + 9 e<sup>1.5(SINR &minus; threshold)</sup>) (10% at threshold),
     * scaled to length as 1 &minus; (1 &minus; PER<sub>1000</sub>)<sup>bytes/1000</sup>.
     */
    public double packetErrorRate(double sinrDb, int bytes) {
        double per1000 = 1.0 / (1.0 + 9.0 * Math.exp(WATERFALL_PER_DB * (sinrDb - minSinrDb)));
        if(bytes <= 0) return 0;
        return -Math.expm1(bytes / 1000.0 * Math.log1p(-Math.min(per1000, 1 - 1e-15)));
    }

    /** Airtime of a payload of {@code bytes} at this rate, excluding preamble, microseconds. */
    public double payloadAirtimeUs(int bytes) {
        return bytes * 8.0 / rateMbps;
    }
}
