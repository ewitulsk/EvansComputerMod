package com.example.evanscomputermod.radio.antenna.tools;

import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.antenna.solver.AntennaResult;
import com.example.evanscomputermod.radio.antenna.solver.Complex;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * The numbers behind the antenna tools (the {@code antenna} peripheral, the
 * {@code antenna} program and the analyzer screen), shaped from a cached
 * {@link AntennaReport} without solving anything. Pure: no world access, so
 * it is unit-tested directly.
 *
 * <p><b>Directions.</b> Azimuth is a compass bearing in degrees, clockwise
 * from north (-Z) through east (+X); elevation is degrees above the horizon.
 * Pattern cuts sample the report's pattern in the antenna's local (structure)
 * frame.
 */
public final class AntennaToolsData {
    /** Most points one sweep returns. */
    public static final int MAX_POINTS = 401;
    /** Gains below this (nulls, the solver's below-ground floor) are reported as this, dBi. */
    public static final double GAIN_FLOOR_DBI = -99.9;

    private AntennaToolsData() {}

    // ------------------------------------------------------------ impedance / SWR

    /** {f, r, x, swr, efficiency, in_band}: feed impedance at {@code hz}; r/x NaN and swr ∞ outside the analysed bands. */
    public static Map<String, Object> impedance(AntennaReport r, double hz) {
        Map<String, Object> m = new LinkedHashMap<>();
        Complex z = r.present() ? r.impedanceAt(hz) : null;
        m.put("f", hz);
        m.put("r", z == null ? Double.NaN : z.re());
        m.put("x", z == null ? Double.NaN : z.im());
        m.put("swr", z == null ? Double.POSITIVE_INFINITY : AntennaResult.swr(z, Complex.real(AntennaReport.Z0)));
        m.put("efficiency", r.present() ? r.efficiencyAt(hz) : 0.0);
        m.put("in_band", z != null);
        return m;
    }

    /** Validates a sweep request; throws {@link IllegalArgumentException} with a user-facing message. */
    public static void checkSweep(double f0, double f1, int points) {
        if (!(f0 > 0) || !Double.isFinite(f1)) throw new IllegalArgumentException("frequencies must be positive, Hz");
        if (!(f1 > f0)) throw new IllegalArgumentException("stop frequency must be above the start");
        if (points < 2 || points > MAX_POINTS) throw new IllegalArgumentException("points must be 2.." + MAX_POINTS);
    }

    /** Evenly spaced frequencies from f0 to f1 inclusive. */
    public static double[] frequencies(double f0, double f1, int points) {
        double[] f = new double[points];
        for (int i = 0; i < points; i++) f[i] = points == 1 ? f0 : f0 + (f1 - f0) * i / (points - 1);
        return f;
    }

    /** One {f, swr, r, x} per point from f0 to f1 (inclusive), read from the cached sweep tables. */
    public static List<Map<String, Object>> sweep(AntennaReport r, double f0, double f1, int points) {
        checkSweep(f0, f1, points);
        List<Map<String, Object>> out = new ArrayList<>(points);
        for (double f : frequencies(f0, f1, points)) {
            Map<String, Object> z = impedance(r, f);
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("f", f);
            m.put("swr", z.get("swr"));
            m.put("r", z.get("r"));
            m.put("x", z.get("x"));
            out.add(m);
        }
        return out;
    }

    /** SWR (against 50 Ω) at each frequency; +∞ outside the analysed bands or with no antenna. */
    public static double[] swrCurve(AntennaReport r, double[] hz) {
        double[] s = new double[hz.length];
        for (int i = 0; i < hz.length; i++) s[i] = r.present() ? r.swrAt(hz[i]) : Double.POSITIVE_INFINITY;
        return s;
    }

