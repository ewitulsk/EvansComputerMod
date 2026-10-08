package com.example.evanscomputermod.radio.antenna.graph;

import com.example.evanscomputermod.radio.antenna.solver.AntennaResult;
import com.example.evanscomputermod.radio.antenna.solver.Complex;
import com.example.evanscomputermod.radio.antenna.solver.GainPattern;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Band;

import java.util.List;
import java.util.Locale;

/**
 * Everything known about one antenna, immutable and thread-safe. Produced by
 * {@link AntennaAnalysis} (solved by the MoM solver, or estimated by the
 * heuristic classifier while a solve is pending / above the segment cap).
 *
 * @param resonantHz        lowest series resonance, NaN if none in the searched range
 * @param feedImpedance     feed impedance at {@code resonantHz} (or at the best-SWR frequency)
 * @param analysisHz        the frequency the single-point values (Z, SWR, η, gain, limits) belong to
 * @param swrBandLowHz      2:1 SWR band (against 50 Ω) around the analysis frequency, NaN if none
 * @param wireLimitW        power at which the hottest conductor reaches its current rating
 * @param voltageLimitW     power at which the weakest insulator (or a bare end's corona limit, or the
 *                          feed point) reaches its voltage rating; NaN when not computed (estimates)
 * @param pattern           solved gain pattern at the analysis frequency, null for estimates
 * @param sweeps            per-band sweep tables for {@link #swrAt} and {@link #efficiencyAt}
 */
