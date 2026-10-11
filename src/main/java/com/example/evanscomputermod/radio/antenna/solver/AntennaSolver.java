package com.example.evanscomputermod.radio.antenna.solver;

/**
 * Thin-wire Method of Moments solver (the NEC approach, mixed-potential form).
 *
 * <ul>
 *   <li>Current: triangle (rooftop) basis functions spanning pairs of segments (see {@link AntennaMesh}),
 *       charge: the matching pulses. Galerkin testing with the same functions gives a symmetric matrix and
 *       power-consistent feed impedances.</li>
 *   <li>Kernel: the reduced thin-wire kernel e<sup>−jkR</sup>/(4πR) with R² = ρ² + a². The 1/R part is
 *       integrated analytically over the source segment and the smooth remainder by Gauss–Legendre, so
 *       self and adjacent terms stay accurate for very thin wires.</li>
 *   <li>Ground: images (see {@link Ground}).</li>
 *   <li>Loss: skin-effect surface impedance (1 + j)·R<sub>s</sub>/(2πa) per metre with
 *       R<sub>s</sub> = √(ωμ/2σ), falling back to the DC resistance when the skin depth exceeds the radius;
 *       plus lumped loads.</li>
 *   <li>Excitation: delta-gap voltage at the feed node.</li>
 * </ul>
 *
 * <p>Stateless: every call allocates its own working arrays, so one solve may run on each worker thread.
 */
public final class AntennaSolver {
    public static final double C0 = 299_792_458.0;
    public static final double MU0 = 4e-7 * Math.PI;
    public static final double EPS0 = 1 / (MU0 * C0 * C0);
    public static final double ETA0 = MU0 * C0;

    private static final double[] G4X, G4W, G8X, G8W, G16X, G16W;

    static {
        double[][] g4 = gauss(4), g8 = gauss(8), g16 = gauss(16);
        G4X = g4[0]; G4W = g4[1]; G8X = g8[0]; G8W = g8[1]; G16X = g16[0]; G16W = g16[1];
    }

    private AntennaSolver() {}

    /** Meshes the model at {@code frequencyHz} and solves it, including the gain pattern. */
    public static AntennaResult solve(AntennaModel model, double frequencyHz) {
        return solve(AntennaMesh.build(model, frequencyHz), frequencyHz, true);
    }

    public static AntennaResult solve(AntennaMesh mesh, double frequencyHz) { return solve(mesh, frequencyHz, true); }

    /**
     * Solves a prepared mesh at one frequency.
     *
     * @param pattern compute the 5° gain pattern (the bulk of the far-field cost); without it, gain fields are
     *                NaN and efficiency over real ground comes from a coarse 15° integration
     */
    public static AntennaResult solve(AntennaMesh mesh, double frequencyHz, boolean pattern) {
        if (!(frequencyHz > 0) || !Double.isFinite(frequencyHz)) throw new AntennaGeometryException("frequency");
        Work w = new Work(mesh, frequencyHz);
        w.fillPairs();
        w.assemble();
        w.solve();
        return w.result(pattern);
    }

    /** Per-solve working state. */
    private static final class Work {
        final AntennaMesh m;
        final double f, omega, k;
        final int S, N;
        final Ground ground;
        final boolean image;
        /** Pair integrals [p][q] → {A00, A01, A10, A11} as re/im pairs (8 doubles), direct and image. */
        final double[] direct, mirror;
        /** Per-pair image scale (re, im): 1 for a perfect ground. */
        final double[] gammaRe, gammaIm;
        double[] zRe, zIm;
        double[] iRe, iIm;
        double inRe, inIm;

        Work(AntennaMesh mesh, double f) {
            m = mesh;
            this.f = f;
            omega = 2 * Math.PI * f;
            k = omega / C0;
            S = mesh.segCount;
            N = mesh.basisCount;
            ground = mesh.model.ground();
            image = ground.present();
            direct = new double[S * S * 8];
            mirror = image ? new double[S * S * 8] : null;
            gammaRe = image ? new double[S * S] : null;
            gammaIm = image ? new double[S * S] : null;
        }

