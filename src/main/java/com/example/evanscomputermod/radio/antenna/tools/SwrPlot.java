package com.example.evanscomputermod.radio.antenna.tools;

import java.util.ArrayList;
import java.util.List;

/**
 * Geometry of an SWR-versus-frequency plot (the analyzer screen): SWR on a
 * log axis from 1:1 at the bottom to {@link #MAX_SWR}:1 at the top, frequency
 * linear left to right, "nice" frequency ticks and the best point. Pure, so
 * the client screen only draws what this computes.
 */
public record SwrPlot(double[] hz, double[] swr) {
    /** Top of the SWR axis; worse readings (and +∞ outside the analysed bands) pin to the top. */
    public static final double MAX_SWR = 10;
    /** SWR grid lines, bottom to top. */
    public static final double[] GRID = {1.5, 2, 3, 5};

    public SwrPlot {
        if (hz.length != swr.length) throw new IllegalArgumentException("hz and swr differ in length");
    }

    /** Height fraction (0 = bottom = 1:1, 1 = top) of an SWR reading. */
    public static double yFrac(double swr) {
        if (!(swr > 1)) return Double.isNaN(swr) ? 1 : 0;
        if (!Double.isFinite(swr) || swr >= MAX_SWR) return 1;
        return Math.log(swr) / Math.log(MAX_SWR);
    }

    /** Width fraction of point {@code i}. */
    public double xFrac(int i) {
        return hz.length < 2 ? 0.5 : (double) i / (hz.length - 1);
    }

    /** Width fraction of a frequency (may fall outside 0..1). */
    public double xFracOf(double f) {
        if (hz.length < 2) return 0.5;
        return (f - hz[0]) / (hz[hz.length - 1] - hz[0]);
    }

    /** Index of the lowest SWR, or -1 if every point is unusable (∞/NaN). */
    public int minIndex() {
        int best = -1;
        for (int i = 0; i < swr.length; i++) {
            if (!Double.isFinite(swr[i])) continue;
            if (best < 0 || swr[i] < swr[best]) best = i;
        }
        return best;
    }

    /** Pixel row in a plot area of height {@code h} (top = 0) for an SWR. */
    public static int row(double swr, int top, int h) {
        return top + (int) Math.round((1 - yFrac(swr)) * (h - 1));
    }

    /** Pixel column in a plot area for point {@code i}. */
    public int col(int i, int left, int w) {
        return left + (int) Math.round(xFrac(i) * (w - 1));
    }

    /** Up to {@code max} round frequencies (1/2/5 × 10^n Hz steps) inside [lo, hi]. */
    public static List<Double> ticks(double lo, double hi, int max) {
        List<Double> out = new ArrayList<>();
        if (!(hi > lo) || max < 2) return out;
        double raw = (hi - lo) / (max - 1);
        double mag = Math.pow(10, Math.floor(Math.log10(raw)));
        double step = mag;
        for (double m : new double[] {1, 2, 5, 10}) {
            step = m * mag;
            if (step >= raw) break;
        }
        for (double f = Math.ceil(lo / step - 1e-9) * step; f <= hi + step * 1e-9; f += step) out.add(f);
        return out;
    }

    /** Frequency label: "7.15" (MHz above 1 MHz, kHz below), trailing zeros trimmed. */
    public static String label(double f) {
        double v = f >= 1e6 ? f / 1e6 : f / 1e3;
        String s = String.format(java.util.Locale.ROOT, "%.3f", v);
        s = s.replaceAll("0+$", "");
        if (s.endsWith(".")) s = s.substring(0, s.length() - 1);
        return s;
    }

    /** Unit for {@link #label}: "MHz" or "kHz". */
    public static String unit(double f) {
        return f >= 1e6 ? "MHz" : "kHz";
    }
}
