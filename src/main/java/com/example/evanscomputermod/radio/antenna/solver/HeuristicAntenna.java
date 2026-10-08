package com.example.evanscomputermod.radio.antenna.solver;

import java.util.ArrayList;
import java.util.List;

/**
 * Closed-form estimates for antennas the MoM solver cannot (yet) answer for:
 * above {@link AntennaMesh#MAX_SEGMENTS}, or while a solve is pending.
 *
 * <p>The wires connected to the feed are classified by shape:
 * <ul>
 *   <li>{@link Type#DIPOLE}: an open chain of wires fed away from its ends. Induced-EMF theory for a
 *       sinusoidal current (Balanis 8.60/8.61, 73 + j42.5 Ω at λ/2), referred to the feed point; the
 *       directivity is integrated from the sinusoidal-current pattern. Bent chains (inverted V) lose 0.5 dB.</li>
 *   <li>{@link Type#MONOPOLE}: a chain fed where it touches the ground plane: half the impedance and twice
 *       the directivity of the dipole of twice the height. Real ground adds a ground-loss resistance of
 *       2/√σ Ω (≤ 40 Ω) and costs 2 dB of low-angle gain.</li>
 *   <li>{@link Type#LOOP}: a closed ring. Small loops (C &lt; 0.7λ) use the uniform-current radiation
 *       resistance 31171·A²/λ⁴ and the ring inductance μ₀b(ln(8b/a) − 2); full-wave loops use a fit to
 *       this solver's square-loop results (resonant near C ≈ 1.06λ at ≈ 125 Ω, 3.1 dBi).</li>
 *   <li>{@link Type#LONG_WIRE}: an open chain at least 1λ long fed within 10% of an end:
 *       R ≈ 73 + 69·log₁₀(2L/λ) Ω, X ≈ 0, gain ≈ 2.55 + 6·log₁₀(L/λ) dBi (ARRL-style long-wire table).</li>
 *   <li>{@link Type#UNKNOWN}: anything else: the dipole estimate for the total connected wire length, capped
 *       at 2.15 dBi.</li>
 * </ul>
 * Wires not connected to the feed (parasitic elements) are ignored. Over a ground plane, elements not touching
 * it gain +3 dB (real) or +5 dB (perfect) from the ground reflection, independent of height.
 * Wire loss uses the same skin-effect model as the solver.
 */
public final class HeuristicAntenna {
    private static final double C0 = AntennaSolver.C0;
    private static final double ETA0 = AntennaSolver.ETA0;
    private static final double EULER = 0.5772156649015329;

    private HeuristicAntenna() {}

    public enum Type { DIPOLE, MONOPOLE, LOOP, LONG_WIRE, UNKNOWN }

    /** An approximate antenna description at one frequency. */
    public record Estimate(Type type, double frequencyHz, Complex feedImpedance, double gainDbi, double efficiency) {
        public double swr(double z0) { return AntennaResult.swr(feedImpedance, Complex.real(z0)); }

        public double swr() { return swr(AntennaResult.DEFAULT_Z0); }
    }