        void fillPairs() {
            double[] tmp = new double[8];
            for (int p = 0; p < S; p++) {
                for (int q = p; q < S; q++) {
                    double a2 = (m.radius[p] * m.radius[p] + m.radius[q] * m.radius[q]) / 2;
                    pair(m.sx[p], m.sy[p], m.sz[p], m.ux[p], m.uy[p], m.uz[p], m.len[p],
                            m.sx[q], m.sy[q], m.sz[q], m.ux[q], m.uy[q], m.uz[q], m.len[q], a2, k, tmp);
                    store(direct, p, q, tmp);
                    if (image) {
                        pair(m.sx[p], m.sy[p], m.sz[p], m.ux[p], m.uy[p], m.uz[p], m.len[p],
                                m.sx[q], m.sy[q], -m.sz[q], m.ux[q], m.uy[q], -m.uz[q], m.len[q], a2, k, tmp);
                        store(mirror, p, q, tmp);
                        double[] g = pairGamma(p, q);
                        gammaRe[p * S + q] = gammaRe[q * S + p] = g[0];
                        gammaIm[p * S + q] = gammaIm[q * S + p] = g[1];
                    }
                }
            }
        }

        private void store(double[] table, int p, int q, double[] v) {
            int a = (p * S + q) * 8, b = (q * S + p) * 8;
            System.arraycopy(v, 0, table, a, 8);
            if (p != q) {
                // Swapping roles swaps which segment's parameter weights the integral: A01 ↔ A10.
                table[b] = v[0]; table[b + 1] = v[1];
                table[b + 2] = v[4]; table[b + 3] = v[5];
                table[b + 4] = v[2]; table[b + 5] = v[3];
                table[b + 6] = v[6]; table[b + 7] = v[7];
            }
        }

        private double[] pairGamma(int p, int q) {
            if (ground.type() == Ground.Type.PERFECT) return new double[] {1, 0};
            double cpx = (m.sx[p] + m.ex[p]) / 2, cpy = (m.sy[p] + m.ey[p]) / 2, cpz = (m.sz[p] + m.ez[p]) / 2;
            double cqx = (m.sx[q] + m.ex[q]) / 2, cqy = (m.sy[q] + m.ey[q]) / 2, cqz = (m.sz[q] + m.ez[q]) / 2;
            double r = Math.sqrt((cpx - cqx) * (cpx - cqx) + (cpy - cqy) * (cpy - cqy) + (cpz + cqz) * (cpz + cqz));
            double sinPsi = Math.min(1, (cpz + cqz) / r);
            double wv = (m.uz[p] * m.uz[p] + m.uz[q] * m.uz[q]) / 2;
            Complex[] rc = fresnel(ground, omega, sinPsi);
            Complex g = rc[0].scale(wv).add(rc[1].negate().scale(1 - wv));
            return new double[] {g.re(), g.im()};
        }

