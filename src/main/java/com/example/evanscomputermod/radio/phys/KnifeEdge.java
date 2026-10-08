package com.example.evanscomputermod.radio.phys;

/**
 * Single knife-edge diffraction, ITU-R P.526 &sect;4.1.
 *
 * <p>The Fresnel-Kirchhoff parameter is v = h &radic;(2/&lambda; &middot; (1/d1 + 1/d2)),
 * where h is the height of the edge above the straight line between the
 * terminals (negative when the line clears the edge). The loss uses the
 * P.526 approximation J(v) = 6.9 + 20 log10(&radic;((v&minus;0.1)&sup2;+1) + v &minus; 0.1),
 * valid for v &gt; &minus;0.78; J is 0 dB at and below that.
 */
public final class KnifeEdge {
    /** At or below this v the edge is clear of the path and J(v) is taken as 0 dB (P.526). */
    public static final double V_MIN = -0.78;

    private KnifeEdge() {}

    /** Diffraction loss J(v) in dB (always &ge; 0). */
    public static double lossDb(double v) {
        if(v <= V_MIN) return 0;
        double a = v - 0.1;
        double j = 6.9 + 20.0 * Math.log10(Math.sqrt(a * a + 1.0) + a);
        return j > 0 ? j : 0;
    }

    /**
     * Fresnel-Kirchhoff diffraction parameter. {@code hM} is the edge height above
     * the line of sight, {@code d1M}/{@code d2M} the distances from each terminal
     * to the edge. An edge at a terminal (d &le; 0) gives &plusmn;&infin; by the sign of h.
     */
    public static double v(double hM, double d1M, double d2M, double wavelengthM) {
        if(d1M <= 0 || d2M <= 0) return hM > 0 ? Double.POSITIVE_INFINITY : Double.NEGATIVE_INFINITY;
        return hM * Math.sqrt(2.0 / wavelengthM * (1.0 / d1M + 1.0 / d2M));
    }

    /** Diffraction loss of one edge from its geometry and the frequency, dB. */
    public static double lossDb(double hM, double d1M, double d2M, double freqHz) {
        return lossDb(v(hM, d1M, d2M, Units.wavelengthM(freqHz)));
    }
}