    public static Estimate estimate(AntennaModel model, double frequencyHz) {
        if (!(frequencyHz > 0) || !Double.isFinite(frequencyHz)) throw new AntennaGeometryException("frequency");
        double lambda = C0 / frequencyHz, k = 2 * Math.PI / lambda, omega = 2 * Math.PI * frequencyHz;
        Shape s = Shape.of(model);
        if (s.type == Type.DIPOLE && s.length >= lambda && s.feedOffset > 0.4 * s.length)
            s = new Shape(Type.LONG_WIRE, s.length, s.feedOffset, s.straight, s.radius, s.resistivity, 0);
        Ground g = model.ground();
        double rPrime = AntennaSolver.surfaceImpedance(s.radius, s.resistivity, omega)[0];
        double groundBonus = !g.present() ? 0 : g.type() == Ground.Type.PERFECT ? 5 : 3;
        switch (s.type) {
            case MONOPOLE -> {
                Dipole d = dipole(2 * s.length, s.radius, k, 0);
                double rLoss = d.lossFactor * rPrime / 2;
                double rGround = g.type() == Ground.Type.REAL ? Math.min(40, 2 / Math.sqrt(Math.max(g.conductivity(), 1e-6))) : 0;
                Complex z = d.z.scale(0.5).add(Complex.real(rLoss + rGround));
                double eff = d.z.re() / 2 / (d.z.re() / 2 + rLoss + rGround);
                double gain = 10 * Math.log10(2 * d.directivity * eff) - (g.type() == Ground.Type.REAL ? 2 : 0);
                return new Estimate(Type.MONOPOLE, frequencyHz, z, gain, eff);
            }
            case DIPOLE, UNKNOWN -> {
                double offset = s.type == Type.DIPOLE ? s.feedOffset : 0;
                Dipole d = dipole(s.length, s.radius, k, offset);
                double rLoss = d.lossFactor * rPrime;
                double eff = d.z.re() / (d.z.re() + rLoss);
                double gain = 10 * Math.log10(d.directivity * eff) - (s.straight ? 0 : 0.5) + groundBonus;
                if (s.type == Type.UNKNOWN) gain = Math.min(gain, 2.15);
                return new Estimate(s.type, frequencyHz, d.z.add(Complex.real(rLoss)), gain, eff);
            }
            case LONG_WIRE -> {
                double n = s.length / lambda;
                double r = 73 + 69 * Math.log10(2 * n), rLoss = rPrime * s.length / 2;
                double eff = r / (r + rLoss);
                double gain = 2.55 + 6 * Math.log10(n) + 10 * Math.log10(eff) + groundBonus;
                return new Estimate(Type.LONG_WIRE, frequencyHz, Complex.real(r + rLoss), gain, eff);
            }
            case LOOP -> {
                double c = s.length / lambda;
                Complex z;
                double rLoss, directivity;
                if (c < 0.7) {
                    double b = s.length / (2 * Math.PI);
                    double rr = 31171 * s.area * s.area / Math.pow(lambda, 4);
                    double x = omega * AntennaSolver.MU0 * b * (Math.log(8 * b / s.radius) - 2);
                    rLoss = rPrime * s.length;
                    z = Complex.of(rr, x);
                    directivity = 1.5;
                } else if (c <= 1.5) {
                    z = Complex.of(122 + 330 * (c - 1.05), 2400 * (c - 1.062));
                    rLoss = rPrime * s.length / 2;
                    directivity = Math.pow(10, 0.31);
                } else {
                    z = Complex.of(150, 0);
                    rLoss = rPrime * s.length / 2;
                    directivity = Math.pow(10, 0.4);
                }
                double eff = z.re() / (z.re() + rLoss);
                return new Estimate(Type.LOOP, frequencyHz, z.add(Complex.real(rLoss)),
                        10 * Math.log10(directivity * eff) + groundBonus, eff);
            }
        }
        throw new IllegalStateException();
    }

    private record Dipole(Complex z, double directivity, double lossFactor) {}

    /**
     * Centre-fed sinusoidal-current dipole of length l, with the impedance referred to a feed {@code offset}
     * metres from the centre. lossFactor × R' is the wire-loss resistance referred to the same feed.
     */
    private static Dipole dipole(double l, double a, double k, double offset) {
        double kl = k * l;
        double feedSin = Math.sin(k * (l / 2 - Math.abs(offset)));
        double feedSin2 = Math.max(feedSin * feedSin, 0.01);
        Complex zm;
        if (kl < 0.3) {
            // Short dipole: triangular current, impedance at the centre.
            double lambda = 2 * Math.PI / k;
            double r = 20 * Math.PI * Math.PI * (l / lambda) * (l / lambda);
            double x = -120 * (Math.log(l / (2 * a)) - 1) / Math.tan(kl / 2);
            zm = Complex.of(r, x).scale(Math.sin(kl / 2) * Math.sin(kl / 2));
        } else {
            double s = Math.sin(kl), c = Math.cos(kl);
            double rr = ETA0 / (2 * Math.PI) * (EULER + Math.log(kl) - ci(kl) + 0.5 * s * (si(2 * kl) - 2 * si(kl))
                    + 0.5 * c * (EULER + Math.log(kl / 2) + ci(2 * kl) - 2 * ci(kl)));
            double xm = ETA0 / (4 * Math.PI) * (2 * si(kl) + c * (2 * si(kl) - si(2 * kl))
                    - s * (2 * ci(kl) - ci(2 * kl) - ci(2 * k * a * a / l)));
            zm = Complex.of(rr, xm);
        }
        Complex z = zm.scale(1 / feedSin2);
        // Directivity of the sinusoidal-current pattern.
        int steps = 720;
        double max = 0, sum = 0, ch = Math.cos(kl / 2);
        for (int i = 1; i < steps; i++) {
            double th = Math.PI * i / steps, st = Math.sin(th);
            double fth = (Math.cos(kl / 2 * Math.cos(th)) - ch) / st;
            double u = fth * fth;
            max = Math.max(max, u);
            sum += u * st * Math.PI / steps;
        }
        double directivity = sum > 0 ? 2 * max / sum : 1.5;
        // Loss: ∫ |I(z)|² dz / |I_feed|² with I(z) = sin(k(l/2 − |z|)).
        double integral = 0;
        for (int i = 0; i < steps; i++) {
            double z0 = -l / 2 + l * (i + 0.5) / steps, cur = Math.sin(k * (l / 2 - Math.abs(z0)));
            integral += cur * cur * l / steps;
        }
        double lossFactor = kl < 0.3 ? l / 3 : integral / feedSin2;
        return new Dipole(z, directivity, lossFactor);
    }