        void assemble() {
            zRe = new double[N * N];
            zIm = new double[N * N];
            double vecScale = omega * MU0 / (4 * Math.PI); // multiplies j
            double scalScale = 1 / (omega * EPS0 * 4 * Math.PI); // multiplies -j
            for (int a = 0; a < N; a++) {
                int[] sa = m.halfSeg[a];
                double[] aa = m.halfAlpha[a], ba = m.halfBeta[a];
                for (int b = a; b < N; b++) {
                    int[] sb = m.halfSeg[b];
                    double[] ab = m.halfAlpha[b], bb = m.halfBeta[b];
                    double re = 0, im = 0;
                    for (int h = 0; h < sa.length; h++) {
                        int p = sa[h];
                        for (int g = 0; g < sb.length; g++) {
                            int q = sb[g];
                            double am = aa[h], bm = ba[h], an = ab[g], bn = bb[g];
                            double div = (bm / m.len[p]) * (bn / m.len[q]);
                            int o = (p * S + q) * 8;
                            double[] t = direct;
                            double dot = m.ux[p] * m.ux[q] + m.uy[p] * m.uy[q] + m.uz[p] * m.uz[q];
                            double wr = am * an * t[o] + am * bn * t[o + 2] + bm * an * t[o + 4] + bm * bn * t[o + 6];
                            double wi = am * an * t[o + 1] + am * bn * t[o + 3] + bm * an * t[o + 5] + bm * bn * t[o + 7];
                            // j·vec·dot·W − j·scal·div·A00
                            double cr = -vecScale * dot * wi + scalScale * div * t[o + 1];
                            double ci = vecScale * dot * wr - scalScale * div * t[o];
                            if (image) {
                                double[] u = mirror;
                                double dotI = m.ux[p] * m.ux[q] + m.uy[p] * m.uy[q] - m.uz[p] * m.uz[q];
                                double ir = am * an * u[o] + am * bn * u[o + 2] + bm * an * u[o + 4] + bm * bn * u[o + 6];
                                double ii = am * an * u[o + 1] + am * bn * u[o + 3] + bm * an * u[o + 5] + bm * bn * u[o + 7];
                                double mr = -vecScale * dotI * ii + scalScale * div * u[o + 1];
                                double mi = vecScale * dotI * ir - scalScale * div * u[o];
                                // Image currents are −Γ times the mirrored source.
                                double gr = gammaRe[p * S + q], gi = gammaIm[p * S + q];
                                cr -= gr * mr - gi * mi;
                                ci -= gr * mi + gi * mr;
                            }
                            re += cr;
                            im += ci;
                            if (p == q) {
                                // Wire loss: z' ∫ f_a f_b dl over the shared segment.
                                double[] zs = surfaceImpedance(m.radius[p], m.resistivity[p], omega);
                                double ov = m.len[p] * (am * an + (am * bn + bm * an) / 2 + bm * bn / 3);
                                re += zs[0] * ov;
                                im += zs[1] * ov;
                            }
                        }
                    }
                    zRe[a * N + b] = zRe[b * N + a] = re;
                    zIm[a * N + b] = zIm[b * N + a] = im;
                }
            }
            for (int l = 0; l < m.loadCoeff.length; l++) {
                double[] c = m.loadCoeff[l];
                Complex z = m.loadImpedance[l];
                for (int a = 0; a < N; a++) {
                    if (c[a] == 0) continue;
                    for (int b = 0; b < N; b++) {
                        if (c[b] == 0) continue;
                        zRe[a * N + b] += z.re() * c[a] * c[b];
                        zIm[a * N + b] += z.im() * c[a] * c[b];
                    }
                }
            }
        }

        void solve() {
            iRe = new double[N];
            iIm = new double[N];
            Complex v = m.feedVoltage;
            for (int a = 0; a < N; a++) { iRe[a] = v.re() * m.feedCoeff[a]; iIm[a] = v.im() * m.feedCoeff[a]; }
            ComplexLu.factorInPlace(N, zRe, zIm).solveInPlace(iRe, iIm);
            zRe = zIm = null;
            for (int a = 0; a < N; a++) { inRe += m.feedCoeff[a] * iRe[a]; inIm += m.feedCoeff[a] * iIm[a]; }
            if (inRe == 0 && inIm == 0) throw new ArithmeticException("no feed current");
        }

