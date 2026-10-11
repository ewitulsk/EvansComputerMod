package com.example.evanscomputermod.radio.phys;

/**
 * Transmit spectral masks and the adjacent-channel interference (ACI) weight
 * between two channels: the fraction of the interferer's power that falls
 * inside the victim's (rectangular) receive band,
 * w = &int;<sub>rx band</sub> PSD(f &minus; f<sub>tx</sub>) df / &int; PSD df.
 *
 * <p>Masks are piecewise linear in dB over offset in units of the nominal
 * bandwidth B (the mask's own scaling), truncated at &plusmn;2B:
 * <ul>
 * <li>{@link #DSSS_22MHZ} (802.11b, 17.4.6.5): 0 dBr to &plusmn;11 MHz, &minus;30 dBr to &plusmn;22 MHz, &minus;50 dBr beyond.</li>
 * <li>{@link #OFDM_20MHZ} (802.11a/g, 17.3.9.3): 0 dBr to &plusmn;9 MHz, &minus;20 at 11, &minus;28 at 20, &minus;40 at 30 MHz.</li>
 * <li>{@link #OFDM_40MHZ} (802.11n HT40): 0 to &plusmn;19, &minus;20 at 21, &minus;28 at 40, &minus;40 at 60 MHz.</li>
 * <li>{@link #GENERIC}: 0 to &plusmn;B/2, &minus;25 at 0.6B, &minus;40 at B, &minus;60 at 1.5B.</li>
 * </ul>
 * Each mask precomputes a cumulative table so {@link #weight} is O(1) and allocation-free.
 * Real spectra sit below their masks, so the weights are conservative.
 */
public enum SpectralMask {
    DSSS_22MHZ(22e6, new double[] {0, 0.5, 0.50001, 1.0, 1.00001, 2.0}, new double[] {0, 0, -30, -30, -50, -50}),
    OFDM_20MHZ(20e6, new double[] {0, 0.45, 0.55, 1.0, 1.5, 2.0}, new double[] {0, 0, -20, -28, -40, -40}),
    OFDM_40MHZ(40e6, new double[] {0, 0.475, 0.525, 1.0, 1.5, 2.0}, new double[] {0, 0, -20, -28, -40, -40}),
    GENERIC(Double.NaN, new double[] {0, 0.5, 0.6, 1.0, 1.5, 2.0}, new double[] {0, 0, -25, -40, -60, -60});

    private static final double SPAN = 2.0;
    private static final int STEPS = 4000;

    private final double nominalBandwidthHz;
    private final double[] u, db;
    /** Cumulative normalised power from -SPAN to u_i, i in [0, STEPS]. */
    private final double[] cdf = new double[STEPS + 1];

    SpectralMask(double nominalBandwidthHz, double[] u, double[] db) {
        this.nominalBandwidthHz = nominalBandwidthHz;
        this.u = u;
        this.db = db;
        double h = 2 * SPAN / STEPS;
        double prev = psd(-SPAN);
        cdf[0] = 0;
        for(int i = 1; i <= STEPS; i++) {
            double cur = psd(-SPAN + i * h);
            cdf[i] = cdf[i - 1] + (prev + cur) * h / 2;
            prev = cur;
        }
        double total = cdf[STEPS];
        for(int i = 0; i <= STEPS; i++) cdf[i] /= total;
    }

    /** The mask's own bandwidth in Hz (NaN for {@link #GENERIC}, which scales to whatever it is given). */
    public double nominalBandwidthHz() {
        return nominalBandwidthHz;
    }

    /** Relative power density at offset {@code un} (in bandwidths) from centre, linear. */
    public double psd(double un) {
        double a = Math.abs(un);
        if(a >= u[u.length - 1]) return Units.dbToLinear(db[db.length - 1]);
        int i = 0;
        while(u[i + 1] < a) i++;
        double t = (a - u[i]) / (u[i + 1] - u[i]);
        return Units.dbToLinear(db[i] + t * (db[i + 1] - db[i]));
    }

    private double cdfAt(double un) {
        if(un <= -SPAN) return 0;
        if(un >= SPAN) return 1;
        double x = (un + SPAN) / (2 * SPAN) * STEPS;
        int i = Math.min(STEPS - 1, (int) x);
        double t = x - i;
        return cdf[i] + t * (cdf[i + 1] - cdf[i]);
    }

    /**
     * Fraction (0..1) of an interferer's power, centred at {@code txCentreHz} with
     * occupied bandwidth {@code txBandwidthHz}, that lands in a receiver at
     * {@code rxCentreHz} with bandwidth {@code rxBandwidthHz}.
     */
    public double weight(double txCentreHz, double txBandwidthHz, double rxCentreHz, double rxBandwidthHz) {
        if(!(txBandwidthHz > 0) || !(rxBandwidthHz > 0)) return 0;
        double lo = (rxCentreHz - rxBandwidthHz / 2 - txCentreHz) / txBandwidthHz;
        double hi = (rxCentreHz + rxBandwidthHz / 2 - txCentreHz) / txBandwidthHz;
        return Math.max(0, cdfAt(hi) - cdfAt(lo));
    }

    /** Same-mask, same-bandwidth channels (e.g. two 802.11b channels). */
    public double weight(double txCentreHz, double rxCentreHz, double bandwidthHz) {
        return weight(txCentreHz, bandwidthHz, rxCentreHz, bandwidthHz);
    }

    /** {@link #weight} in dB (&le; 0; &minus;infinity when nothing overlaps). */
    public double weightDb(double txCentreHz, double txBandwidthHz, double rxCentreHz, double rxBandwidthHz) {
        return Units.linearToDb(weight(txCentreHz, txBandwidthHz, rxCentreHz, rxBandwidthHz));
    }
}
