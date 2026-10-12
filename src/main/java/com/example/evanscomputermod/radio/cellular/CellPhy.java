package com.example.evanscomputermod.radio.cellular;

import java.util.Locale;

/**
 * Cellular link adaptation and airtime, pure. Channel quality indicators (CQI)
 * follow the 3GPP 4-bit tables: CQI 1-15 are LTE's QPSK/16QAM/64QAM entries
 * (TS 36.213 Table 7.2.3-1) with the SINR each needs for 10 % block error on
 * an AWGN channel; CQI 16-18 extend to NR's 256QAM entries (TS 38.214 Table
 * 5.2.2.1-3) for mmWave. A frame's rate is spectral efficiency x bandwidth x
 * (1 - overhead); on the medium it is an {@code Emission.frame} with
 * modulation {@code "CELL-CQI<n>"}, whose packet error rate comes from the
 * same threshold (see {@code BasicRadioMedium.requiredSinrDb}).
 */
public final class CellPhy {
    private CellPhy() {}

    /** Spectral efficiency (bits/s/Hz) per CQI 1..18 (index 0 unused). */
    static final double[] EFFICIENCY = {0,
            0.1523, 0.2344, 0.3770, 0.6016, 0.8770, 1.1758,          // QPSK
            1.4766, 1.9141, 2.4063,                                  // 16QAM
            2.7305, 3.3223, 3.9023, 4.5234, 5.1152, 5.5547,          // 64QAM
            6.2266, 6.9141, 7.4063};                                 // 256QAM (NR)
    /** SINR for 10 % BLER per CQI, dB. */
    static final double[] REQUIRED_SINR_DB = {0,
            -6.7, -4.7, -2.3, 0.2, 2.4, 4.3,
            5.9, 8.1, 10.3,
            11.7, 14.1, 16.3, 18.7, 21.0, 22.7,
            24.5, 26.5, 28.5};
    static final String[] MODULATION = {"", "QPSK", "QPSK", "QPSK", "QPSK", "QPSK", "QPSK",
            "16QAM", "16QAM", "16QAM", "64QAM", "64QAM", "64QAM", "64QAM", "64QAM", "64QAM", "256QAM", "256QAM", "256QAM"};
    public static final int MAX_CQI = 18;
    /** Link adaptation keeps this much SINR in hand over the chosen CQI's threshold, dB. */
    public static final double MARGIN_DB = 3;
    /** The robust CQI beacons, attach messages and stale-feedback frames use. */
    public static final int ROBUST_CQI = 1;
    public static final String MODULATION_PREFIX = "CELL-CQI";
    private static final double BOLTZMANN_DBM_PER_HZ = -174;

    public static double efficiency(int cqi) {
        return EFFICIENCY[clampCqi(cqi)];
    }

    public static double requiredSinrDb(int cqi) {
        return REQUIRED_SINR_DB[clampCqi(cqi)];
    }

    /** "QPSK", "16QAM", "64QAM" or "256QAM". */
    public static String constellation(int cqi) {
        return MODULATION[clampCqi(cqi)];
    }

    /** The medium modulation name for a CQI ("CELL-CQI7"). */
    public static String modulation(int cqi) {
        return MODULATION_PREFIX + clampCqi(cqi);
    }

    /** SINR threshold for a medium modulation name ("CELL-CQI7"), or NaN if it isn't one of ours. */
    public static double requiredSinrDb(String modulation) {
        if (modulation == null || !modulation.toUpperCase(Locale.ROOT).startsWith(MODULATION_PREFIX)) return Double.NaN;
        try {
            return requiredSinrDb(Integer.parseInt(modulation.substring(MODULATION_PREFIX.length()).trim()));
        } catch (NumberFormatException e) {
            return Double.NaN;
        }
    }