        AntennaResult result(boolean withPattern) {
            Complex v = m.feedVoltage, iin = Complex.of(inRe, inIm);
            Complex zin = v.div(iin);
            double pin = 0.5 * v.mul(iin.conj()).re();
            if (!(pin > 0)) throw new ArithmeticException("non-positive input power " + pin + " (Z=" + zin + ")");
            double perWatt = 1 / Math.sqrt(pin);

            // Segment currents I(t) = c0 + c1 t.
            double[] c0r = new double[S], c0i = new double[S], c1r = new double[S], c1i = new double[S];
            for (int a = 0; a < N; a++) {
                for (int h = 0; h < m.halfSeg[a].length; h++) {
                    int s = m.halfSeg[a][h];
                    double al = m.halfAlpha[a][h], be = m.halfBeta[a][h];
                    c0r[s] += al * iRe[a]; c0i[s] += al * iIm[a];
                    c1r[s] += be * iRe[a]; c1i[s] += be * iIm[a];
                }
            }
            double ploss = 0;
            for (int s = 0; s < S; s++) {
                double r = surfaceImpedance(m.radius[s], m.resistivity[s], omega)[0];
                double i2 = c0r[s] * c0r[s] + c0i[s] * c0i[s] + (c0r[s] * c1r[s] + c0i[s] * c1i[s])
                        + (c1r[s] * c1r[s] + c1i[s] * c1i[s]) / 3;
                ploss += 0.5 * r * m.len[s] * i2;
            }
            for (int l = 0; l < m.loadCoeff.length; l++) {
                double cr = 0, ci = 0;
                for (int a = 0; a < N; a++) { cr += m.loadCoeff[l][a] * iRe[a]; ci += m.loadCoeff[l][a] * iIm[a]; }
                ploss += 0.5 * m.loadImpedance[l].re() * (cr * cr + ci * ci);
            }
            double[] segCurrent = new double[S];
            double peakI = 0;
            for (int s = 0; s < S; s++) {
                segCurrent[s] = Math.hypot(c0r[s] + c1r[s] / 2, c0i[s] + c1i[s] / 2) * perWatt;
                double e0 = Math.hypot(c0r[s], c0i[s]), e1 = Math.hypot(c0r[s] + c1r[s], c0i[s] + c1i[s]);
                peakI = Math.max(peakI, Math.max(segCurrent[s], Math.max(e0, e1) * perWatt));
            }

            // Far field.
            double prad = Math.max(0, pin - ploss);
            GainPattern pattern = null;
            double peakG = Double.NaN, peakT = Double.NaN, peakP = Double.NaN;
            if (withPattern || ground.type() == Ground.Type.REAL) {
                int step = withPattern ? GainPattern.STEP_DEG : 15;
                double[][] far = farField(step, c0r, c0i, c1r, c1i, pin);
                if (ground.type() == Ground.Type.REAL) {
                    // Space-wave power over the upper hemisphere (trapezoid in θ).
                    int np = 360 / step;
                    double dt = Math.toRadians(step), dp = Math.toRadians(step), sum = 0;
                    for (int ti = 0; ti <= 90 / step; ti++) {
                        double wt = (ti == 0 || ti == 90 / step) ? 0.5 : 1;
                        double st = Math.sin(Math.toRadians(ti * step));
                        for (int pj = 0; pj < np; pj++) sum += (far[0][ti * np + pj] + far[1][ti * np + pj]) * st * wt;
                    }
                    double gainIntegral = sum * dt * dp / (4 * Math.PI); // = P_rad / P_in
                    prad = Math.min(prad, gainIntegral * pin);
                }
                if (withPattern) {
                    pattern = new GainPattern(far[0], far[1], far[2]);
                    peakG = pattern.peakDbi();
                    peakT = GainPattern.thetaDeg(pattern.peakThetaIndex());
                    peakP = GainPattern.phiDeg(pattern.peakPhiIndex());
                }
            }
            double eff = Math.max(0, Math.min(1, prad / pin));
            double vEnd = endVoltage(c1r, c1i) * perWatt;
            return new AntennaResult(f, zin, eff, Math.min(1, ploss / pin), pattern, peakG, peakT, peakP, segCurrent, peakI, vEnd);
        }

