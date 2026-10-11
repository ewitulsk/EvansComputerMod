package com.example.evanscomputermod.radio.wifi.ap;

import com.example.evanscomputermod.radio.api.AntennaPattern;

/**
 * The Access Point's built-in antenna: a vertical omni (a short collinear
 * array) with {@value #PEAK_DBI} dBi at the horizon, a sin^4 elevation
 * pattern (a narrower beam than a dipole) and deep nulls straight up and down,
 * vertically polarized. Local frame: +Y up.
 */
public final class ApAntenna implements AntennaPattern {

    public static final double PEAK_DBI = 5;
    /** Gain floor in the nulls (dBi). */
    public static final double NULL_DBI = -20;
    public static final ApAntenna INSTANCE = new ApAntenna();

    private ApAntenna() {}

    @Override
    public double gainDbi(double lx, double ly, double lz) {
        double len = Math.sqrt(lx * lx + ly * ly + lz * lz);
        if (len == 0) return PEAK_DBI;
        double c = ly / len;
        double s2 = Math.max(0, 1 - c * c);
        return Math.max(NULL_DBI, PEAK_DBI + 10 * Math.log10(Math.max(1e-9, s2 * s2)));
    }

    @Override
    public double[] polarization(double lx, double ly, double lz) {
        return new double[] {0, 1, 0};
    }

    @Override
    public double peakGainDbi() {
        return PEAK_DBI;
    }
}
