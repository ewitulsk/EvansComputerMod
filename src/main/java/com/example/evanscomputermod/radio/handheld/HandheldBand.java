package com.example.evanscomputermod.radio.handheld;

/** Handheld receiver bands (spec: AM broadcast, shortwave, VHF FM). */
public enum HandheldBand {
    AM(530e3, 1700e3, 10e3, 1000e3, 10e3, Mode.AM, -25),
    SW(3e6, 30e6, 5e3, 7.1e6, 6e3, Mode.AM, -12),
    VHF(30e6, 300e6, 12.5e3, 146.52e6, 12.5e3, Mode.FM, -2);

    public enum Mode { AM, FM }

    public final double minHz, maxHz, stepHz, defaultHz, bandwidthHz;
    public final Mode mode;
    /** Telescopic whip efficiency/mismatch loss at this band (it is tiny next to a 300 m MF wavelength). */
    public final double whipLossDb;

    HandheldBand(double minHz, double maxHz, double stepHz, double defaultHz, double bandwidthHz, Mode mode, double whipLossDb) {
        this.minHz = minHz;
        this.maxHz = maxHz;
        this.stepHz = stepHz;
        this.defaultHz = defaultHz;
        this.bandwidthHz = bandwidthHz;
        this.mode = mode;
        this.whipLossDb = whipLossDb;
    }

    /** VHF between 88 and 108 MHz is broadcast FM: wide (200 kHz) channels. */
    public boolean wide(double hz) {
        return this == VHF && hz >= 88e6 && hz <= 108e6;
    }

    public double bandwidth(double hz) {
        return wide(hz) ? 200e3 : bandwidthHz;
    }

    public double step(double hz) {
        return wide(hz) ? 100e3 : stepHz;
    }

    public double clamp(double hz) {
        return Math.max(minHz, Math.min(maxHz, hz));
    }

    public HandheldBand next() {
        return values()[(ordinal() + 1) % values().length];
    }
}