    private static final double SERIES_LIMIT = 20;

    /** Sine integral Si(x): power series up to x = 20, asymptotic expansion beyond. */
    static double si(double x) {
        if (x < 0) return -si(-x);
        if (x <= SERIES_LIMIT) {
            double sum = 0, term = x;
            for (int n = 0; n < 200 && Math.abs(term) > 1e-18 * Math.max(1, Math.abs(sum)); n++) {
                sum += term / (2 * n + 1);
                term *= -x * x / ((2 * n + 2) * (2 * n + 3));
            }
            return sum;
        }
        double[] fg = auxiliary(x);
        return Math.PI / 2 - fg[0] * Math.cos(x) - fg[1] * Math.sin(x);
    }

    /** Cosine integral Ci(x), x &gt; 0. */
    static double ci(double x) {
        if (x <= SERIES_LIMIT) {
            double sum = 0, term = -x * x / 2;
            for (int n = 1; n < 200 && Math.abs(term) > 1e-18 * Math.max(1, Math.abs(sum)); n++) {
                sum += term / (2 * n);
                term *= -x * x / ((2 * n + 1) * (2 * n + 2));
            }
            return EULER + Math.log(x) + sum;
        }
        double[] fg = auxiliary(x);
        return fg[0] * Math.sin(x) - fg[1] * Math.cos(x);
    }

    /** Auxiliary functions f(x), g(x) by their asymptotic series (A&amp;S 5.2.34/5.2.35), x &gt; 20. */
    private static double[] auxiliary(double x) {
        double f = 0, g = 0, tf = 1 / x, tg = 1 / (x * x);
        for (int n = 0; n < 40; n++) {
            f += tf;
            g += tg;
            double nf = tf * -(2 * n + 1) * (2 * n + 2) / (x * x), ng = tg * -(2 * n + 2) * (2 * n + 3) / (x * x);
            if (Math.abs(nf) >= Math.abs(tf) || Math.abs(ng) >= Math.abs(tg)) break; // divergent tail: stop at the smallest term
            tf = nf;
            tg = ng;
        }
        return new double[] {f, g};
    }

