package com.example.evanscomputermod.radio.antenna.solver;

import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.OptionalDouble;

/**
 * Frequency sweeps over one mesh. The mesh is built once at the top of the
 * range so every point uses the same segmentation (no jumps in the curve).
 * Sweep points skip the 5° pattern unless asked; impedance, SWR, efficiency,
 * currents and end voltages are always present.
 */
public final class FrequencySweep {
    private static final int REFINE_STEPS = 30;

    private FrequencySweep() {}

    /** A frequency band in Hz. */
    public record Band(double lowHz, double highHz) {
        public double widthHz() { return highHz - lowHz; }

        public double centerHz() { return (lowHz + highHz) / 2; }
    }

    public static List<AntennaResult> sweep(AntennaModel model, double fStartHz, double fStopHz, int points) {
        return sweep(model, fStartHz, fStopHz, points, false);
    }

    public static List<AntennaResult> sweep(AntennaModel model, double fStartHz, double fStopHz, int points, boolean patterns) {
        checkRange(fStartHz, fStopHz, points);
        AntennaMesh mesh = AntennaMesh.build(model, fStopHz);
        List<AntennaResult> out = new ArrayList<>(points);
        for (int i = 0; i < points; i++) out.add(AntennaSolver.solve(mesh, frequency(fStartHz, fStopHz, points, i), patterns));
        return out;
    }

    /**
     * The lowest series resonance (reactance crossing zero from capacitive to inductive) in the range,
     * refined by bisection to well under the sweep spacing.
     */
    public static OptionalDouble resonantFrequency(AntennaModel model, double fStartHz, double fStopHz, int points) {
        checkRange(fStartHz, fStopHz, points);
        AntennaMesh mesh = AntennaMesh.build(model, fStopHz);
        double prevF = fStartHz, prevX = reactance(mesh, fStartHz);
        for (int i = 1; i < points; i++) {
            double f = frequency(fStartHz, fStopHz, points, i), x = reactance(mesh, f);
            if (prevX < 0 && x >= 0) {
                double lo = prevF, hi = f;
                for (int it = 0; it < REFINE_STEPS && hi - lo > lo * 1e-7; it++) {
                    double mid = (lo + hi) / 2;
                    if (reactance(mesh, mid) < 0) lo = mid; else hi = mid;
                }
                return OptionalDouble.of((lo + hi) / 2);
            }
            prevF = f;
            prevX = x;
        }
        return OptionalDouble.empty();
    }

    public static Optional<Band> swr2to1Bandwidth(AntennaModel model, double fStartHz, double fStopHz, int points) {
        return swrBandwidth(model, fStartHz, fStopHz, points, AntennaResult.DEFAULT_Z0, 2);
    }

    /**
     * The contiguous band around the best-matched sweep point where SWR (against z0) stays at or below
     * {@code maxSwr}, with edges refined by bisection. Empty if no point reaches it. An edge that runs past
     * the sweep range is clipped to the range.
     */
    public static Optional<Band> swrBandwidth(AntennaModel model, double fStartHz, double fStopHz, int points, double z0, double maxSwr) {
        checkRange(fStartHz, fStopHz, points);
        AntennaMesh mesh = AntennaMesh.build(model, fStopHz);
        double[] swr = new double[points];
        int best = 0;
        for (int i = 0; i < points; i++) {
            swr[i] = swr(mesh, frequency(fStartHz, fStopHz, points, i), z0);
            if (swr[i] < swr[best]) best = i;
        }
        if (!(swr[best] <= maxSwr)) return Optional.empty();
        int lo = best, hi = best;
        while (lo > 0 && swr[lo - 1] <= maxSwr) lo--;
        while (hi < points - 1 && swr[hi + 1] <= maxSwr) hi++;
        double low = lo == 0 ? fStartHz : edge(mesh, frequency(fStartHz, fStopHz, points, lo - 1), frequency(fStartHz, fStopHz, points, lo), z0, maxSwr);
        double high = hi == points - 1 ? fStopHz : edge(mesh, frequency(fStartHz, fStopHz, points, hi + 1), frequency(fStartHz, fStopHz, points, hi), z0, maxSwr);
        return Optional.of(new Band(low, high));
    }

    /** Bisects between a frequency outside the band and one inside it. */
    private static double edge(AntennaMesh mesh, double outside, double inside, double z0, double maxSwr) {
        for (int it = 0; it < REFINE_STEPS && Math.abs(inside - outside) > inside * 1e-7; it++) {
            double mid = (outside + inside) / 2;
            if (swr(mesh, mid, z0) <= maxSwr) inside = mid; else outside = mid;
        }
        return (outside + inside) / 2;
    }

    private static double reactance(AntennaMesh mesh, double f) {
        return AntennaSolver.solve(mesh, f, false).feedImpedance().im();
    }

    private static double swr(AntennaMesh mesh, double f, double z0) {
        return AntennaSolver.solve(mesh, f, false).swr(z0);
    }

    private static double frequency(double start, double stop, int points, int i) {
        return points == 1 ? start : start + (stop - start) * i / (points - 1);
    }

    private static void checkRange(double start, double stop, int points) {
        if (!(start > 0) || !(stop >= start) || !Double.isFinite(stop) || points < 1 || (points == 1 && stop != start))
            throw new IllegalArgumentException("bad sweep range " + start + ".." + stop + " x" + points);
    }
}
