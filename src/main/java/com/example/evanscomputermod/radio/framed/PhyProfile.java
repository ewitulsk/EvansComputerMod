package com.example.evanscomputermod.radio.framed;

import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;

/**
 * The PHY profiles of the framed radio ({@link FramedRadio}: the SDR's framed PHY
 * mode and the modem modules' built-in radio; docs/radio/SDR_PROGRAMS.md "Framed PHY
 * mode"), moved here from the v1 cellular engine, which still uses them
 * (docs/gui/CELLULAR.md). Both are FDD: a tower transmits on a downlink channel and hears its devices on
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
public enum PhyProfile {
    LTE("lte", "LTE", 6, 791e6, 703e6, 10e6, 10e6, 4,
            43, 23, 3, 7, 14, 2.15, 0.25, 71, 15, 100_000, 0.04,
            1000, new double[] {1.4e6, 3e6, 5e6, 10e6}),
    MMWAVE("mmwave", "mmWave", 4, 28.05e9, 27.55e9, 100e6, 100e6, 4,
            30, 23, 5, 9, 20, 8, 0.18, 9, 18, 100_000, 0,
            125, new double[] {50e6, 100e6});

    /** HARQ: a receiver acknowledges a frame this many slots after the slot it started in (LTE n+4). */
    public static final int HARQ_ACK_SLOTS = 4;
    /** HARQ: a sender with no acknowledgement this many slots after a transmission's start retransmits (LTE n+8). */
    public static final int HARQ_RETX_SLOTS = 8;
    /** HARQ: retransmissions after the first attempt before the frame is reported failed. */
    public static final int HARQ_MAX_RETX = 3;

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
    /** Framed mode: slot length on the airtime clock, µs (LTE subframe 1 ms; NR numerology 3 slot 125 µs). */
    public final long slotUs;
    /** Framed mode: carrier bandwidths a radio may use inside one channel, Hz (the last is the full channel). */
    private final double[] bandwidths;

    PhyProfile(String id, String displayName, int channels, double dlBaseHz, double ulBaseHz, double spacingHz, double bandwidthHz,
             int ulSlices, double towerTxDbm, double ueTxDbm, double towerNoiseFigureDb, double ueNoiseFigureDb,
             double towerPanelGainDbi, double ueAntennaGainDbi, double overhead, long symbolUs, int maxCqi,
             long beaconIntervalUs, double feederDbPerMetre, long slotUs, double[] bandwidths) {
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
        this.slotUs = slotUs;
        this.bandwidths = bandwidths;
    }

    /** Carrier bandwidths a framed radio may use, Hz, narrowest first. */
    public double[] bandwidths() {
        return bandwidths.clone();
    }

    /** True if {@code hz} is one of {@link #bandwidths()} (within 1 kHz). */
    public boolean validBandwidth(double hz) {
        for (double b : bandwidths) if (Math.abs(b - hz) < 1e3) return true;
        return false;
    }

    /** Downlink channel {@code n} narrowed to {@code bandwidthHz} (same centre). */
    public Channel downlink(int n, double bandwidthHz) {
        return new Channel(downlink(n).centerHz(), bandwidthHz);
    }

    /** Uplink channel {@code n} narrowed to {@code bandwidthHz} (same centre). */
    public Channel uplink(int n, double bandwidthHz) {
        return new Channel(uplink(n).centerHz(), bandwidthHz);
    }

    /** Sub-channel {@code slice} (0..ulSlices-1) of {@code whole}: one of {@link #ulSlices} equal parts. */
    public Channel slice(Channel whole, int slice) {
        if (slice < 0 || slice >= ulSlices) throw new IllegalArgumentException(displayName + " sub-channel " + (slice + 1) + " (1-" + ulSlices + ")");
        double w = whole.bandwidthHz() / ulSlices;
        return new Channel(whole.lowHz() + w * (slice + 0.5), w);
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

    public static PhyProfile byId(String id) {
        for (PhyProfile t : values()) if (t.id.equalsIgnoreCase(id) || t.displayName.equalsIgnoreCase(id)) return t;
        return null;
    }

    private void check(int n) {
        if (!validChannel(n)) throw new IllegalArgumentException(displayName + " channel " + n + " (1-" + channels + ")");
    }
}