    /**
     * The frequency span the analyzer screen plots: ±15 % around resonance
     * (or the best-SWR frequency), which the cached local sweep always covers.
     * Null with no antenna.
     */
    @Nullable
    public static double[] analyzerSpan(AntennaReport r) {
        if (!r.present()) return null;
        double fa = Double.isFinite(r.resonantHz()) ? r.resonantHz() : r.analysisHz();
        if (!(fa > 0)) return null;
        return new double[] {fa * 0.85, fa * 1.15};
    }

    // ------------------------------------------------------------ pattern

    /** Local unit direction for a compass azimuth and an elevation, degrees. */
    public static double[] direction(double azDeg, double elDeg) {
        double az = Math.toRadians(azDeg), el = Math.toRadians(elDeg);
        return new double[] {Math.sin(az) * Math.cos(el), Math.sin(el), -Math.cos(az) * Math.cos(el)};
    }

    static double gain(AntennaPattern p, double[] d) {
        double g = p.gainDbi(d[0], d[1], d[2]);
        return Double.isFinite(g) ? Math.max(GAIN_FLOOR_DBI, g) : GAIN_FLOOR_DBI;
    }

    /** {gain_dbi, az, el}: the strongest direction on a 5° grid (elevations -85..85). */
    public static Map<String, Object> peak(AntennaPattern p) {
        double best = Double.NEGATIVE_INFINITY, bestAz = 0, bestEl = 0;
        for (int el = -85; el <= 85; el += 5) {
            for (int az = 0; az < 360; az += 5) {
                double g = gain(p, direction(az, el));
                if (g > best + 1e-9) { best = g; bestAz = az; bestEl = el; }
            }
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("gain_dbi", best);
        m.put("az", bestAz);
        m.put("el", bestEl);
        return m;
    }

    /**
     * A pattern cut, one gain (dBi) per {@code stepDeg} starting at 0°.
     * {@code "azimuth"}: compass bearings 0..360 at elevation {@code angleDeg}.
     * {@code "elevation"}: the vertical plane through azimuth {@code angleDeg};
     * angle 0 is the horizon towards that azimuth, 90 the zenith, 180 the
     * opposite horizon and 270 the nadir.
     */
    public static List<Double> patternCut(AntennaPattern p, String plane, double stepDeg, double angleDeg) {
        boolean az = isAzimuth(plane);
        if (!(stepDeg >= 1 && stepDeg <= 90)) throw new IllegalArgumentException("step must be 1..90 degrees");
        int n = (int) Math.round(360 / stepDeg);
        List<Double> out = new ArrayList<>(n);
        for (int i = 0; i < n; i++) {
            double a = i * 360.0 / n;
            out.add(gain(p, az ? direction(a, angleDeg) : elevationDirection(angleDeg, a)));
        }
        return out;
    }

    /** True for "azimuth"/"az"/"h", false for "elevation"/"el"/"v"; anything else is an error. */
    public static boolean isAzimuth(String plane) {
        String s = plane == null ? "azimuth" : plane.toLowerCase(Locale.ROOT);
        return switch (s) {
            case "azimuth", "az", "h", "horizontal" -> true;
            case "elevation", "el", "v", "vertical" -> false;
            default -> throw new IllegalArgumentException("plane must be 'azimuth' or 'elevation'");
        };
    }

    /** Direction at {@code angle} round the vertical plane through azimuth {@code cutAz}. */
    public static double[] elevationDirection(double cutAzDeg, double angleDeg) {
        double[] h = direction(cutAzDeg, 0);
        double t = Math.toRadians(angleDeg), c = Math.cos(t), s = Math.sin(t);
        return new double[] {h[0] * c, s, h[2] * c};
    }

    /**
     * {az, el, x, y, z, tilt_deg, sense}: the E-field direction towards the
     * peak, its tilt above the horizontal plane and a name
     * ("horizontal" under 20°, "vertical" over 70°, else "slant").
     */
    public static Map<String, Object> polarization(AntennaPattern p) {
        Map<String, Object> pk = peak(p);
        double az = ((Number) pk.get("az")).doubleValue(), el = ((Number) pk.get("el")).doubleValue();
        double[] d = direction(az, el);
        double[] e = p.polarization(d[0], d[1], d[2]);
        double len = Math.sqrt(e[0] * e[0] + e[1] * e[1] + e[2] * e[2]);
        if (len > 0) e = new double[] {e[0] / len, e[1] / len, e[2] / len};
        double tilt = Math.toDegrees(Math.asin(Math.min(1, Math.abs(e[1]))));
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("az", az);
        m.put("el", el);
        m.put("x", e[0]);
        m.put("y", e[1]);
        m.put("z", e[2]);
        m.put("tilt_deg", tilt);
        m.put("sense", polarizationName(tilt));
        return m;
    }

    public static String polarizationName(double tiltDeg) {
        return tiltDeg < 20 ? "horizontal" : tiltDeg > 70 ? "vertical" : "slant";
    }

    // ------------------------------------------------------------ power limit

    /** What drives the antenna: a transmitter or amplifier found on the feedline (or named by the caller). */
    public record Transmitter(String name, double watts) {}

    /**
     * {watts, wire_watts, voltage_watts, cause, weakest:{x,y,z,block},
     * transmitter:{name,watts}, verdict, text}. {@code tx} may be null.
     */
    public static Map<String, Object> powerLimit(AntennaReport r, int x, int y, int z, String block, @Nullable Transmitter tx) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("watts", r.powerLimitW());
        m.put("wire_watts", r.present() ? r.wireLimitW() : 0.0);
        m.put("voltage_watts", r.present() ? r.voltageLimitW() : Double.NaN);
        m.put("cause", r.limitCause());
        Map<String, Object> w = new LinkedHashMap<>();
        w.put("x", x);
        w.put("y", y);
        w.put("z", z);
        w.put("block", block);
        w.put("part", r.weakestLink());
        m.put("weakest", w);
        if (tx != null) {
            Map<String, Object> t = new LinkedHashMap<>();
            t.put("name", tx.name());
            t.put("watts", tx.watts());
            m.put("transmitter", t);
            m.put("verdict", verdict(r, tx.watts()));
        }
        m.put("text", limitText(r, x, y, z, tx));
        return m;
    }

