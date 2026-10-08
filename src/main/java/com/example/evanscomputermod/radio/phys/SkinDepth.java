package com.example.evanscomputermod.radio.phys;

/**
 * Penetration of a plane wave into a lossy medium (earth, water, rock). The
 * attenuation constant of a medium with permittivity &epsilon; = &epsilon;0&epsilon;r,
 * permeability &mu;0 and conductivity &sigma; is
 * <pre>
 *   &alpha; = &omega; &radic;(&mu;&epsilon;/2) &middot; &radic;( &radic;(1 + (&sigma;/&omega;&epsilon;)&sup2;) &minus; 1 )   Np/m
 * </pre>
 * and the skin depth is &delta; = 1/&alpha;. For a good conductor (&sigma; &gt;&gt; &omega;&epsilon;)
 * this reduces to &delta; = &radic;(2/(&omega;&mu;&sigma;)); the full form keeps the
 * displacement-current (permittivity) correction so dielectric media at GHz are right.
 * Sea water at 10 kHz gives &delta; &asymp; 2.5 m, so VLF/LF reaches metres into
 * ground and water while GHz is stopped within millimetres.
 */
public final class SkinDepth {
    private SkinDepth() {}

    /** Attenuation constant &alpha;, nepers per metre (field). */
    public static double attenuationNpPerM(double freqHz, Ground ground) {
        if(freqHz <= 0 || ground.conductivitySPerM() == 0) return 0;
        if(ground.isPerfectConductor()) return Double.POSITIVE_INFINITY;
        double w = Units.angularFrequency(freqHz);
        double eps = Units.EPS0_F_PER_M * ground.relativePermittivity();
        double x = ground.conductivitySPerM() / (w * eps);
        double term = x * x / (Math.sqrt(1.0 + x * x) + 1.0); // sqrt(1+x^2) - 1 without cancellation
        return w * Math.sqrt(Units.MU0_H_PER_M * eps / 2.0) * Math.sqrt(term);
    }

    /** Skin depth &delta; = 1/&alpha;, metres (&infin; for a lossless medium, 0 for a perfect conductor). */
    public static double depthM(double freqHz, Ground ground) {
        double a = attenuationNpPerM(freqHz, ground);
        return a == 0 ? Double.POSITIVE_INFINITY : 1.0 / a;
    }

    /** Good-conductor skin depth &radic;(2/(&omega;&mu;0&sigma;)), metres, ignoring permittivity. */
    public static double goodConductorDepthM(double freqHz, double conductivitySPerM) {
        return Math.sqrt(2.0 / (Units.angularFrequency(freqHz) * Units.MU0_H_PER_M * conductivitySPerM));
    }

    /** Power attenuation in dB per metre of travel through the medium (8.686 &alpha;). */
    public static double attenuationDbPerM(double freqHz, Ground ground) {
        return Units.DB_PER_NEPER * attenuationNpPerM(freqHz, ground);
    }

    /** Loss through {@code depthM} of the medium, dB (refraction loss at the surface is not included). */
    public static double penetrationLossDb(double depthM, double freqHz, Ground ground) {
        if(depthM <= 0) return 0;
        return attenuationDbPerM(freqHz, ground) * depthM;
    }
}
