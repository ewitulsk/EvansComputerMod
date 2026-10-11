package com.example.evanscomputermod.radio.phys;

/**
 * Fresnel zones. The n-th zone radius at a point d1 from one end and d2 from
 * the other is r<sub>n</sub> = &radic;(n &lambda; d1 d2 / (d1 + d2)). A 100 m
 * link at 2.4 GHz needs r1 &asymp; 1.77 m of clearance at mid-path.
 */
public final class Fresnel {
    private Fresnel() {}

    /** Radius of Fresnel zone {@code zone} (1 = first zone), metres; 0 at an endpoint. */
    public static double radiusM(double freqHz, double d1M, double d2M, int zone) {
        if(d1M <= 0 || d2M <= 0 || zone <= 0 || freqHz <= 0) return 0;
        return Math.sqrt(zone * Units.wavelengthM(freqHz) * d1M * d2M / (d1M + d2M));
    }

    /** First-zone radius at mid-path of a link {@code distanceM} long (its maximum), metres. */
    public static double midpathRadiusM(double freqHz, double distanceM) {
        return radiusM(freqHz, distanceM / 2, distanceM / 2, 1);
    }

    /**
     * Loss from an obstacle whose top is {@code clearanceM} below the line of sight
     * (negative clearance means it pokes through), treated as a knife edge. Zero
     * once clearance exceeds about 0.55 of the first Fresnel radius (v &le; &minus;0.78).
     */
    public static double clearanceLossDb(double clearanceM, double freqHz, double d1M, double d2M) {
        return KnifeEdge.lossDb(KnifeEdge.v(-clearanceM, d1M, d2M, Units.wavelengthM(freqHz)));
    }

    /** Clearance as a fraction of the first Fresnel radius at that point (1 = full zone clear). */
    public static double clearanceRatio(double clearanceM, double freqHz, double d1M, double d2M) {
        double r = radiusM(freqHz, d1M, d2M, 1);
        return r == 0 ? Double.POSITIVE_INFINITY : clearanceM / r;
    }
}
