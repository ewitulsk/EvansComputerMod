package com.example.evanscomputermod.radio.cellular;

import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;

/**
 * The two cellular PHY profiles on the shared radio medium (docs/gui/CELLULAR.md).
 * Both are FDD: a tower transmits on a downlink channel and hears its devices on
 * the paired uplink channel, which is split into {@link #ulSlices} equal
 * sub-channels (OFDMA resource-block groups), one per attached device, so devices
 * of one tower never collide on the uplink. Slice 0 doubles as the random-access
 * channel used for attach requests.
 *
 * <ul>
 *   <li><b>LTE</b>: 10 MHz channels at 791-841 MHz downlink / 703-753 MHz uplink,
 *       inside {@link Band#UHF}. 20 W towers, 200 mW devices. Long range; goes
 *       through walls and thin terrain and diffracts over hills (λ ≈ 37 cm).</li>
 *   <li><b>mmWave</b> (5G NR FR2): 100 MHz channels at 28.05-28.35 GHz downlink /
 *       27.55-27.85 GHz uplink, inside {@link Band#MICROWAVE}. 1 W towers behind a
 *       20 dBi beam-steered array, 200 mW devices with an 8 dBi array. Very fast;
 *       λ ≈ 1 cm, so any block of earth, stone or water on the path stops it and
 *       diffraction round a hill is hopeless: line of sight only.</li>
 * </ul>
 *
 * Nothing in the 1-2.4 GHz or 2.5-5.1 GHz gaps is used. Pure (no Minecraft).
 */
public enum CellTech {
    LTE("lte", "LTE", 6, 791e6, 703e6, 10e6, 10e6, 4,
            43, 23, 3, 7, 14, 2.15, 0.25, 71, 15, 100_000, 0.04),
    MMWAVE("mmwave", "mmWave", 4, 28.05e9, 27.55e9, 100e6, 100e6, 4,
            30, 23, 5, 9, 20, 8, 0.18, 9, 18, 100_000, 0);

    /** Registry-style lower-case name ("lte", "mmwave"). */
    public final String id;
    public final String displayName;
    /** Channels in the plan, numbered 1..channels. */
    public final int channels;
    public final double dlBaseHz, ulBaseHz, spacingHz, bandwidthHz;
    /** Uplink sub-channels per tower (one per attached device, slice 0 = random access too). */
    public final int ulSlices;
    public final double towerTxDbm, ueTxDbm, towerNoiseFigureDb, ueNoiseFigureDb;
    /** Peak gain of a tower panel (LTE sector panel, or the mmWave array's steered beam), dBi. */
    public final double towerPanelGainDbi;
    /** Device antenna gain, dBi (LTE: a half-wave whip; mmWave: a steered 4-element array). */
    public final double ueAntennaGainDbi;
    /** Fraction of raw capacity lost to reference signals, control channels and cyclic prefix. */
    public final double overhead;
    /** Per-transmission overhead on the air, µs (one OFDM symbol). */
    public final long symbolUs;
    /** Highest CQI index the modulation table goes to (15 = 64QAM for LTE, 18 = 256QAM for NR). */
    public final int maxCqi;
    /** Synchronisation/system-information beacon period, µs. */
    public final long beaconIntervalUs;
    /** Feeder loss per metre of mast between base station and antenna, dB/m (mmWave radios sit at the antenna). */
    public final double feederDbPerMetre;

    CellTech(String id, String displayName, int channels, double dlBaseHz, double ulBaseHz, double spacingHz, double bandwidthHz,
             int ulSlices, double towerTxDbm, double ueTxDbm, double towerNoiseFigureDb, double ueNoiseFigureDb,
             double towerPanelGainDbi, double ueAntennaGainDbi, double overhead, long symbolUs, int maxCqi,
             long beaconIntervalUs, double feederDbPerMetre) {
        this.id = id;
        this.displayName = displayName;
        this.channels = channels;
        this.dlBaseHz = dlBaseHz;
        this.ulBaseHz = ulBaseHz;
        this.spacingHz = spacingHz;
        this.bandwidthHz = bandwidthHz;
        this.ulSlices = ulSlices;
        this.towerTxDbm = towerTxDbm;
        this.ueTxDbm = ueTxDbm;
        this.towerNoiseFigureDb = towerNoiseFigureDb;
        this.ueNoiseFigureDb = ueNoiseFigureDb;
        this.towerPanelGainDbi = towerPanelGainDbi;
        this.ueAntennaGainDbi = ueAntennaGainDbi;
        this.overhead = overhead;
        this.symbolUs = symbolUs;
        this.maxCqi = maxCqi;
        this.beaconIntervalUs = beaconIntervalUs;
        this.feederDbPerMetre = feederDbPerMetre;
    }

    public boolean validChannel(int n) {
        return n >= 1 && n <= channels;
    }

    /** Downlink channel {@code n} (1-based): what towers transmit and devices listen to. */
    public Channel downlink(int n) {
        check(n);
        return new Channel(dlBaseHz + spacingHz * (n - 1), bandwidthHz);
    }

    /** The whole uplink channel paired with downlink {@code n}. */
    public Channel uplink(int n) {
        check(n);
        return new Channel(ulBaseHz + spacingHz * (n - 1), bandwidthHz);
    }

    /** Uplink sub-channel {@code slice} (0..ulSlices-1) of channel {@code n}. */
    public Channel uplinkSlice(int n, int slice) {
        check(n);
        if (slice < 0 || slice >= ulSlices) throw new IllegalArgumentException(displayName + " uplink slice " + slice);
        double w = bandwidthHz / ulSlices;
        double low = ulBaseHz + spacingHz * (n - 1) - bandwidthHz / 2;
        return new Channel(low + w * (slice + 0.5), w);
    }

    public double sliceBandwidthHz() {
        return bandwidthHz / ulSlices;
    }

    /** The radio band the profile lives in (UHF for LTE, MICROWAVE for mmWave). */
    public Band band() {
        return Band.of(dlBaseHz);
    }

    /** The plan channel whose downlink is centred nearest {@code hz}, or -1. */
    public int channelOfDownlink(double hz) {
        int n = (int) Math.round((hz - dlBaseHz) / spacingHz) + 1;
        return validChannel(n) && Math.abs(downlink(n).centerHz() - hz) < spacingHz / 4 ? n : -1;
    }

    public static CellTech byId(String id) {
        for (CellTech t : values()) if (t.id.equalsIgnoreCase(id) || t.displayName.equalsIgnoreCase(id)) return t;
        return null;
    }

    private void check(int n) {
        if (!validChannel(n)) throw new IllegalArgumentException(displayName + " channel " + n + " (1-" + channels + ")");
    }
}