    /** Shape of the wires connected to the feed. */
    private record Shape(Type type, double length, double feedOffset, boolean straight, double radius, double resistivity, double area) {
        static Shape of(AntennaModel model) {
            List<Wire> wires = model.wires();
            double tol = AntennaModel.JOIN_TOLERANCE;
            List<double[]> nodes = new ArrayList<>();
            int[][] ends = new int[wires.size()][2];
            for (int i = 0; i < wires.size(); i++) {
                Wire w = wires.get(i);
                ends[i][0] = node(nodes, w.x1(), w.y1(), w.z1(), tol);
                ends[i][1] = node(nodes, w.x2(), w.y2(), w.z2(), tol);
            }
            // Connected component of the feed wire.
            boolean[] in = new boolean[wires.size()];
            in[model.feed().wire()] = true;
            for (boolean grew = true; grew; ) {
                grew = false;
                for (int i = 0; i < wires.size(); i++) {
                    if (in[i]) continue;
                    for (int j = 0; j < wires.size(); j++) {
                        if (!in[j]) continue;
                        if (ends[i][0] == ends[j][0] || ends[i][0] == ends[j][1] || ends[i][1] == ends[j][0] || ends[i][1] == ends[j][1]) {
                            in[i] = true;
                            grew = true;
                            break;
                        }
                    }
                }
            }
            int[] degree = new int[nodes.size()];
            int edges = 0;
            double total = 0, rSum = 0, rhoSum = 0;
            for (int i = 0; i < wires.size(); i++) {
                if (!in[i]) continue;
                edges++;
                degree[ends[i][0]]++;
                degree[ends[i][1]]++;
                double l = wires.get(i).length();
                total += l;
                rSum += wires.get(i).radius() * l;
                rhoSum += wires.get(i).resistivity() * l;
            }
            double radius = rSum / total, rho = rhoSum / total;
            int used = 0, maxDegree = 0;
            for (int d : degree) { if (d > 0) used++; maxDegree = Math.max(maxDegree, d); }
            Feed feed = model.feed();
            if (maxDegree > 2) return new Shape(Type.UNKNOWN, total, 0, false, radius, rho, 0);
            if (edges == used) {
                return new Shape(Type.LOOP, total, 0, false, radius, rho, loopArea(wires, in, ends, nodes));
            }
            // Open chain: walk from an end, measuring the feed's distance along it.
            int start = -1;
            for (int n = 0; n < degree.length; n++) if (degree[n] == 1) { start = n; break; }
            double along = 0, feedAt = 0;
            boolean straight = true;
            double[] dir = null;
            boolean[] walked = new boolean[wires.size()];
            int node = start;
            for (int step = 0; step < edges; step++) {
                int next = -1;
                for (int i = 0; i < wires.size(); i++) if (in[i] && !walked[i] && (ends[i][0] == node || ends[i][1] == node)) { next = i; break; }
                if (next < 0) break;
                walked[next] = true;
                Wire w = wires.get(next);
                boolean forward = ends[next][0] == node;
                double l = w.length();
                double[] d = {(w.x2() - w.x1()) / l * (forward ? 1 : -1), (w.y2() - w.y1()) / l * (forward ? 1 : -1), (w.z2() - w.z1()) / l * (forward ? 1 : -1)};
                if (dir != null && d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2] < 0.999) straight = false;
                dir = d;
                if (next == feed.wire()) feedAt = along + (forward ? feed.position() : 1 - feed.position()) * l;
                along += l;
                node = forward ? ends[next][1] : ends[next][0];
            }
            boolean groundPlane = model.ground().present();
            double[] startPos = nodes.get(start), endPos = nodes.get(node);
            boolean startGrounded = groundPlane && Math.abs(startPos[2]) <= tol, endGrounded = groundPlane && Math.abs(endPos[2]) <= tol;
            if ((startGrounded && feedAt <= tol * 10) || (endGrounded && total - feedAt <= tol * 10))
                return new Shape(Type.MONOPOLE, total, 0, straight, radius, rho, 0);
            // Long wires are told apart from dipoles in estimate(), which knows the wavelength.
            double offset = Math.abs(feedAt - total / 2);
            return new Shape(Type.DIPOLE, total, offset, straight, radius, rho, 0);
        }

        private static int node(List<double[]> nodes, double x, double y, double z, double tol) {
            for (int i = 0; i < nodes.size(); i++) {
                double[] n = nodes.get(i);
                if (Math.abs(n[0] - x) <= tol && Math.abs(n[1] - y) <= tol && Math.abs(n[2] - z) <= tol) return i;
            }
            nodes.add(new double[] {x, y, z});
            return nodes.size() - 1;
        }

        /** Area of the ring's polygon (vector area magnitude). */
        private static double loopArea(List<Wire> wires, boolean[] in, int[][] ends, List<double[]> nodes) {
            double ax = 0, ay = 0, az = 0;
            int first = -1;
            for (int i = 0; i < wires.size(); i++) if (in[i]) { first = i; break; }
            boolean[] walked = new boolean[wires.size()];
            int node = ends[first][0];
            for (int step = 0; step < wires.size(); step++) {
                int next = -1;
                for (int i = 0; i < wires.size(); i++) if (in[i] && !walked[i] && (ends[i][0] == node || ends[i][1] == node)) { next = i; break; }
                if (next < 0) break;
                walked[next] = true;
                int other = ends[next][0] == node ? ends[next][1] : ends[next][0];
                double[] p = nodes.get(node), q = nodes.get(other);
                ax += p[1] * q[2] - p[2] * q[1];
                ay += p[2] * q[0] - p[0] * q[2];
                az += p[0] * q[1] - p[1] * q[0];
                node = other;
            }
            return Math.sqrt(ax * ax + ay * ay + az * az) / 2;
        }
    }
}
