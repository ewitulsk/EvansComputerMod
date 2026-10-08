package com.example.evanscomputermod.radio.phys;

/**
 * Surface (ground) wave for antennas near the ground, the dominant mode at
 * VLF/LF/MF. Uses the Sommerfeld-Norton flat-earth attenuation factor in the
 * Terman approximation:
 * <pre>
 *   x = &sigma;/(&omega;&epsilon;0) = 60&sigma;&lambda;
 *   p<sub>v</sub> = (&pi;R/&lambda;) / &radic;(x&sup2; + (&epsilon;r+1)&sup2;)       (vertical; = &pi;R cos b/(&lambda;x))
 *   p<sub>h</sub> = (&pi;R/&lambda;) &middot; &radic;(x&sup2; + (&epsilon;r&minus;1)&sup2;)    (horizontal; = &pi;Rx/(&lambda; cos b'))
 *   A &asymp; (2 + 0.3p) / (2 + p + 0.6p&sup2;)
 * </pre>
 * The field over the ground is twice free space (image) times A, so
 * L = L<sub>fs</sub> &minus; 6.02 &minus; 20 log10 A. Over sea water at MF A stays near 1
 * for kilometres; over land at GHz it collapses within metres; horizontal
 * polarization has essentially no ground wave. Also hosts {@link #penetrationLossDb}
 * for links into ground or water (mines, underwater bases) via {@link SkinDepth}.
 */
public final class GroundWave {
    /** 20 log10 2: field doubling over a perfectly conducting plane. */
    public static final double GROUND_GAIN_DB = 20.0 * Math.log10(2.0);

    private GroundWave() {}

    /** Norton numerical distance p for the given polarization (circular uses vertical). */
    public static double numericalDistance(double distanceM, double freqHz, Ground ground, Polarization pol) {
        if(ground.isPerfectConductor() || distanceM <= 0) return 0;
        double k = Math.PI * distanceM / Units.wavelengthM(freqHz);
        double x = ground.lossFactor(freqHz);
        double er = ground.relativePermittivity();
        if(pol == Polarization.HORIZONTAL) return k * Math.sqrt(x * x + (er - 1) * (er - 1));
        return k / Math.sqrt(x * x + (er + 1) * (er + 1));
    }

    /** Field attenuation factor |A| in (0, 1]. Circular polarization averages the vertical and horizontal power. */
    public static double attenuationFactor(double distanceM, double freqHz, Ground ground, Polarization pol) {
        if(pol.isCircular()) {
            double v = factor(numericalDistance(distanceM, freqHz, ground, Polarization.VERTICAL));
            double h = factor(numericalDistance(distanceM, freqHz, ground, Polarization.HORIZONTAL));
            return Math.sqrt((v * v + h * h) / 2);
        }
        return factor(numericalDistance(distanceM, freqHz, ground, pol));
    }

    private static double factor(double p) {
        return (2.0 + 0.3 * p) / (2.0 + p + 0.6 * p * p);
    }

    /** Ground-wave path loss, dB (never below 0). */
    public static double lossDb(double distanceM, double freqHz, Ground ground, Polarization pol) {
        if(distanceM <= 0 || freqHz <= 0) return 0;
        double a = attenuationFactor(distanceM, freqHz, ground, pol);
        double l = FreeSpace.lossDb(distanceM, freqHz) - GROUND_GAIN_DB - 20.0 * Math.log10(a);
        return Math.max(0, l);
    }

    /** Extra loss for an endpoint buried {@code depthM} in {@code medium} (skin-depth attenuation). */
    public static double penetrationLossDb(double depthM, double freqHz, Ground medium) {
        return SkinDepth.penetrationLossDb(depthM, freqHz, medium);
    }
}