public record AntennaReport(Status status, String message, String kind, double resonantHz, double analysisHz,
        Complex feedImpedance, double efficiency, double peakGainDbi, double swrBandLowHz, double swrBandHighHz,
        double wireLimitW, String wireLimitLabel, double voltageLimitW, String voltageLimitLabel,
        double peakCurrentPerWatt, double peakEndVoltagePerWatt, GainPattern pattern, List<BandSweep> sweeps,
        int segments, double wireLengthM, String groundName, boolean fineWire) {

    public enum Status {
        /** No conductor attached to the feed point. */
        NO_ANTENNA,
        /** Heuristic classifier values (solve pending, or too large for the solver). */
        ESTIMATE,
        /** Method-of-Moments solution. */
        SOLVED,
        /** The geometry can't be analysed (message says why). */
        INVALID
    }

    public static final double Z0 = AntennaResult.DEFAULT_Z0;

    public AntennaReport {
        sweeps = sweeps == null ? List.of() : List.copyOf(sweeps);
    }

    public static AntennaReport none(String message) {
        return new AntennaReport(Status.NO_ANTENNA, message, "none", Double.NaN, Double.NaN, null, 0, Double.NaN,
                Double.NaN, Double.NaN, 0, "", 0, "", 0, 0, null, List.of(), 0, 0, "", false);
    }

    public static AntennaReport invalid(String message, double wireLength, String ground) {
        return new AntennaReport(Status.INVALID, message, "invalid", Double.NaN, Double.NaN, null, 0, Double.NaN,
                Double.NaN, Double.NaN, 0, "", 0, "", 0, 0, null, List.of(), 0, wireLength, ground, false);
    }

    /** One band's sweep: frequencies with feed impedance and efficiency. */
    public record BandSweep(Band band, double[] hz, double[] r, double[] x, double[] efficiency) {
        public boolean covers(double f) { return hz.length > 1 && f >= hz[0] && f <= hz[hz.length - 1]; }

        double[] interp(double f) {
            int i = 1;
            while (i < hz.length - 1 && hz[i] < f) i++;
            double t = (f - hz[i - 1]) / (hz[i] - hz[i - 1]);
            return new double[] {r[i - 1] + t * (r[i] - r[i - 1]), x[i - 1] + t * (x[i] - x[i - 1]),
                    efficiency[i - 1] + t * (efficiency[i] - efficiency[i - 1])};
        }
    }

    public boolean present() { return status == Status.SOLVED || status == Status.ESTIMATE; }

    public boolean solved() { return status == Status.SOLVED; }

    public double swr() { return feedImpedance == null ? Double.POSITIVE_INFINITY : AntennaResult.swr(feedImpedance, Complex.real(Z0)); }

    /** Power the antenna can take continuously: the lower of the wire and voltage limits, W. */
    public double powerLimitW() {
        if (!present()) return 0;
        double v = Double.isNaN(voltageLimitW) ? Double.POSITIVE_INFINITY : voltageLimitW;
        return Math.min(wireLimitW, v);
    }

    /** What sets {@link #powerLimitW}: the conductor or insulator label. */
    public String weakestLink() {
        if (!present()) return "";
        return !Double.isNaN(voltageLimitW) && voltageLimitW < wireLimitW ? voltageLimitLabel : wireLimitLabel;
    }

    /** The sweep table entry nearest {@code f}: {R, X, η}, or null outside every sweep. Densest table wins. */
    private double[] tableAt(double f) {
        BandSweep best = null;
        for (BandSweep s : sweeps) {
            if (!s.covers(f)) continue;
            double step = (s.hz[s.hz.length - 1] - s.hz[0]) / (s.hz.length - 1);
            if (best == null || step < (best.hz[best.hz.length - 1] - best.hz[0]) / (best.hz.length - 1)) best = s;
        }
        return best == null ? null : best.interp(f);
    }

    /** Feed impedance at {@code f} (interpolated), or null outside the swept bands. */
    public Complex impedanceAt(double f) {
        double[] t = tableAt(f);
        return t == null ? null : Complex.of(t[0], t[1]);
    }

    /** SWR against {@code z0} at {@code f}; +∞ outside the swept bands (treat as unusable). */
    public double swrAt(double f, double z0) {
        Complex z = impedanceAt(f);
        return z == null ? Double.POSITIVE_INFINITY : AntennaResult.swr(z, Complex.real(z0));
    }

    public double swrAt(double f) { return swrAt(f, Z0); }

    /** Radiation efficiency at {@code f}, 0 outside the swept bands. */
    public double efficiencyAt(double f) {
        double[] t = tableAt(f);
        return t == null ? 0 : Math.max(0, Math.min(1, t[2]));
    }

    /** Mismatch loss into the antenna at {@code f} against z0, dB (∞ outside the swept bands). */
    public double mismatchLossDb(double f, double z0) {
        double s = swrAt(f, z0);
        if (!Double.isFinite(s)) return Double.POSITIVE_INFINITY;
        double g = (s - 1) / (s + 1);
        return -10 * Math.log10(Math.max(1e-12, 1 - g * g));
    }

    /**
     * The radiation pattern for use at {@code f} by a radio endpoint (local
     * frame = the Minecraft frame of the antenna's structure). Uses the
     * pattern solved at the analysis frequency, scaled by the efficiency at
     * {@code f}; {@code feedLossDb} carries the mismatch loss at {@code f}.
     * Estimates and missing antennas fall back to a dipole / -30 dBi isotrope.
     */
    public AntennaPattern patternAt(double f) {
        if (!present()) return new Scaled(AntennaPattern.ISOTROPIC, -30, 0);
        double effHere = efficiencyAt(f), offset = 0;
        if (efficiency > 0 && effHere > 0) offset = 10 * Math.log10(effHere / efficiency);
        else if (effHere <= 0) offset = -30;
        double mismatch = Math.min(30, mismatchLossDb(f, Z0));
        if (pattern != null) return new SolvedPattern(pattern, offset, mismatch);
        return new Scaled(AntennaPattern.VERTICAL_DIPOLE, peakGainDbi - 2.15 + offset, mismatch);
    }

    private record Scaled(AntennaPattern base, double offset, double feedLoss) implements AntennaPattern {
        @Override public double gainDbi(double lx, double ly, double lz) { return base.gainDbi(lx, ly, lz) + offset; }
        @Override public double[] polarization(double lx, double ly, double lz) { return base.polarization(lx, ly, lz); }
        @Override public double peakGainDbi() { return base.peakGainDbi() + offset; }
        @Override public double feedLossDb() { return feedLoss; }
    }

    // ------------------------------------------------------------------ text

    /**
     * One line, e.g. "Resonant at 7.1 MHz · 2:1 SWR band 6.9–7.3 MHz · rated
     * 52 W (copper wire) / 4.1 kW (insulators)".
     */
    public String summary() {
        return switch (status) {
            case NO_ANTENNA -> "No antenna: " + message;
            case INVALID -> "Antenna can't be analysed: " + message;
            case ESTIMATE, SOLVED -> {
                String approx = status == Status.ESTIMATE ? "≈ " : "";
                StringBuilder sb = new StringBuilder();
                if (Double.isFinite(resonantHz)) sb.append(approx).append("Resonant at ").append(hz(resonantHz));
                else sb.append(approx).append("No resonance found · best SWR ").append(ratio(swr())).append(" at ").append(hz(analysisHz));
                if (Double.isFinite(swrBandLowHz)) sb.append(" · 2:1 SWR band ").append(band(swrBandLowHz, swrBandHighHz));
                else if (Double.isFinite(resonantHz)) sb.append(" · SWR ").append(ratio(swr())).append(" (no 2:1 band)");
                sb.append(" · rated ").append(watts(wireLimitW)).append(" (").append(wireLimitLabel).append(")");
                if (!Double.isNaN(voltageLimitW)) sb.append(" / ").append(watts(voltageLimitW)).append(" (").append(voltageLimitLabel).append(")");
                if (status == Status.ESTIMATE) sb.append(" (estimate").append(message.isEmpty() ? "" : ": " + message).append(")");
                yield sb.toString();
            }
        };
    }

    /** A second line with the numbers behind the summary. */
    public String details() {
        if (!present()) return "";
        StringBuilder sb = new StringBuilder();
        sb.append(kind).append(" · Z ").append(String.format(Locale.ROOT, "%.1f %s j%.1f Ω", feedImpedance.re(),
                feedImpedance.im() < 0 ? "-" : "+", Math.abs(feedImpedance.im())));
        sb.append(" · SWR ").append(ratio(swr()));
        sb.append(" · efficiency ").append(String.format(Locale.ROOT, "%.0f%%", efficiency * 100));
        if (Double.isFinite(peakGainDbi)) sb.append(" · gain ").append(String.format(Locale.ROOT, "%.1f dBi", peakGainDbi));
        sb.append(" · ").append(String.format(Locale.ROOT, "%.1f m of wire", wireLengthM));
        if (segments > 0) sb.append(" · ").append(segments).append(" segments");
        sb.append(" · ground: ").append(groundName);
        sb.append(" · limited to ").append(watts(powerLimitW())).append(" by ").append(weakestLink());
        return sb.toString();
    }

    public static String hz(double f) {
        if (!Double.isFinite(f)) return "?";
        if (f >= 1e9) return String.format(Locale.ROOT, "%.2f GHz", f / 1e9);
        if (f >= 100e6) return String.format(Locale.ROOT, "%.0f MHz", f / 1e6);
        if (f >= 1e6) return String.format(Locale.ROOT, "%.1f MHz", f / 1e6);
        return String.format(Locale.ROOT, "%.0f kHz", f / 1e3);
    }

    static String band(double lo, double hi) {
        String a = hz(lo), b = hz(hi);
        int sp = a.lastIndexOf(' ');
        if (sp > 0 && a.substring(sp).equals(b.substring(b.lastIndexOf(' ')))) a = a.substring(0, sp);
        return a + "–" + b;
    }

    public static String watts(double w) {
        if (!Double.isFinite(w)) return "∞ W";
        if (w >= 1e6) return String.format(Locale.ROOT, "%.1f MW", w / 1e6);
        if (w >= 1e3) return String.format(Locale.ROOT, w >= 1e4 ? "%.0f kW" : "%.1f kW", w / 1e3);
        return String.format(Locale.ROOT, w >= 10 ? "%.0f W" : "%.1f W", w);
    }

    static String ratio(double swr) {
        if (!Double.isFinite(swr) || swr > 99) return ">99:1";
        return String.format(Locale.ROOT, "%.1f:1", swr);
    }
}
