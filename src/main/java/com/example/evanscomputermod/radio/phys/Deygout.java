package com.example.evanscomputermod.radio.phys;

/**
 * Multiple knife-edge diffraction by the Deygout method (ITU-R P.526 &sect;4.5.1),
 * limited to three edges: the principal edge (largest v over the whole path),
 * then the largest-v edge on each sub-path (transmitter to principal edge,
 * principal edge to receiver), each judged against the line to the principal
 * edge's top. Sub-path edges only count when the principal edge diffracts.
 * Candidate edges are terrain peaks (local maxima), see {@link #isPeak}.
 * The total is the sum of J(v) over those edges. The world is flat, so no
 * earth-bulge term is added. Allocation-free.
 */
public final class Deygout {
    public static final int MAX_EDGES = 3;

    private Deygout() {}

    /**
     * Diffraction loss over {@code profile}, dB. Antenna heights are above the
     * ground at the first (transmitter) and last (receiver) sample.
     */
    public static double lossDb(TerrainProfile profile, double freqHz, double txHeightM, double rxHeightM) {
        int n = profile.size();
        if(n < 3 || freqHz <= 0) return 0;
        double[] d = profile.distancesM();
        double[] h = profile.heightsM();
        double lambda = Units.wavelengthM(freqHz);
        double ta = h[0] + txHeightM;
        double rb = h[n - 1] + rxHeightM;
        int m = worstEdge(d, h, 0, ta, n - 1, rb, lambda);
        if(m < 0) return 0;
        double loss = KnifeEdge.lossDb(v(d, h, 0, ta, n - 1, rb, m, lambda));
        if(loss <= 0) return 0;
        int l = worstEdge(d, h, 0, ta, m, h[m], lambda);
        if(l >= 0) loss += KnifeEdge.lossDb(v(d, h, 0, ta, m, h[m], l, lambda));
        int r = worstEdge(d, h, m, h[m], n - 1, rb, lambda);
        if(r >= 0) loss += KnifeEdge.lossDb(v(d, h, m, h[m], n - 1, rb, r, lambda));
        return loss;
    }

    /** Index of the largest-v sample strictly inside (a, b) by distance, or -1. */
    private static int worstEdge(double[] d, double[] h, int a, double ha, int b, double hb, double lambda) {
        int best = -1;
        double bestV = Double.NEGATIVE_INFINITY;
        for(int i = a + 1; i < b; i++) {
            if(d[i] <= d[a] || d[i] >= d[b] || !isPeak(h, i)) continue;
            double v = v(d, h, a, ha, b, hb, i, lambda);
            if(v > bestV) {
                bestV = v;
                best = i;
            }
        }
        return best;
    }

    /**
     * Only local maxima of the profile are knife edges. Flat or falling ground is
     * not an edge: its effect inside the Fresnel zone is ground reflection
     * ({@link TwoRay}) or ground wave, and counting every flat sample as an edge
     * would make HF (whose Fresnel zone spans tens of metres) lose diffraction
     * loss on open plains, the classic Deygout overestimate.
     */
    private static boolean isPeak(double[] h, int i) {
        double l = h[i - 1], c = h[i], r = h[i + 1];
        return c >= l && c >= r && (c > l || c > r);
    }

    private static double v(double[] d, double[] h, int a, double ha, int b, double hb, int i, double lambda) {
        double d1 = d[i] - d[a];
        double line = ha + (hb - ha) * d1 / (d[b] - d[a]);
        return KnifeEdge.v(h[i] - line, d1, d[b] - d[i], lambda);
    }
}
