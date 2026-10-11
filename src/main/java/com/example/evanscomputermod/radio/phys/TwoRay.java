package com.example.evanscomputermod.radio.phys;

/**
 * Two-ray ground-reflection model. The received field is the direct ray plus a
 * ray reflected off flat ground:
 * E/E<sub>fs</sub> = 1 + &Gamma;(&psi;) (r1/r2) e<sup>&minus;jk(r2&minus;r1)</sup>,
 * with r1 = &radic;(d&sup2; + (h<sub>t</sub>&minus;h<sub>r</sub>)&sup2;),
 * r2 = &radic;(d&sup2; + (h<sub>t</sub>+h<sub>r</sub>)&sup2;) and grazing angle
 * sin&psi; = (h<sub>t</sub>+h<sub>r</sub>)/r2. The Fresnel reflection coefficients for
 * complex permittivity &epsilon;<sub>c</sub> = &epsilon;<sub>r</sub> &minus; j60&sigma;&lambda; are
 * <pre>
 *   &Gamma;h = (sin&psi; &minus; &radic;(&epsilon;c &minus; cos&sup2;&psi;)) / (sin&psi; + &radic;(&epsilon;c &minus; cos&sup2;&psi;))
 *   &Gamma;v = (&epsilon;c sin&psi; &minus; &radic;(&epsilon;c &minus; cos&sup2;&psi;)) / (&epsilon;c sin&psi; + &radic;(&epsilon;c &minus; cos&sup2;&psi;))
 * </pre>
 * (Rappaport; ITU-R P.1546 annex). Circular polarization keeps the co-polar
 * part (&Gamma;v + &Gamma;h)/2. Both tend to &minus;1 at grazing, so past the
 * crossover distance d<sub>c</sub> = 4&pi;h<sub>t</sub>h<sub>r</sub>/&lambda; the loss approaches
 * 40 log10 d &minus; 20 log10(h<sub>t</sub>h<sub>r</sub>) and antenna height matters.
 */
public final class TwoRay {
    /** Deepest destructive null allowed, as a power ratio (&minus;60 dB). Real ground is never perfectly flat. */
    private static final double MIN_FACTOR = 1e-6;
    /** Antenna heights below this are lifted to it (an antenna is never exactly in the ground plane). */
    public static final double MIN_HEIGHT_M = 0.05;

    private TwoRay() {}

    /** d<sub>c</sub> = 4&pi;h<sub>t</sub>h<sub>r</sub>/&lambda;: beyond it the loss slope becomes 40 dB/decade. */
    public static double crossoverDistanceM(double freqHz, double txHeightM, double rxHeightM) {
        return 4.0 * Math.PI * txHeightM * rxHeightM / Units.wavelengthM(freqHz);
    }

    /** Plane-earth asymptote 40 log10 d &minus; 20 log10(h<sub>t</sub>h<sub>r</sub>), dB (valid well past crossover). */
    public static double planeEarthLossDb(double distanceM, double txHeightM, double rxHeightM) {
        return 40.0 * Math.log10(distanceM) - 20.0 * Math.log10(txHeightM * rxHeightM);
    }

    /** Ground reflection coefficient at grazing angle {@code grazingRad}. */
    public static Complex reflectionCoefficient(double grazingRad, double freqHz, Ground ground, Polarization pol) {
        double s = Math.sin(grazingRad);
        if(ground.isPerfectConductor()) {
            return switch(pol) {
                case VERTICAL -> Complex.ONE;
                case HORIZONTAL -> new Complex(-1, 0);
                default -> Complex.ZERO;
            };
        }
        double c2 = 1.0 - s * s;
        Complex eps = ground.complexPermittivity(freqHz);
        Complex root = eps.plus(-c2).sqrt();
        Complex gh = new Complex(s, 0).minus(root).div(new Complex(s, 0).plus(root));
        if(pol == Polarization.HORIZONTAL) return gh;
        Complex es = eps.times(s);
        Complex gv = es.minus(root).div(es.plus(root));
        if(pol == Polarization.VERTICAL) return gv;
        return gv.plus(gh).times(0.5);
    }

    /**
     * Two-ray path loss in dB for horizontal ground distance {@code distanceM}
     * and antenna heights above the reflecting ground.
     */
    public static double lossDb(double distanceM, double freqHz, double txHeightM, double rxHeightM,
            Ground ground, Polarization pol) {
        double ht = Math.max(MIN_HEIGHT_M, txHeightM);
        double hr = Math.max(MIN_HEIGHT_M, rxHeightM);
        double d = Math.max(0, distanceM);
        double r1 = Math.hypot(d, ht - hr);
        double r2 = Math.hypot(d, ht + hr);
        if(r1 <= 0 || freqHz <= 0) return 0;
        Complex g = reflectionCoefficient(Math.asin((ht + hr) / r2), freqHz, ground, pol);
        double pathDiff = 4.0 * ht * hr / (r1 + r2); // r2 - r1 without cancellation
        double phase = -2.0 * Math.PI * pathDiff / Units.wavelengthM(freqHz);
        double ratio = r1 / r2;
        double cr = Math.cos(phase), ci = Math.sin(phase);
        double re = 1.0 + ratio * (g.re() * cr - g.im() * ci);
        double im = ratio * (g.re() * ci + g.im() * cr);
        double factor = Math.max(MIN_FACTOR, re * re + im * im);
        // Inside the near field (where Friis is clamped to 0 dB) the reflection must not turn the
        // path into a gain: path loss is never negative.
        return Math.max(0, FreeSpace.lossDb(r1, freqHz) - 10.0 * Math.log10(factor));
    }
}