        /** Returns {gainTheta[], gainPhi[], phase(Eφ) − phase(Eθ)[]} on the given grid step. */
        double[][] farField(int step, double[] c0r, double[] c0i, double[] c1r, double[] c1i, double pin) {
            int nt = 180 / step + 1, np = 360 / step;
            double[] gt = new double[nt * np], gp = new double[nt * np], ph = new double[nt * np];
            double scale = ETA0 * k * k / (8 * Math.PI * pin);
            double[] acc = new double[6];
            for (int ti = 0; ti < nt; ti++) {
                double th = Math.toRadians(ti * step), st = Math.sin(th), ct = Math.cos(th);
                // Below the antenna's horizontal (θ > 90°) over ground, a receiver is still above the
                // ground but nearer than the far field: it gets the direct ray of the solved currents
                // (which already include the ground's effect on them), while that path's own ground
                // reflection arrives at another angle and is the propagation model's two-ray term.
                // So the pattern there is the currents' direct radiation, not zero.
                boolean below = image && ti * step > 90;
                Complex rv = Complex.ONE, rh = Complex.ONE;
                if (ground.type() == Ground.Type.REAL) {
                    Complex[] rc = fresnel(ground, omega, Math.max(0, ct));
                    rv = rc[0];
                    rh = rc[1].negate();
                }
                for (int pj = 0; pj < np; pj++) {
                    double phi = Math.toRadians(pj * step), cp = Math.cos(phi), sp = Math.sin(phi);
                    double rx = st * cp, ry = st * sp, rz = ct;
                    double tx = ct * cp, ty = ct * sp, tz = -st, px = -sp, py = cp;
                    radiationVector(rx, ry, rz, false, c0r, c0i, c1r, c1i, acc);
                    double ntr = acc[0] * tx + acc[2] * ty + acc[4] * tz, nti = acc[1] * tx + acc[3] * ty + acc[5] * tz;
                    double npr = acc[0] * px + acc[2] * py, npi = acc[1] * px + acc[3] * py;
                    if (image && !below) {
                        radiationVector(rx, ry, rz, true, c0r, c0i, c1r, c1i, acc);
                        double itr = acc[0] * tx + acc[2] * ty + acc[4] * tz, iti = acc[1] * tx + acc[3] * ty + acc[5] * tz;
                        double ipr = acc[0] * px + acc[2] * py, ipi = acc[1] * px + acc[3] * py;
                        ntr += rv.re() * itr - rv.im() * iti;
                        nti += rv.re() * iti + rv.im() * itr;
                        npr += rh.re() * ipr - rh.im() * ipi;
                        npi += rh.re() * ipi + rh.im() * ipr;
                    }
                    int idx = ti * np + pj;
                    gt[idx] = scale * (ntr * ntr + nti * nti);
                    gp[idx] = scale * (npr * npr + npi * npi);
                    ph[idx] = Math.atan2(npi, npr) - Math.atan2(nti, ntr);
                }
            }
            return new double[][] {gt, gp, ph};
        }

