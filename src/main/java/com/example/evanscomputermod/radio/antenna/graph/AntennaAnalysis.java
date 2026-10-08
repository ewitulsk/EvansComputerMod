package com.example.evanscomputermod.radio.antenna.graph;

import com.example.evanscomputermod.radio.antenna.solver.AntennaGeometryException;
import com.example.evanscomputermod.radio.antenna.solver.AntennaMesh;
import com.example.evanscomputermod.radio.antenna.solver.AntennaModel;
import com.example.evanscomputermod.radio.antenna.solver.AntennaResult;
import com.example.evanscomputermod.radio.antenna.solver.AntennaSolver;
import com.example.evanscomputermod.radio.antenna.solver.Complex;
import com.example.evanscomputermod.radio.antenna.solver.FrequencySweep;
import com.example.evanscomputermod.radio.antenna.solver.HeuristicAntenna;
import com.example.evanscomputermod.radio.api.Band;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalDouble;

/**
 * Analyses an {@link AntennaGraph}: the solver integration of the spec's
 * "Antenna solver" section. Pure and thread-safe; the game runs
 * {@link #solve} on a worker thread and shows {@link #estimate} meanwhile.
 *
 * <h2>What a solve does</h2>
 * <ol>
 *   <li>Searches for the lowest series resonance between 0.45× and 1.25× the
 *       half-wave frequency of the graph's tip-to-tip length (monopoles count
 *       their image).</li>
 *   <li>Solves at resonance (or at the best-SWR frequency if there is none)
 *       with the 5° gain pattern: impedance, efficiency, gain, segment
 *       currents and end voltage.</li>
 *   <li>Finds the 2:1 SWR band and sweeps every {@link Band} the antenna could
 *       serve (from 0.45× resonance up to 3.2× resonance, capped at the
 *       construction's accuracy limit: 30 MHz for block wires, 500 MHz for
 *       fine wire), so link budgets can read SWR and efficiency at any
 *       frequency without solving.</li>
 *   <li>Power limits: per segment, the RMS current at P watts is
 *       I<sub>pk/W</sub>·√P/√2; the conductor's current rating gives
 *       P = 2·I<sub>rating</sub>²/I<sub>pk/W</sub>². Voltage: the open-end
 *       voltage per √W against each insulator touching the antenna (and the
 *       corona limit of open ends no insulator holds), and the feed voltage
 *       |Z|·√(2/R) against the feed point's rating. The lowest is named.</li>
 * </ol>
 */
public final class AntennaAnalysis {
    private AntennaAnalysis() {}

    public static final double C0 = AntennaSolver.C0;
    /** Above this many segments (or while pending) the heuristic answers. */
    public static final int SEGMENT_CAP = AntennaMesh.MAX_SEGMENTS;
    public static final double BLOCK_WIRE_ACCURACY_HZ = 30e6;
    public static final double FINE_WIRE_ACCURACY_HZ = 500e6;
    private static final int SEARCH_POINTS = 28;
    private static final int BAND_POINTS = 48;
    private static final int LOCAL_POINTS = 25;

    /** The guess the search and the heuristic start from: a half wave on the tip-to-tip length. */
    public static double guessHz(AntennaGraph g) {
        return 0.96 * C0 / (2 * Math.max(0.05, g.pathLength()));
    }

    /** Quick heuristic report (no MoM), for use while the solve is pending. */
    public static AntennaReport estimate(AntennaGraph g) {
        return estimate(g, "solving");
    }

    static AntennaReport estimate(AntennaGraph g, String why) {
        if (g.empty()) return AntennaReport.none(noAntennaMessage());
        AntennaModelBuilder.Built built;
        try {
            built = AntennaModelBuilder.build(g);
        } catch (AntennaGeometryException e) {
            return AntennaReport.invalid(e.getMessage(), g.totalLength(), g.groundName);
        }
        double f = guessHz(g);
        HeuristicAntenna.Estimate est;
        try {
            est = HeuristicAntenna.estimate(built.model(), f);
        } catch (AntennaGeometryException e) {
            return AntennaReport.invalid(e.getMessage(), g.totalLength(), g.groundName);
        }
        double r = Math.max(1, est.feedImpedance().re());
        double ipw = Math.sqrt(2 / r);
        double wireLimit = Double.POSITIVE_INFINITY;
        String wireLabel = "";
        for (int i = 0; i < built.specs().size(); i++) {
            double p = 2 * sq(built.specs().get(i).currentRatingA()) / (ipw * ipw);
            if (p < wireLimit) { wireLimit = p; wireLabel = built.specs().get(i).name(); }
        }
        boolean resonantShape = est.type() != HeuristicAntenna.Type.LONG_WIRE && est.type() != HeuristicAntenna.Type.UNKNOWN;
        return new AntennaReport(AntennaReport.Status.ESTIMATE, why, kindName(est.type()), resonantShape ? f : Double.NaN, f,
                est.feedImpedance(), est.efficiency(), est.gainDbi(), Double.NaN, Double.NaN, wireLimit, wireLabel,
                Double.NaN, "", ipw, 0, null, List.of(), 0, g.totalLength(), g.groundName, g.hasFineWire());
    }

