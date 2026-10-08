package com.example.evanscomputermod.radio.antenna.solver;

/**
 * Far-field gain on a 5° grid. θ is measured from the zenith (+z, 0°) to the
 * nadir (180°); φ from +x towards +y, 0°–355°. Gains are IEEE gain (they
 * include ohmic and ground losses), relative to an isotropic radiator fed with
 * the same input power. Over a ground plane everything below the horizon
 * (θ &gt; 90°) is zero.
 *
 * <p>The θ component is vertical polarisation at the horizon; the φ component
 * is horizontal. The relative phase of the two is kept so the axial ratio
 * (circular polarisation) can be read too.
 */
public final class GainPattern {
    public static final int STEP_DEG = 5;
    public static final int THETA_COUNT = 180 / STEP_DEG + 1;
    public static final int PHI_COUNT = 360 / STEP_DEG;
    /** dBi reported for zero gain. */
    public static final double FLOOR_DBI = -999;

    private final double[] gTheta, gPhi, phase;
    private final int peakIndex;

    GainPattern(double[] gTheta, double[] gPhi, double[] phase) {
        this.gTheta = gTheta;
        this.gPhi = gPhi;
        this.phase = phase;
        int best = 0;
        for (int i = 1; i < gTheta.length; i++) if (gTheta[i] + gPhi[i] > gTheta[best] + gPhi[best]) best = i;
        peakIndex = best;
    }

    private static int index(int thetaIndex, int phiIndex) {
        if (thetaIndex < 0 || thetaIndex >= THETA_COUNT) throw new IndexOutOfBoundsException("theta index " + thetaIndex);
        return thetaIndex * PHI_COUNT + Math.floorMod(phiIndex, PHI_COUNT);
    }

    public static double thetaDeg(int thetaIndex) { return thetaIndex * STEP_DEG; }

    public static double phiDeg(int phiIndex) { return phiIndex * STEP_DEG; }

    /** Linear total gain. */
    public double gain(int thetaIndex, int phiIndex) {
        int i = index(thetaIndex, phiIndex);
        return gTheta[i] + gPhi[i];
    }

    public double gainTheta(int thetaIndex, int phiIndex) { return gTheta[index(thetaIndex, phiIndex)]; }

    public double gainPhi(int thetaIndex, int phiIndex) { return gPhi[index(thetaIndex, phiIndex)]; }

    public double totalDbi(int thetaIndex, int phiIndex) { return dbi(gain(thetaIndex, phiIndex)); }

    public double thetaDbi(int thetaIndex, int phiIndex) { return dbi(gainTheta(thetaIndex, phiIndex)); }

    public double phiDbi(int thetaIndex, int phiIndex) { return dbi(gainPhi(thetaIndex, phiIndex)); }

    /** Fraction of the power in the θ (vertical at the horizon) component, 0..1. */
    public double verticalFraction(int thetaIndex, int phiIndex) {
        double g = gain(thetaIndex, phiIndex);
        return g > 0 ? gainTheta(thetaIndex, phiIndex) / g : 0;
    }

    /** Polarisation-ellipse axial ratio in dB: 0 = circular, large = linear. */
    public double axialRatioDb(int thetaIndex, int phiIndex) {
        int i = index(thetaIndex, phiIndex);
        double a2 = gTheta[i], b2 = gPhi[i], s = a2 + b2;
        if (s <= 0) return Double.POSITIVE_INFINITY;
        double root = Math.sqrt(Math.max(0, a2 * a2 + b2 * b2 + 2 * a2 * b2 * Math.cos(2 * phase[i])));
        double major = (s + root) / 2, minor = (s - root) / 2;
        if (minor <= major * 1e-12) return Double.POSITIVE_INFINITY;
        return 10 * Math.log10(major / minor);
    }

    /** Total gain in dBi at any direction, bilinear in linear gain between grid points. */
    public double gainDbi(double thetaDeg, double phiDeg) {
        double t = Math.max(0, Math.min(180, thetaDeg)) / STEP_DEG;
        double p = Math.floorMod((long) Math.floor(phiDeg * 1e6), 360_000_000L) / 1e6 / STEP_DEG;
        int t0 = Math.min((int) Math.floor(t), THETA_COUNT - 2), p0 = (int) Math.floor(p);
        double ft = t - t0, fp = p - p0;
        double g = gain(t0, p0) * (1 - ft) * (1 - fp) + gain(t0 + 1, p0) * ft * (1 - fp)
                + gain(t0, p0 + 1) * (1 - ft) * fp + gain(t0 + 1, p0 + 1) * ft * fp;
        return dbi(g);
    }

    public double peakDbi() { return dbi(gTheta[peakIndex] + gPhi[peakIndex]); }

    public int peakThetaIndex() { return peakIndex / PHI_COUNT; }

    public int peakPhiIndex() { return peakIndex % PHI_COUNT; }

    public static double dbi(double linear) { return linear > 0 ? 10 * Math.log10(linear) : FLOOR_DBI; }
}