        /**
         * Radiation vector Σ ∫ I(l) û e^{jk r̂·r'} dl (re/im of x, y, z into out). With {@code mirrored} it is the
         * perfect-ground image: currents −I along mirrored segments.
         */
        void radiationVector(double rx, double ry, double rz, boolean mirrored, double[] c0r, double[] c0i,
                double[] c1r, double[] c1i, double[] out) {
            double sxr = 0, sxi = 0, syr = 0, syi = 0, szr = 0, szi = 0;
            double sgn = mirrored ? -1 : 1;
            for (int s = 0; s < S; s++) {
                double uzs = mirrored ? -m.uz[s] : m.uz[s], zs = mirrored ? -m.sz[s] : m.sz[s];
                double ph0 = k * (rx * m.sx[s] + ry * m.sy[s] + rz * zs);
                double b = k * m.len[s] * (rx * m.ux[s] + ry * m.uy[s] + rz * uzs);
                double e0r, e0i, e1r, e1i;
                if (Math.abs(b) < 1e-3) {
                    double b2 = b * b;
                    e0r = 1 - b2 / 6; e0i = b / 2 - b * b2 / 24;
                    e1r = 0.5 - b2 / 8; e1i = b / 3 - b * b2 / 30;
                } else {
                    double cb = Math.cos(b), sb = Math.sin(b);
                    // E0 = (e^{jb} − 1)/(jb); E1 = e^{jb}/(jb) + (e^{jb} − 1)/b².
                    e0r = sb / b; e0i = (1 - cb) / b;
                    e1r = sb / b + (cb - 1) / (b * b);
                    e1i = -cb / b + sb / (b * b);
                }
                double vr = c0r[s] * e0r - c0i[s] * e0i + c1r[s] * e1r - c1i[s] * e1i;
                double vi = c0r[s] * e0i + c0i[s] * e0r + c1r[s] * e1i + c1i[s] * e1r;
                double cr = Math.cos(ph0), ci = Math.sin(ph0);
                double tr = (vr * cr - vi * ci) * m.len[s] * sgn, ti = (vr * ci + vi * cr) * m.len[s] * sgn;
                sxr += tr * m.ux[s]; sxi += ti * m.ux[s];
                syr += tr * m.uy[s]; syi += ti * m.uy[s];
                szr += tr * uzs; szi += ti * uzs;
            }
            out[0] = sxr; out[1] = sxi; out[2] = syr; out[3] = syi; out[4] = szr; out[5] = szi;
        }

        /**
         * Largest wire potential |Φ| near an open end: at the end node, the middle of the end segment and the
         * node behind it (the thin-wire potential at the very tip is depressed by the end effect, so the
         * wire just inside it is the better measure of what an end insulator sees). Without open ends
         * (loops), every ungrounded node is probed.
         */
        double endVoltage(double[] c1r, double[] c1i) {
            java.util.List<double[]> probes = new java.util.ArrayList<>();
            for (int s = 0; s < S; s++) {
                boolean startOpen = m.openEnd[m.segStart[s]], endOpen = m.openEnd[m.segEnd[s]];
                if (!startOpen && !endOpen) continue;
                probes.add(new double[] {m.sx[s], m.sy[s], m.sz[s]});
                probes.add(new double[] {(m.sx[s] + m.ex[s]) / 2, (m.sy[s] + m.ey[s]) / 2, (m.sz[s] + m.ez[s]) / 2});
                probes.add(new double[] {m.ex[s], m.ey[s], m.ez[s]});
            }
            if (probes.isEmpty())
                for (int n = 0; n < m.nodeCount; n++) if (!m.grounded[n]) probes.add(new double[] {m.nx[n], m.ny[n], m.nz[n]});
            double best = 0;
            double[] tmp = new double[4];
            for (double[] o : probes) {
                double pr = 0, pi = 0;
                for (int s = 0; s < S; s++) {
                    // Line charge ρ = −(dI/dl)/(jω) = j·c1/(ωL).
                    double rr = -c1i[s] / (omega * m.len[s]), ri = c1r[s] / (omega * m.len[s]);
                    double a2 = m.radius[s] * m.radius[s];
                    inner(o[0], o[1], o[2], m.sx[s], m.sy[s], m.sz[s], m.ux[s], m.uy[s], m.uz[s], m.len[s], a2, k, tmp);
                    double gr = tmp[0], gi = tmp[1];
                    if (image) {
                        inner(o[0], o[1], o[2], m.sx[s], m.sy[s], -m.sz[s], m.ux[s], m.uy[s], -m.uz[s], m.len[s], a2, k, tmp);
                        double[] g = ground.type() == Ground.Type.PERFECT ? new double[] {1, 0} : probeGamma(o, s);
                        gr -= g[0] * tmp[0] - g[1] * tmp[1];
                        gi -= g[0] * tmp[1] + g[1] * tmp[0];
                    }
                    pr += rr * gr - ri * gi;
                    pi += rr * gi + ri * gr;
                }
                best = Math.max(best, Math.hypot(pr, pi) / (4 * Math.PI * EPS0));
            }
            return best;
        }

