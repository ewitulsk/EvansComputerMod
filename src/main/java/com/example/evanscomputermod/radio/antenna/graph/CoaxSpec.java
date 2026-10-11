package com.example.evanscomputermod.radio.antenna.graph;

/**
 * A feedline tier. Matched-line loss follows the usual two-term model
 * {@code dB = a·√f + b·f} (conductor skin loss + dielectric loss), fitted
 * through two published points per 10 blocks (10 m).
 *
 * @param name        display name
 * @param lossAt10MHz loss in dB per 10 blocks at 10 MHz
 * @param lossAt1GHz  loss in dB per 10 blocks at 1 GHz
 * @param maxPowerW   continuous power rating at 10 MHz (falls as 1/√f above), W
 */
public record CoaxSpec(String name, double lossAt10MHz, double lossAt1GHz, double maxPowerW) {
    private static final double F1 = 10e6, F2 = 1e9;

    public CoaxSpec {
        if (!(lossAt10MHz > 0) || !(lossAt1GHz > lossAt10MHz) || !(maxPowerW > 0))
            throw new IllegalArgumentException("bad coax spec " + name);
    }

    /** Loss in dB per 10 blocks at a frequency. */
    public double lossPer10Db(double hz) {
        // Solve a·√F1 + b·F1 = L1, a·√F2 + b·F2 = L2.
        double s1 = Math.sqrt(F1), s2 = Math.sqrt(F2);
        double det = s1 * F2 - s2 * F1;
        double a = (lossAt10MHz * F2 - lossAt1GHz * F1) / det;
        double b = (s1 * lossAt1GHz - s2 * lossAt10MHz) / det;
        return Math.max(0, a * Math.sqrt(hz) + b * hz);
    }

    /** Loss in dB over {@code blocks} blocks. */
    public double lossDb(double hz, int blocks) {
        return lossPer10Db(hz) * blocks / 10.0;
    }

    /** Continuous power rating at a frequency (heating grows with conductor loss, ∝ √f). */
    public double powerRatingW(double hz) {
        return hz <= F1 ? maxPowerW : maxPowerW * Math.sqrt(F1 / hz);
    }
}
