package com.example.evanscomputermod.radio.microwave;

import com.example.evanscomputermod.radio.api.Channel;

/**
 * Microwave link bands and their channel plans. Channel {@code n} of width
 * {@code bw} is centred at {@code lowEdge + bw (n + ½)} and must fit below the
 * high edge, so wider channels mean fewer of them. Two radios pair by using the
 * same band, width and channel number.
 *
 * <ul>
 *   <li>10 GHz: 10.00–10.68 GHz, 28 or 56 MHz channels (licensed backhaul style).</li>
 *   <li>24 GHz: 24.00–24.25 GHz ISM, 28 / 56 / 112 MHz.</li>
 *   <li>60 GHz: 57–60 GHz (kept inside {@code Band.MICROWAVE}), 250 MHz to 2 GHz; oxygen absorption limits hops.</li>
 * </ul>
 */
public enum MwBand {
    GHZ_10(10, 10.00e9, 10.68e9, new int[] {28, 56}, 56),
    GHZ_24(24, 24.00e9, 24.25e9, new int[] {28, 56, 112}, 56),
    GHZ_60(60, 57.00e9, 60.00e9, new int[] {250, 500, 1000, 2000}, 1000);

    public final int ghz;
    public final double lowEdgeHz, highEdgeHz;
    private final int[] widthsMhz;
    public final int defaultWidthMhz;

    MwBand(int ghz, double lowEdgeHz, double highEdgeHz, int[] widthsMhz, int defaultWidthMhz) {
        this.ghz = ghz;
        this.lowEdgeHz = lowEdgeHz;
        this.highEdgeHz = highEdgeHz;
        this.widthsMhz = widthsMhz;
        this.defaultWidthMhz = defaultWidthMhz;
    }

    public int[] widthsMhz() {
        return widthsMhz.clone();
    }

    public boolean allowsWidth(int mhz) {
        for (int w : widthsMhz) if (w == mhz) return true;
        return false;
    }

    /** How many channels of {@code widthMhz} fit in the band. */
    public int channelCount(int widthMhz) {
        return (int) Math.floor((highEdgeHz - lowEdgeHz) / (widthMhz * 1e6) + 1e-9);
    }

    public Channel channel(int number, int widthMhz) {
        if (!allowsWidth(widthMhz)) throw new IllegalArgumentException(ghz + " GHz band has no " + widthMhz + " MHz channels");
        if (number < 0 || number >= channelCount(widthMhz))
            throw new IllegalArgumentException(ghz + " GHz band, " + widthMhz + " MHz: channel must be 0-" + (channelCount(widthMhz) - 1));
        double bw = widthMhz * 1e6;
        return new Channel(lowEdgeHz + bw * (number + 0.5), bw);
    }

    public static MwBand of(int ghz) {
        for (MwBand b : values()) if (b.ghz == ghz) return b;
        throw new IllegalArgumentException("band must be 10, 24 or 60 (GHz), got " + ghz);
    }
}
