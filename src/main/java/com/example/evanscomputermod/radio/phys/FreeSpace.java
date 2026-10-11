package com.example.evanscomputermod.radio.phys;

/**
 * Friis free-space path loss between isotropic antennas:
 * L = 20 log10(4&pi;d/&lambda;) = 20 log10 d + 20 log10 f + 20 log10(4&pi;/c)
 * = 20 log10 d<sub>m</sub> + 20 log10 f<sub>Hz</sub> &minus; 147.55 dB.
 */
public final class FreeSpace {
    /** 20 log10(4&pi;/c) &asymp; &minus;147.55 dB. */
    public static final double FRIIS_CONSTANT_DB = 20.0 * Math.log10(4.0 * Math.PI / Units.SPEED_OF_LIGHT_MPS);

    private FreeSpace() {}

    /**
     * Free-space loss in dB. The far-field formula would go negative inside
     * d &lt; &lambda;/4&pi; (near field); it is clamped to 0 dB there, and for
     * non-positive distance or frequency.
     */
    public static double lossDb(double distanceM, double freqHz) {
        if(distanceM <= 0 || freqHz <= 0) return 0;
        double l = 20.0 * Math.log10(distanceM * freqHz) + FRIIS_CONSTANT_DB;
        return l > 0 ? l : 0;
    }

    /** Distance at which free-space loss reaches {@code lossDb} (inverse Friis), metres. */
    public static double distanceForLossM(double lossDb, double freqHz) {
        return Math.pow(10.0, (lossDb - FRIIS_CONSTANT_DB) / 20.0) / freqHz;
    }
}