    /** "ok", "will arc" (insulator voltage) or "will overheat" (wire current) when driven with {@code watts}. */
    public static String verdict(AntennaReport r, double watts) {
        if (!r.present()) return "no antenna";
        if (watts <= r.powerLimitW()) return "ok";
        return "insulator_voltage".equals(r.limitCause()) ? "will arc" : "will overheat";
    }

    /**
     * The analyzer's limit line, e.g. "Limited to 640 W by insulators at
     * (12, 80, -4); wire rated 2.0 kW; amplifier 1kw 1.0 kW -> will arc".
     */
    public static String limitText(AntennaReport r, int x, int y, int z, @Nullable Transmitter tx) {
        if (!r.present()) return r.summary();
        StringBuilder sb = new StringBuilder("Limited to ").append(AntennaReport.watts(r.powerLimitW()))
                .append(" by ").append(r.weakestLink()).append(" at (").append(x).append(", ").append(y).append(", ").append(z).append(")");
        boolean voltage = "insulator_voltage".equals(r.limitCause());
        if (voltage) sb.append("; wire rated ").append(AntennaReport.watts(r.wireLimitW()));
        else if (!Double.isNaN(r.voltageLimitW())) sb.append("; insulation rated ").append(AntennaReport.watts(r.voltageLimitW()));
        if (r.status() == AntennaReport.Status.ESTIMATE) sb.append(" (estimate)");
        if (tx != null) {
            String v = verdict(r, tx.watts());
            sb.append("; ").append(tx.name()).append(" ").append(AntennaReport.watts(tx.watts())).append(" -> ")
                    .append("ok".equals(v) ? "within rating" : v);
        }
        return sb.toString();
    }
}
