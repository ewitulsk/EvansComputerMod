package com.example.evanscomputermod.radio.api;

/**
 * A slice of spectrum: centre frequency and occupied bandwidth. Wi-Fi channels
 * use the 802.11 numbering; everything else is a free frequency.
 */
public record Channel(double centerHz, double bandwidthHz) {

    public Channel {
        if (!(centerHz > 0) || !(bandwidthHz > 0)) throw new IllegalArgumentException("bad channel " + centerHz + "/" + bandwidthHz);
    }

    public Band band() {
        return Band.of(centerHz);
    }

    public double lowHz() {
        return centerHz - bandwidthHz / 2;
    }

    public double highHz() {
        return centerHz + bandwidthHz / 2;
    }

    /** True if the two channels share any spectrum. */
    public boolean overlaps(Channel o) {
        return lowHz() < o.highHz() && o.lowHz() < highHz();
    }

    /** 2.4 GHz Wi-Fi channel 1–13 (22 MHz wide, 5 MHz spacing). */
    public static Channel wifi24(int number) {
        if (number < 1 || number > 13) throw new IllegalArgumentException("2.4 GHz channel " + number);
        return new Channel(2.407e9 + 5e6 * number, 22e6);
    }

    /** 5 GHz Wi-Fi channel by number (centre = 5000 + 5n MHz), 20 or 40 MHz wide. */
    public static Channel wifi5(int number, int widthMhz) {
        if (number < 32 || number > 177) throw new IllegalArgumentException("5 GHz channel " + number);
        if (widthMhz != 20 && widthMhz != 40) throw new IllegalArgumentException("width " + widthMhz);
        return new Channel(5.000e9 + 5e6 * number, widthMhz * 1e6);
    }

    /** The 802.11 channel number of a Wi-Fi channel, or -1. */
    public int wifiNumber() {
        if (band() == Band.WIFI_2G4) return (int) Math.round((centerHz - 2.407e9) / 5e6);
        if (band() == Band.WIFI_5G) return (int) Math.round((centerHz - 5.000e9) / 5e6);
        return -1;
    }
}