    static String noAntennaMessage() {
        return "nothing conductive on the feed point's arms";
    }

    /** Full Method-of-Moments analysis. Never throws: bad geometry gives an INVALID report, too big an ESTIMATE. */
    public static AntennaReport solve(AntennaGraph g) {
        if (g.empty()) return AntennaReport.none(noAntennaMessage());
        AntennaModelBuilder.Built built;
        try {
            built = AntennaModelBuilder.build(g);
        } catch (AntennaGeometryException e) {
            return AntennaReport.invalid(e.getMessage(), g.totalLength(), g.groundName);
        }
        AntennaModel model = built.model();
        double guess = guessHz(g);
        double fLo = 0.45 * guess / 0.96, fHi = 1.25 * guess / 0.96;
        double accuracy = g.hasFineWire() && onlyFineWire(g) ? FINE_WIRE_ACCURACY_HZ : BLOCK_WIRE_ACCURACY_HZ;
        try {
            OptionalDouble res = FrequencySweep.resonantFrequency(model, fLo, fHi, SEARCH_POINTS);
            double fa;
            String kind;
            if (res.isPresent()) {
                fa = res.getAsDouble();
            } else {
                // No series resonance: analyse at the best-matched point of the search range.
                List<AntennaResult> pts = FrequencySweep.sweep(model, fLo, fHi, SEARCH_POINTS);
                AntennaResult best = pts.get(0);
                for (AntennaResult p : pts) if (p.swr() < best.swr()) best = p;
                fa = best.frequencyHz();
            }
            kind = kindName(HeuristicAntenna.estimate(model, fa).type());
            AntennaMesh mesh = AntennaMesh.build(model, fa);
            AntennaResult r = AntennaSolver.solve(mesh, fa, true);

            // Local sweep and 2:1 band around the analysis frequency.
            List<AntennaReport.BandSweep> sweeps = new ArrayList<>();
            sweeps.add(table(null, FrequencySweep.sweep(model, fa * 0.8, fa * 1.2, LOCAL_POINTS)));
            Optional<FrequencySweep.Band> band = FrequencySweep.swrBandwidth(model, fa * 0.8, fa * 1.2, LOCAL_POINTS,
                    AntennaReport.Z0, 2);
            // Per-band sweeps.
            double wLo = Math.min(fLo, fa * 0.45), wHi = Math.max(Math.min(accuracy, fa * 3.2), fa * 1.25);
            for (Band b : Band.values()) {
                double lo = Math.max(b.minHz, wLo), hi = Math.min(b.maxHz, wHi);
                if (hi <= lo * 1.01) continue;
                try {
                    sweeps.add(table(b, FrequencySweep.sweep(model, lo, hi, BAND_POINTS)));
                } catch (AntennaMesh.SegmentCapException e) {
                    // The band's top is too fine for the cap: sweep the part that fits.
                    double top = hi;
                    while (top > lo * 1.05) {
                        top = lo + (top - lo) * 0.6;
                        try {
                            sweeps.add(table(b, FrequencySweep.sweep(model, lo, top, BAND_POINTS)));
                            break;
                        } catch (AntennaMesh.SegmentCapException again) {
                            // keep shrinking
                        }
                    }
                }
            }

            // Power limits.
            double wireLimit = Double.POSITIVE_INFINITY;
            String wireLabel = "";
            double[] seg = r.segmentCurrentPerWatt();
            for (int i = 0; i < seg.length; i++) {
                if (!(seg[i] > 0)) continue;
                ConductorSpec spec = built.specs().get(mesh.segmentWire(i));
                double p = 2 * sq(spec.currentRatingA()) / (seg[i] * seg[i]);
                if (p < wireLimit) { wireLimit = p; wireLabel = built.labels().get(mesh.segmentWire(i)); }
            }
            VoltageLimit v = voltageLimit(g, r);
            return new AntennaReport(AntennaReport.Status.SOLVED, "", kind, res.isPresent() ? fa : Double.NaN, fa,
                    r.feedImpedance(), r.efficiency(), r.peakGainDbi(),
                    band.map(FrequencySweep.Band::lowHz).orElse(Double.NaN), band.map(FrequencySweep.Band::highHz).orElse(Double.NaN),
                    wireLimit, shortLabel(wireLabel), v.watts, v.label, r.peakCurrentPerWatt(), r.peakEndVoltagePerWatt(),
                    r.pattern(), sweeps, mesh.segmentCount(), g.totalLength(), g.groundName, g.hasFineWire());
        } catch (AntennaMesh.SegmentCapException e) {
            return estimate(g, "too large for the solver (over " + SEGMENT_CAP + " segments)");
        } catch (AntennaGeometryException e) {
            return AntennaReport.invalid(e.getMessage(), g.totalLength(), g.groundName);
        }
    }

