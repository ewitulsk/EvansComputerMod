package com.example.evanscomputermod.radio.microwave;

import com.example.evanscomputermod.radio.api.AntennaPattern;

/**
 * A parabolic dish of diameter D at one frequency, boresight along local +Z,
 * vertical polarization.
 *
 * <ul>
 *   <li><b>Peak gain</b> G0 = eta (pi D / lambda)^2, eta = 0.55.</li>
 *   <li><b>Main lobe and near sidelobes</b>: the far field of a circular aperture
 *       with a parabolic-on-pedestal taper, E(u) = 0.15 * 2 J1(u)/u + 0.85 * 8 J2(u)/u^2,
 *       u = (pi D / lambda) sin(theta). This taper gives a -3 dB beamwidth of 70 lambda/D
 *       and first sidelobes near -24 dB.</li>
 *   <li><b>Far sidelobes and back</b>: never below the ITU-R F.699 reference
 *       envelope minus 10 dB (typical dishes sit about that far under it), so a
 *       dish turned away still hears a little, with a front-to-back ratio of
 *       roughly G0 + 20 dB.</li>
 * </ul>
 */
public final class DishPattern implements AntennaPattern {
    public static final double EFFICIENCY = 0.55;
    /** Pedestal/parabolic taper mix; 0.85 puts the -3 dB beamwidth at 70 lambda/D. */
    static final double TAPER = 0.85;
    private static final double C = 299_792_458.0;

    private final double diameterM, freqHz, ka, dOverLambda, g0Dbi, g1Dbi;
    private final double feedLossDb;

    public DishPattern(double diameterM, double freqHz) {
        this(diameterM, freqHz, 0);
    }

    public DishPattern(double diameterM, double freqHz, double feedLossDb) {
        if (!(diameterM > 0) || !(freqHz > 0)) throw new IllegalArgumentException("dish " + diameterM + " m at " + freqHz + " Hz");
        this.diameterM = diameterM;
        this.freqHz = freqHz;
        double lambda = C / freqHz;
        this.ka = Math.PI * diameterM / lambda;
        this.dOverLambda = diameterM / lambda;
        this.g0Dbi = formulaGainDbi(diameterM, freqHz);
        this.g1Dbi = 2 + 15 * Math.log10(dOverLambda);
        this.feedLossDb = feedLossDb;
    }

    /** G0 = eta (pi D / lambda)^2 in dBi. */
    public static double formulaGainDbi(double diameterM, double freqHz) {
        double ka = Math.PI * diameterM * freqHz / C;
        return 10 * Math.log10(EFFICIENCY * ka * ka);
    }

    /** The -3 dB beamwidth rule of thumb, 70 lambda/D degrees. */
    public static double ruleBeamwidthDeg(double diameterM, double freqHz) {
        return 70 * (C / freqHz) / diameterM;
    }

    public double diameterM() { return diameterM; }
    public double freqHz() { return freqHz; }

    public double beamwidthDeg() {
        return ruleBeamwidthDeg(diameterM, freqHz);
    }

    /** Gain at {@code thetaRad} off boresight, dBi. */
    public double gainOffAxisDbi(double thetaRad) {
        double t = Math.abs(thetaRad);
        double floor = envelopeDbi(Math.toDegrees(t)) - 10;
        if (t >= Math.PI / 2) return floor;
        double u = ka * Math.sin(t);
        double e = aperture(u);
        double main = g0Dbi + 20 * Math.log10(Math.max(1e-9, Math.abs(e)));
        return Math.max(main, floor);
    }

    /** Normalised field of the tapered aperture (1 at u = 0). */
    static double aperture(double u) {
        if (u < 1e-3) return 1 - u * u * ((1 - TAPER) / 8 + TAPER / 12);
        double j0 = Bessel.j0(u), j1 = Bessel.j1(u);
        double j2 = 2 * j1 / u - j0;
        return (1 - TAPER) * 2 * j1 / u + TAPER * 8 * j2 / (u * u);
    }

    /** ITU-R F.699 reference sidelobe envelope beyond the main lobe, dBi. */
    private double envelopeDbi(double phiDeg) {
        double phi = Math.max(phiDeg, 1e-3);
        if (dOverLambda > 100) {
            double phiR = 15.85 * Math.pow(dOverLambda, -0.6);
            if (phi < phiR) return g1Dbi;
            if (phi < 48) return Math.min(g1Dbi, 32 - 25 * Math.log10(phi));
            return -10;
        }
        double phiR = 100 / dOverLambda;
        if (phi < phiR) return g1Dbi;
        if (phi < 48) return Math.min(g1Dbi, 52 - 10 * Math.log10(dOverLambda) - 25 * Math.log10(phi));
        return 10 - 10 * Math.log10(dOverLambda);
    }

    @Override
    public double gainDbi(double lx, double ly, double lz) {
        double len = Math.sqrt(lx * lx + ly * ly + lz * lz);
        if (len == 0) return g0Dbi;
        return gainOffAxisDbi(Math.acos(Math.max(-1, Math.min(1, lz / len))));
    }

    @Override
    public double[] polarization(double lx, double ly, double lz) {
        return new double[] {0, 1, 0};
    }

    @Override
    public double peakGainDbi() {
        return g0Dbi;
    }

    @Override
    public double feedLossDb() {
        return feedLossDb;
    }

    /** Bessel functions of the first kind (Numerical Recipes rational/asymptotic fits, |error| below 1e-8). */
    static final class Bessel {
        private Bessel() {}

        static double j0(double x) {
            double ax = Math.abs(x);
            if (ax < 8.0) {
                double y = x * x;
                double a1 = 57568490574.0 + y * (-13362590354.0 + y * (651619640.7 + y * (-11214424.18 + y * (77392.33017 + y * (-184.9052456)))));
                double a2 = 57568490411.0 + y * (1029532985.0 + y * (9494680.718 + y * (59272.64853 + y * (267.8532712 + y))));
                return a1 / a2;
            }
            double z = 8.0 / ax, y = z * z, xx = ax - 0.785398164;
            double a1 = 1.0 + y * (-0.1098628627e-2 + y * (0.2734510407e-4 + y * (-0.2073370639e-5 + y * 0.2093887211e-6)));
            double a2 = -0.1562499995e-1 + y * (0.1430488765e-3 + y * (-0.6911147651e-5 + y * (0.7621095161e-6 - y * 0.934935152e-7)));
            return Math.sqrt(0.636619772 / ax) * (Math.cos(xx) * a1 - z * Math.sin(xx) * a2);
        }

        static double j1(double x) {
            double ax = Math.abs(x);
            if (ax < 8.0) {
                double y = x * x;
                double a1 = x * (72362614232.0 + y * (-7895059235.0 + y * (242396853.1 + y * (-2972611.439 + y * (15704.48260 + y * (-30.16036606))))));
                double a2 = 144725228442.0 + y * (2300535178.0 + y * (18583304.74 + y * (99447.43394 + y * (376.9991397 + y))));
                return a1 / a2;
            }
            double z = 8.0 / ax, y = z * z, xx = ax - 2.356194491;
            double a1 = 1.0 + y * (0.183105e-2 + y * (-0.3516396496e-4 + y * (0.2457520174e-5 + y * (-0.240337019e-6))));
            double a2 = 0.04687499995 + y * (-0.2002690873e-3 + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
            double ans = Math.sqrt(0.636619772 / ax) * (Math.cos(xx) * a1 - z * Math.sin(xx) * a2);
            return x < 0 ? -ans : ans;
        }
    }
}