    /** CQI index of a medium modulation name, or 0. */
    public static int cqiOf(String modulation) {
        if (modulation == null || !modulation.toUpperCase(Locale.ROOT).startsWith(MODULATION_PREFIX)) return 0;
        try {
            return clampCqi(Integer.parseInt(modulation.substring(MODULATION_PREFIX.length()).trim()));
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    /**
     * The fastest CQI (at most {@code tech.maxCqi}) whose threshold plus {@link #MARGIN_DB}
     * the SINR clears; {@link #ROBUST_CQI} when nothing does or the SINR is unknown (NaN).
     */
    public static int cqiFor(CellTech tech, double sinrDb) {
        if (Double.isNaN(sinrDb)) return ROBUST_CQI;
        int best = ROBUST_CQI;
        for (int c = 1; c <= tech.maxCqi; c++) if (sinrDb >= REQUIRED_SINR_DB[c] + MARGIN_DB) best = c;
        return best;
    }

    /** Net data rate at a CQI over {@code bandwidthHz}, bit/s. */
    public static double rateBps(CellTech tech, int cqi, double bandwidthHz) {
        return efficiency(cqi) * bandwidthHz * (1 - tech.overhead);
    }

    /** Downlink rate at a CQI over a whole channel, bit/s. */
    public static double downlinkRateBps(CellTech tech, int cqi) {
        return rateBps(tech, cqi, tech.bandwidthHz);
    }

    /** Uplink rate at a CQI on one device's slice, bit/s. */
    public static double uplinkRateBps(CellTech tech, int cqi) {
        return rateBps(tech, cqi, tech.sliceBandwidthHz());
    }

    /** Time on the air for {@code bytes} at {@code rateBps}, µs (one symbol of overhead, at least 1 µs). */
    public static long airtimeUs(CellTech tech, int bytes, double rateBps) {
        return tech.symbolUs + Math.max(1, (long) Math.ceil(bytes * 8.0 / rateBps * 1e6));
    }

    /** Thermal noise floor of a receiver, dBm. */
    public static double noiseDbm(double bandwidthHz, double noiseFigureDb) {
        return BOLTZMANN_DBM_PER_HZ + 10 * Math.log10(bandwidthHz) + noiseFigureDb;
    }

    /** Weakest signal a device tries to decode: noise floor + the robust CQI's threshold, dBm. */
    public static double ueSensitivityDbm(CellTech tech) {
        return noiseDbm(tech.bandwidthHz, tech.ueNoiseFigureDb) + REQUIRED_SINR_DB[ROBUST_CQI] - 1;
    }

    /** Weakest uplink a tower tries to decode on one slice, dBm. */
    public static double towerSensitivityDbm(CellTech tech) {
        return noiseDbm(tech.sliceBandwidthHz(), tech.towerNoiseFigureDb) + REQUIRED_SINR_DB[ROBUST_CQI] - 1;
    }

    /**
     * Open-loop uplink estimate from a downlink beacon, by reciprocity: the path gain
     * (antenna gains included) is the beacon's RSSI minus the tower's transmit power, so the
     * tower hears {@code ueTx + gain} over its slice's noise floor. Returns the uplink SINR, dB.
     */
    public static double uplinkSinrEstimateDb(CellTech tech, double beaconRssiDbm, double towerTxDbm) {
        double gain = beaconRssiDbm - towerTxDbm;
        return tech.ueTxDbm + gain - noiseDbm(tech.sliceBandwidthHz(), tech.towerNoiseFigureDb);
    }

    /** Signal bars (0-4) for a downlink SINR, as the status bar shows them. */
    public static int bars(double sinrDb) {
        if (Double.isNaN(sinrDb) || sinrDb < -6) return 0;
        if (sinrDb < 0) return 1;
        if (sinrDb < 8) return 2;
        if (sinrDb < 16) return 3;
        return 4;
    }

    private static int clampCqi(int cqi) {
        return Math.max(1, Math.min(MAX_CQI, cqi));
    }
}