        private double[] probeGamma(double[] o, int s) {
            double cz = (m.sz[s] + m.ez[s]) / 2, cx = (m.sx[s] + m.ex[s]) / 2, cy = (m.sy[s] + m.ey[s]) / 2;
            double r = Math.sqrt((o[0] - cx) * (o[0] - cx) + (o[1] - cy) * (o[1] - cy) + (o[2] + cz) * (o[2] + cz));
            Complex[] rc = fresnel(ground, omega, r > 0 ? Math.min(1, (o[2] + cz) / r) : 1);
            double wv = m.uz[s] * m.uz[s];
            Complex g = rc[0].scale(wv).add(rc[1].negate().scale(1 - wv));
            return new double[] {g.re(), g.im()};
        }
    }

    /**
     * Fresnel reflection coefficients {R_v (TM), R_h (TE)} at grazing angle ψ (given as sin ψ) for the
     * ground's complex permittivity. For a perfect conductor R_v = 1 and R_h = −1.
     */
    static Complex[] fresnel(Ground ground, double omega, double sinPsi) {
        if (ground.type() != Ground.Type.REAL) return new Complex[] {Complex.ONE, Complex.real(-1)};
        Complex eps = Complex.of(ground.relativePermittivity(), -ground.conductivity() / (omega * EPS0));
        double cos2 = 1 - sinPsi * sinPsi;
        Complex root = eps.sub(Complex.real(cos2)).sqrt();
        Complex es = eps.scale(sinPsi);
        Complex rv = es.sub(root).div(es.add(root));
        Complex rh = Complex.real(sinPsi).sub(root).div(Complex.real(sinPsi).add(root));
        return new Complex[] {rv, rh};
    }

    /** Series surface impedance per metre {R', X'} of a round wire. */
    static double[] surfaceImpedance(double radius, double resistivity, double omega) {
        if (resistivity <= 0) return new double[] {0, 0};
        double rdc = resistivity / (Math.PI * radius * radius);
        double rs = Math.sqrt(omega * MU0 * resistivity / 2);
        double skin = Math.sqrt(2 * resistivity / (omega * MU0));
        if (skin >= radius) return new double[] {rdc, omega * MU0 / (8 * Math.PI)};
        double rac = rs / (2 * Math.PI * radius);
        return new double[] {Math.max(rac, rdc), rac};
    }

    /**
     * Pair integrals of the reduced kernel e^{−jkR}/R (no 1/4π) between observation segment p and source
     * segment q: out = {A00, A01, A10, A11} as (re, im), A_ij = ∫∫ t^i t'^j G dl dl'.
     */
    static void pair(double px, double py, double pz, double pux, double puy, double puz, double lp,
            double qx, double qy, double qz, double qux, double quy, double quz, double lq, double a2, double k,
            double[] out) {
        java.util.Arrays.fill(out, 0);
        double cpx = px + pux * lp / 2, cpy = py + puy * lp / 2, cpz = pz + puz * lp / 2;
        double cqx = qx + qux * lq / 2, cqy = qy + quy * lq / 2, cqz = qz + quz * lq / 2;
        double dc = Math.sqrt((cpx - cqx) * (cpx - cqx) + (cpy - cqy) * (cpy - cqy) + (cpz - cqz) * (cpz - cqz));
        double size = lp + lq;
        if (dc <= 1.01 * size) {
            double[] tmp = new double[4];
            for (int i = 0; i < G16X.length; i++) {
                double t = G16X[i], w = G16W[i] * lp;
                inner(px + pux * lp * t, py + puy * lp * t, pz + puz * lp * t, qx, qy, qz, qux, quy, quz, lq, a2, k, tmp);
                out[0] += w * tmp[0]; out[1] += w * tmp[1];
                out[2] += w * tmp[2]; out[3] += w * tmp[3];
                out[4] += w * t * tmp[0]; out[5] += w * t * tmp[1];
                out[6] += w * t * tmp[2]; out[7] += w * t * tmp[3];
            }
            return;
        }
        double[] xs = dc <= 3 * size ? G8X : G4X, ws = dc <= 3 * size ? G8W : G4W;
        for (int i = 0; i < xs.length; i++) {
            double t = xs[i], wt = ws[i] * lp;
            double ox = px + pux * lp * t, oy = py + puy * lp * t, oz = pz + puz * lp * t;
            for (int j = 0; j < xs.length; j++) {
                double s = xs[j], w = wt * ws[j] * lq;
                double dx = ox - qx - qux * lq * s, dy = oy - qy - quy * lq * s, dz = oz - qz - quz * lq * s;
                double r = Math.sqrt(dx * dx + dy * dy + dz * dz + a2);
                double gr = Math.cos(k * r) / r * w, gi = -Math.sin(k * r) / r * w;
                out[0] += gr; out[1] += gi;
                out[2] += s * gr; out[3] += s * gi;
                out[4] += t * gr; out[5] += t * gi;
                out[6] += t * s * gr; out[7] += t * s * gi;
            }
        }
    }