    private static boolean onlyFineWire(AntennaGraph g) {
        for (AntennaGraph.Edge e : g.edges) if (!e.fineWire()) return false;
        return true;
    }

    /** Labels name a conductor kind and place ("copper wire at (1, 2, 3)"); the summary shows the kind. */
    private static String shortLabel(String label) {
        int at = label.indexOf(" at (");
        return at > 0 ? label.substring(0, at) : label;
    }

    private record VoltageLimit(double watts, String label) {}

    private static VoltageLimit voltageLimit(AntennaGraph g, AntennaResult r) {
        double best = Double.POSITIVE_INFINITY;
        String label = "";
        double vEnd = r.peakEndVoltagePerWatt();
        if (vEnd > 0) {
            for (AntennaGraph.Insulated ins : g.insulatedPoints) {
                double p = sq(ins.voltageRating() / vEnd);
                if (p < best) { best = p; label = "insulators"; }
            }
            for (OpenEnd end : openEnds(g)) {
                if (end.insulated) continue;
                double p = sq(end.spec.coronaVoltage() / vEnd);
                if (p < best) { best = p; label = "bare end of " + end.spec.name(); }
            }
        }
        Complex z = r.feedImpedance();
        if (z.re() > 0) {
            double vFeed = z.abs() * Math.sqrt(2 / z.re());
            double p = sq(g.feedVoltageRating / vFeed);
            if (p < best) { best = p; label = "feed point"; }
        }
        return new VoltageLimit(best, label);
    }

    record OpenEnd(AntennaGraph.Point at, ConductorSpec spec, boolean insulated) {}

    /** Wire ends joined to nothing else (excluding the feed and, for monopoles, the ground). */
    static List<OpenEnd> openEnds(AntennaGraph g) {
        Map<Long, List<Object[]>> nodes = new HashMap<>();
        List<Object[]> all = new ArrayList<>();
        for (AntennaGraph.Edge e : g.edges) {
            count(nodes, all, e.a(), e.spec());
            count(nodes, all, e.b(), e.spec());
        }
        count(nodes, all, g.feedA, g.feedSpec);
        count(nodes, all, g.feedB, g.feedSpec);
        List<OpenEnd> out = new ArrayList<>();
        for (Object[] n : all) {
            AntennaGraph.Point p = (AntennaGraph.Point) n[0];
            if ((int) n[2] != 1 || p.near(g.feedA) || p.near(g.feedB)) continue;
            boolean insulated = false;
            for (AntennaGraph.Insulated ins : g.insulatedPoints) if (ins.at().near(p)) insulated = true;
            out.add(new OpenEnd(p, (ConductorSpec) n[1], insulated));
        }
        return out;
    }

    private static void count(Map<Long, List<Object[]>> nodes, List<Object[]> all, AntennaGraph.Point p, ConductorSpec spec) {
        List<Object[]> bucket = nodes.computeIfAbsent(p.key(), k -> new ArrayList<>());
        for (Object[] n : bucket) {
            if (((AntennaGraph.Point) n[0]).near(p)) { n[2] = (int) n[2] + 1; return; }
        }
        Object[] n = {p, spec, 1};
        bucket.add(n);
        all.add(n);
    }

    private static AntennaReport.BandSweep table(Band band, List<AntennaResult> pts) {
        int n = pts.size();
        double[] f = new double[n], re = new double[n], im = new double[n], eff = new double[n];
        for (int i = 0; i < n; i++) {
            AntennaResult p = pts.get(i);
            f[i] = p.frequencyHz();
            re[i] = p.feedImpedance().re();
            im[i] = p.feedImpedance().im();
            eff[i] = p.efficiency();
        }
        return new AntennaReport.BandSweep(band, f, re, im, eff);
    }

    static String kindName(HeuristicAntenna.Type t) {
        return switch (t) {
            case DIPOLE -> "dipole";
            case MONOPOLE -> "monopole";
            case LOOP -> "loop";
            case LONG_WIRE -> "long wire";
            case UNKNOWN -> "wire antenna";
        };
    }

    /** Power at which a conductor carrying {@code peakAmpsPerRootWatt} reaches its RMS rating. */
    public static double currentLimitW(double ratingRmsA, double peakAmpsPerRootWatt) {
        return 2 * sq(ratingRmsA) / sq(peakAmpsPerRootWatt);
    }

    /** Power at which a {@code voltsPerRootWatt} point reaches an insulator's peak rating. */
    public static double voltageLimitW(double ratingPeakV, double voltsPerRootWatt) {
        return sq(ratingPeakV / voltsPerRootWatt);
    }

    private static double sq(double x) { return x * x; }
}