    /**
     * ∫ e^{−jkR}/R dl' and ∫ t' e^{−jkR}/R dl' over source segment q seen from point (ox, oy, oz), with the
     * 1/R part done analytically. out = {I0re, I0im, I1re, I1im}.
     */
    static void inner(double ox, double oy, double oz, double qx, double qy, double qz, double qux, double quy,
            double quz, double lq, double a2, double k, double[] out) {
        double dx = ox - qx, dy = oy - qy, dz = oz - qz;
        double z = dx * qux + dy * quy + dz * quz;
        double rho2 = Math.max(0, dx * dx + dy * dy + dz * dz - z * z) + a2;
        double rho = Math.sqrt(rho2);
        double s0 = asinh((lq - z) / rho) + asinh(z / rho);
        double rl = Math.sqrt(rho2 + (lq - z) * (lq - z)), r0 = Math.sqrt(rho2 + z * z);
        double s1 = ((rl - r0) + z * s0) / lq;
        double i0r = s0, i0i = 0, i1r = s1, i1i = 0;
        for (int j = 0; j < G8X.length; j++) {
            double s = G8X[j], w = G8W[j] * lq;
            double r = Math.sqrt(rho2 + (s * lq - z) * (s * lq - z));
            double half = Math.sin(k * r / 2);
            double fr = -2 * half * half / r * w, fi = -Math.sin(k * r) / r * w;
            i0r += fr; i0i += fi;
            i1r += s * fr; i1i += s * fi;
        }
        out[0] = i0r; out[1] = i0i; out[2] = i1r; out[3] = i1i;
    }

    static double asinh(double x) {
        double a = Math.abs(x);
        double r = a > 1e8 ? Math.log(2 * a) : Math.log(a + Math.sqrt(a * a + 1));
        return Math.copySign(r, x);
    }

    /** Gauss–Legendre nodes and weights on [0, 1]. */
    static double[][] gauss(int n) {
        double[] x = new double[n], w = new double[n];
        for (int i = 0; i < n; i++) {
            double z = Math.cos(Math.PI * (i + 0.75) / (n + 0.5)), dp = 0;
            for (int it = 0; it < 100; it++) {
                double p1 = 1, p2 = 0;
                for (int j = 1; j <= n; j++) {
                    double p3 = p2;
                    p2 = p1;
                    p1 = ((2 * j - 1) * z * p2 - (j - 1) * p3) / j;
                }
                dp = n * (z * p1 - p2) / (z * z - 1);
                double z1 = z;
                z = z1 - p1 / dp;
                if (Math.abs(z - z1) < 1e-15) break;
            }
            x[i] = (1 - z) / 2;
            w[i] = 1 / ((1 - z * z) * dp * dp);
        }
        return new double[][] {x, w};
    }
}
