package com.example.evanscomputermod.radio.antenna.solver;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * The discretised antenna: straight segments, nodes, and the triangle
 * ("rooftop") current basis functions the solver works with. Immutable and
 * safe to share between threads; build it once per geometry and reuse it for
 * every frequency of a sweep so the segmentation does not jump between points.
 *
 * <h2>Segmentation</h2>
 * Wires that continue each other in a straight line through a node shared by
 * nothing else (same radius and resistivity, both auto-split, not grounded)
 * are first merged into one run, so a row of 1 m block wires costs no more
 * than one long wire. Auto-split runs are then cut at every feed and load
 * position and each piece is split into segments no longer than
 * λ/{@link #SEGMENTS_PER_WAVELENGTH} at the design frequency, falling back to
 * λ/{@link #MIN_SEGMENTS_PER_WAVELENGTH} if that would exceed
 * {@link #MAX_SEGMENTS}. Segments are kept at least 4 radii long where the
 * piece allows it (the thin-wire kernel's validity limit). Wires with an
 * explicit segment count are split exactly as asked; feeds and loads on them
 * snap to the nearest node.
 *
 * <h2>Basis functions</h2>
 * One triangle per pair of segments meeting at a node: at a node with k
 * segments the first is paired with each of the others (k − 1 unknowns),
 * which enforces current continuity. At a grounded node each segment gets a
 * half triangle whose image completes it below the ground. Open wire ends
 * carry no basis, so their current is zero.
 */
public final class AntennaMesh {
    public static final int MAX_SEGMENTS = 200;
    public static final double SEGMENTS_PER_WAVELENGTH = 20;
    public static final double MIN_SEGMENTS_PER_WAVELENGTH = 10;
    private static final double C0 = 299_792_458.0;
    private static final double TOL = AntennaModel.JOIN_TOLERANCE;

    final AntennaModel model;
    final double designFrequency;
    // Segments.
    final int segCount;
    final double[] sx, sy, sz, ex, ey, ez, len, ux, uy, uz, radius, resistivity;
    final int[] segStart, segEnd, segWire;
    // Nodes.
    final int nodeCount;
    final double[] nx, ny, nz;
    final boolean[] grounded, openEnd;
    // Basis functions: halves[b] = list of {segment, alpha, beta} with f(t) = alpha + beta t along +u.
    final int basisCount;
    final int[][] halfSeg;
    final double[][] halfAlpha, halfBeta;
    // Feed and loads as coefficient vectors over the basis.
    final double[] feedCoeff;
    final Complex feedVoltage;
    final double[][] loadCoeff;
    final Complex[] loadImpedance;

    private AntennaMesh(AntennaModel model, double designFrequency, List<double[]> segs, List<Integer> segWires,
            List<double[]> nodes, int[][] segNodes, double[] feedCoeff, double[][] loadCoeff,
            boolean[] grounded, boolean[] openEnd, List<int[]> halfSegList, List<double[]> halfAlphaList,
            List<double[]> halfBetaList) {
        this.model = model;
        this.designFrequency = designFrequency;
        segCount = segs.size();
        sx = new double[segCount]; sy = new double[segCount]; sz = new double[segCount];
        ex = new double[segCount]; ey = new double[segCount]; ez = new double[segCount];
        len = new double[segCount]; ux = new double[segCount]; uy = new double[segCount]; uz = new double[segCount];
        radius = new double[segCount]; resistivity = new double[segCount];
        segStart = new int[segCount]; segEnd = new int[segCount]; segWire = new int[segCount];
        for (int i = 0; i < segCount; i++) {
            double[] s = segs.get(i);
            sx[i] = s[0]; sy[i] = s[1]; sz[i] = s[2]; ex[i] = s[3]; ey[i] = s[4]; ez[i] = s[5];
            double dx = s[3] - s[0], dy = s[4] - s[1], dz = s[5] - s[2], l = Math.sqrt(dx * dx + dy * dy + dz * dz);
            len[i] = l; ux[i] = dx / l; uy[i] = dy / l; uz[i] = dz / l;
            radius[i] = s[6]; resistivity[i] = s[7];
            segStart[i] = segNodes[i][0]; segEnd[i] = segNodes[i][1]; segWire[i] = segWires.get(i);
        }
        nodeCount = nodes.size();
        nx = new double[nodeCount]; ny = new double[nodeCount]; nz = new double[nodeCount];
        for (int i = 0; i < nodeCount; i++) { nx[i] = nodes.get(i)[0]; ny[i] = nodes.get(i)[1]; nz[i] = nodes.get(i)[2]; }
        this.grounded = grounded;
        this.openEnd = openEnd;
        basisCount = halfSegList.size();
        halfSeg = halfSegList.toArray(new int[0][]);
        halfAlpha = halfAlphaList.toArray(new double[0][]);
        halfBeta = halfBetaList.toArray(new double[0][]);
        this.feedCoeff = feedCoeff;
        this.feedVoltage = model.feed().voltage();
        this.loadCoeff = loadCoeff;
        loadImpedance = new Complex[model.loads().size()];
        for (int i = 0; i < loadImpedance.length; i++) loadImpedance[i] = model.loads().get(i).impedance();
    }

    public AntennaModel model() { return model; }

    public double designFrequency() { return designFrequency; }

    public int segmentCount() { return segCount; }

    public int basisCount() { return basisCount; }

    public int nodeCount() { return nodeCount; }

    /** {x1, y1, z1, x2, y2, z2} of a segment. */
    public double[] segment(int i) { return new double[] {sx[i], sy[i], sz[i], ex[i], ey[i], ez[i]}; }

    public double segmentLength(int i) { return len[i]; }

    /** The model wire the segment came from (for merged runs, the wire containing its midpoint). */
    public int segmentWire(int i) { return segWire[i]; }

    /** Number of segments {@link #build} would create, or -1 if it would exceed {@link #MAX_SEGMENTS}. */
    public static int estimateSegments(AntennaModel model, double designFrequencyHz) {
        try {
            return build(model, designFrequencyHz).segCount;
        } catch (AntennaGeometryException e) {
            if (e instanceof SegmentCapException) return -1;
            throw e;
        }
    }

    /** Thrown when the geometry needs more than {@link #MAX_SEGMENTS} segments; use {@link HeuristicAntenna} instead. */
    public static final class SegmentCapException extends AntennaGeometryException {
        public SegmentCapException(String message) { super(message); }
    }

    /**
     * Discretises the model for solving at frequencies up to {@code designFrequencyHz}.
     *
     * @throws AntennaGeometryException for invalid geometry, a feed on an open end, or too many segments
     */
    public static AntennaMesh build(AntennaModel model, double designFrequencyHz) {
        if (!(designFrequencyHz > 0) || !Double.isFinite(designFrequencyHz)) throw new AntennaGeometryException("design frequency");
        List<Wire> wires = model.wires();
        boolean groundPlane = model.ground().present();

        // 1. Cluster wire endpoints into nodes.
        List<double[]> nodes = new ArrayList<>();
        int[][] wireNodes = new int[wires.size()][2];
        for (int i = 0; i < wires.size(); i++) {
            Wire w = wires.get(i);
            wireNodes[i][0] = nodeFor(nodes, w.x1(), w.y1(), w.z1());
            wireNodes[i][1] = nodeFor(nodes, w.x2(), w.y2(), w.z2());
            if (wireNodes[i][0] == wireNodes[i][1]) throw new AntennaGeometryException("wire " + i + " has zero length");
        }
        int modelNodes = nodes.size();
        int[] degree = new int[modelNodes];
        for (int[] wn : wireNodes) { degree[wn[0]]++; degree[wn[1]]++; }
        boolean[] modelGrounded = new boolean[modelNodes];
        for (int i = 0; i < modelNodes; i++) modelGrounded[i] = groundPlane && Math.abs(nodes.get(i)[2]) <= TOL;

        // 2. Merge straight runs of compatible auto-split wires.
        List<Run> runs = buildRuns(wires, wireNodes, degree, modelGrounded);

        // 3. Segment runs, honouring the cap.
        double lambda = C0 / designFrequencyHz;
        List<double[]> segs = null;
        List<Integer> segWires = null;
        List<int[]> segNodeList = null;
        List<double[]> meshNodes = null;
        for (double perLambda : new double[] {SEGMENTS_PER_WAVELENGTH, MIN_SEGMENTS_PER_WAVELENGTH}) {
            segs = new ArrayList<>();
            segWires = new ArrayList<>();
            segNodeList = new ArrayList<>();
            meshNodes = new ArrayList<>(nodes);
            for (Run run : runs) segmentRun(run, model, wires, lambda / perLambda, segs, segWires, segNodeList, meshNodes);
            if (segs.size() <= MAX_SEGMENTS) break;
        }
        if (segs.size() > MAX_SEGMENTS)
            throw new SegmentCapException("antenna needs " + segs.size() + " segments (cap " + MAX_SEGMENTS + ")");

        // 4. Drop nodes no segment uses (wire joints inside merged runs), then incidence and grounding.
        int[][] segNodes = segNodeList.toArray(new int[0][]);
        int[] remap = new int[meshNodes.size()];
        Arrays.fill(remap, -1);
        List<double[]> usedNodes = new ArrayList<>();
        for (int[] sn : segNodes) {
            for (int e = 0; e < 2; e++) {
                if (remap[sn[e]] < 0) {
                    remap[sn[e]] = usedNodes.size();
                    usedNodes.add(meshNodes.get(sn[e]));
                }
                sn[e] = remap[sn[e]];
            }
        }
        meshNodes = usedNodes;
        int nodeCount = meshNodes.size();
        List<List<int[]>> incident = new ArrayList<>(); // {segment, 0 = node is start / 1 = node is end}
        for (int i = 0; i < nodeCount; i++) incident.add(new ArrayList<>());
        for (int s = 0; s < segNodes.length; s++) {
            incident.get(segNodes[s][0]).add(new int[] {s, 0});
            incident.get(segNodes[s][1]).add(new int[] {s, 1});
        }
        boolean[] grounded = new boolean[nodeCount], openEnd = new boolean[nodeCount];
        for (int i = 0; i < nodeCount; i++) {
            grounded[i] = groundPlane && Math.abs(meshNodes.get(i)[2]) <= TOL;
            openEnd[i] = !grounded[i] && incident.get(i).size() == 1;
        }

        // 5. Basis functions. halfNode tracks which node each half belongs to, for feeds and loads.
        List<int[]> halfSegList = new ArrayList<>();
        List<double[]> halfAlphaList = new ArrayList<>(), halfBetaList = new ArrayList<>();
        List<int[]> halfNodeList = new ArrayList<>();
        List<boolean[]> halfAwayList = new ArrayList<>();
        for (int n = 0; n < nodeCount; n++) {
            List<int[]> inc = incident.get(n);
            if (grounded[n]) {
                for (int[] h : inc) addBasis(halfSegList, halfAlphaList, halfBetaList, halfNodeList, halfAwayList, n, new int[][] {h}, new boolean[] {true});
            } else if (inc.size() >= 2) {
                int[] ref = inc.get(0);
                for (int k = 1; k < inc.size(); k++)
                    addBasis(halfSegList, halfAlphaList, halfBetaList, halfNodeList, halfAwayList, n, new int[][] {ref, inc.get(k)}, new boolean[] {false, true});
            }
        }
        if (halfSegList.isEmpty()) throw new AntennaGeometryException("antenna has no current paths (needs at least two segments or a ground connection)");

        // 6. Feed and loads.
        double[] feedCoeff = portCoefficients(model.feed().wire(), model.feed().position(), wires, runs, segNodes, meshNodes,
                incident, halfSegList, halfNodeList, halfAwayList);
        boolean any = false;
        for (double c : feedCoeff) any |= c != 0;
        if (!any) throw new AntennaGeometryException("feed is on an open wire end");
        double[][] loadCoeff = new double[model.loads().size()][];
        for (int i = 0; i < loadCoeff.length; i++) {
            Load l = model.loads().get(i);
            loadCoeff[i] = portCoefficients(l.wire(), l.position(), wires, runs, segNodes, meshNodes, incident, halfSegList,
                    halfNodeList, halfAwayList);
        }

        List<double[]> segGeom = new ArrayList<>();
        for (double[] s : segs) segGeom.add(s);
        return new AntennaMesh(model, designFrequencyHz, segGeom, segWires, meshNodes, segNodes, feedCoeff, loadCoeff,
                grounded, openEnd, halfSegList, halfAlphaList, halfBetaList);
    }

    private static int nodeFor(List<double[]> nodes, double x, double y, double z) {
        for (int i = 0; i < nodes.size(); i++) {
            double[] n = nodes.get(i);
            if (Math.abs(n[0] - x) <= TOL && Math.abs(n[1] - y) <= TOL && Math.abs(n[2] - z) <= TOL) return i;
        }
        nodes.add(new double[] {x, y, z});
        return nodes.size() - 1;
    }

    /** A straight chain of wires: members in order, each possibly reversed relative to the run. */
    private record Run(int[] members, boolean[] reversed, double[] offsets, double length, int startNode, int endNode,
            double[] start, double[] dir, boolean explicit) {}

    private static List<Run> buildRuns(List<Wire> wires, int[][] wireNodes, int[] degree, boolean[] grounded) {
        int n = wires.size();
        boolean[] used = new boolean[n];
        List<Run> runs = new ArrayList<>();
        for (int i = 0; i < n; i++) {
            if (used[i]) continue;
            used[i] = true;
            // Chain as a deque of (wire, reversed); grow forward from end node and backward from start node.
            List<int[]> chain = new ArrayList<>();
            chain.add(new int[] {i, 0});
            int tail = wireNodes[i][1], head = wireNodes[i][0];
            for (int dirPass = 0; dirPass < 2; dirPass++) {
                while (true) {
                    int node = dirPass == 0 ? tail : head;
                    int[] edge = dirPass == 0 ? chain.get(chain.size() - 1) : chain.get(0);
                    int next = mergeCandidate(wires, wireNodes, degree, grounded, used, edge[0], node);
                    if (next < 0) break;
                    used[next] = true;
                    boolean nextStartsHere = wireNodes[next][0] == node;
                    if (dirPass == 0) {
                        chain.add(new int[] {next, nextStartsHere ? 0 : 1});
                        tail = nextStartsHere ? wireNodes[next][1] : wireNodes[next][0];
                    } else {
                        chain.add(0, new int[] {next, nextStartsHere ? 1 : 0});
                        head = nextStartsHere ? wireNodes[next][1] : wireNodes[next][0];
                    }
                }
            }
            int m = chain.size();
            int[] members = new int[m];
            boolean[] reversed = new boolean[m];
            double[] offsets = new double[m + 1];
            for (int k = 0; k < m; k++) {
                members[k] = chain.get(k)[0];
                reversed[k] = chain.get(k)[1] == 1;
                offsets[k + 1] = offsets[k] + wires.get(members[k]).length();
            }
            Wire first = wires.get(members[0]);
            double[] start = reversed[0] ? new double[] {first.x2(), first.y2(), first.z2()} : new double[] {first.x1(), first.y1(), first.z1()};
            Wire last = wires.get(members[m - 1]);
            double[] end = reversed[m - 1] ? new double[] {last.x1(), last.y1(), last.z1()} : new double[] {last.x2(), last.y2(), last.z2()};
            double L = offsets[m];
            double[] dir = {(end[0] - start[0]) / L, (end[1] - start[1]) / L, (end[2] - start[2]) / L};
            runs.add(new Run(members, reversed, offsets, L, head, tail, start, dir, first.segments() != Wire.AUTO));
        }
        return runs;
    }

    private static int mergeCandidate(List<Wire> wires, int[][] wireNodes, int[] degree, boolean[] grounded, boolean[] used,
            int wire, int node) {
        if (degree[node] != 2 || grounded[node]) return -1;
        Wire a = wires.get(wire);
        if (a.segments() != Wire.AUTO) return -1;
        for (int j = 0; j < wires.size(); j++) {
            if (used[j] || j == wire || (wireNodes[j][0] != node && wireNodes[j][1] != node)) continue;
            Wire b = wires.get(j);
            if (b.segments() != Wire.AUTO || b.radius() != a.radius() || b.resistivity() != a.resistivity()) return -1;
            double[] da = away(a, wireNodes[wire][0] == node), db = away(b, wireNodes[j][0] == node);
            double dot = da[0] * db[0] + da[1] * db[1] + da[2] * db[2];
            return dot < -1 + 1e-9 ? j : -1;
        }
        return -1;
    }

    /** Unit vector pointing from the given end of the wire into the wire. */
    private static double[] away(Wire w, boolean fromStart) {
        double l = w.length(), s = fromStart ? 1 : -1;
        return new double[] {s * (w.x2() - w.x1()) / l, s * (w.y2() - w.y1()) / l, s * (w.z2() - w.z1()) / l};
    }

    /** Distance along the run of a position on one of its member wires, and +1/−1 for the wire's sense along the run. */
    private static double[] runPosition(Run run, int wire, double position, List<Wire> wires) {
        for (int k = 0; k < run.members.length; k++) {
            if (run.members[k] != wire) continue;
            double l = wires.get(wire).length();
            return new double[] {run.offsets[k] + (run.reversed[k] ? 1 - position : position) * l, run.reversed[k] ? -1 : 1};
        }
        return null;
    }

    private static void segmentRun(Run run, AntennaModel model, List<Wire> wires, double maxSeg, List<double[]> segs,
            List<Integer> segWires, List<int[]> segNodes, List<double[]> nodes) {
        // Node positions along the run.
        List<Double> cuts = new ArrayList<>();
        if (run.explicit) {
            // An explicit run is a single wire (explicit wires never merge).
            int count = wires.get(run.members[0]).segments();
            for (int i = 0; i <= count; i++) cuts.add(run.length * i / count);
        } else {
            List<Double> breaks = new ArrayList<>();
            breaks.add(0.0);
            breaks.add(run.length);
            List<double[]> ports = new ArrayList<>();
            ports.add(new double[] {model.feed().wire(), model.feed().position()});
            for (Load l : model.loads()) ports.add(new double[] {l.wire(), l.position()});
            for (double[] p : ports) {
                double[] rp = runPosition(run, (int) p[0], p[1], wires);
                if (rp != null) breaks.add(rp[0]);
            }
            breaks.sort(Double::compare);
            double a = wires.get(run.members[0]).radius();
            cuts.add(0.0);
            for (int i = 1; i < breaks.size(); i++) {
                double from = cuts.get(cuts.size() - 1), to = breaks.get(i), piece = to - from;
                if (piece <= TOL * 10) continue;
                int count = (int) Math.ceil(piece / maxSeg - 1e-9);
                count = Math.max(1, Math.min(count, (int) Math.floor(piece / (4 * a))));
                for (int c = 1; c <= count; c++) cuts.add(from + piece * c / count);
            }
            // The final cut must be exactly the run end.
            cuts.set(cuts.size() - 1, run.length);
        }
        int prev = run.startNode;
        for (int i = 1; i < cuts.size(); i++) {
            int node;
            if (i == cuts.size() - 1) node = run.endNode;
            else {
                double d = cuts.get(i);
                nodes.add(new double[] {run.start[0] + run.dir[0] * d, run.start[1] + run.dir[1] * d, run.start[2] + run.dir[2] * d});
                node = nodes.size() - 1;
            }
            double[] p = nodes.get(prev), q = nodes.get(node);
            double mid = (cuts.get(i - 1) + cuts.get(i)) / 2;
            int member = 0;
            while (member < run.members.length - 1 && run.offsets[member + 1] < mid) member++;
            Wire w = wires.get(run.members[member]);
            segs.add(new double[] {p[0], p[1], p[2], q[0], q[1], q[2], w.radius(), w.resistivity()});
            segWires.add(run.members[member]);
            segNodes.add(new int[] {prev, node});
            prev = node;
        }
    }

    private static void addBasis(List<int[]> segList, List<double[]> alphaList, List<double[]> betaList,
            List<int[]> nodeList, List<boolean[]> awayList, int node, int[][] halves, boolean[] away) {
        int k = halves.length;
        int[] seg = new int[k];
        double[] alpha = new double[k], beta = new double[k];
        int[] nodes = new int[k];
        for (int i = 0; i < k; i++) {
            seg[i] = halves[i][0];
            boolean atStart = halves[i][1] == 0;
            // Current flowing away from the node: 1 - t at a segment's start (+u), -t at its end (-u).
            // Toward the node is the negation.
            double a = atStart ? 1 : 0, b = -1;
            if (!away[i]) { a = -a; b = -b; }
            alpha[i] = a;
            beta[i] = b;
            nodes[i] = node;
        }
        segList.add(seg);
        alphaList.add(alpha);
        betaList.add(beta);
        nodeList.add(nodes);
        awayList.add(away.clone());
    }

    /**
     * Coefficient of each basis function in the current flowing through a series port (feed or load) at the
     * node nearest the given wire position, in the wire's first-to-second endpoint sense.
     */
    private static double[] portCoefficients(int wire, double position, List<Wire> wires, List<Run> runs, int[][] segNodes,
            List<double[]> nodes, List<List<int[]>> incident, List<int[]> halfSegList, List<int[]> halfNodeList,
            List<boolean[]> halfAwayList) {
        Wire w = wires.get(wire);
        double px = w.x1() + (w.x2() - w.x1()) * position, py = w.y1() + (w.y2() - w.y1()) * position,
                pz = w.z1() + (w.z2() - w.z1()) * position;
        double dx = w.x2() - w.x1(), dy = w.y2() - w.y1(), dz = w.z2() - w.z1();
        // Nearest mesh node lying on this wire, and the segment of this wire beside it.
        int bestNode = -1, bestSeg = -1;
        double best = Double.MAX_VALUE, sense = 0;
        for (int s = 0; s < segNodes.length; s++) {
            double[] a = nodes.get(segNodes[s][0]), b = nodes.get(segNodes[s][1]);
            if (!onLine(w, a, false) || !onLine(w, b, false)) continue;
            for (int e = 0; e < 2; e++) {
                double[] n = e == 0 ? a : b;
                if (!onLine(w, n, true)) continue;
                double d = Math.sqrt((n[0] - px) * (n[0] - px) + (n[1] - py) * (n[1] - py) + (n[2] - pz) * (n[2] - pz));
                // Prefer the segment on the far side of the node (towards the wire's second endpoint) on ties.
                double[] o = e == 0 ? b : a;
                double along = (o[0] - n[0]) * dx + (o[1] - n[1]) * dy + (o[2] - n[2]) * dz;
                double score = d - (along > 0 ? 1e-12 : 0);
                if (score < best) {
                    best = score;
                    bestNode = segNodes[s][e];
                    bestSeg = s;
                    sense = along > 0 ? 1 : -1;
                }
            }
        }
        double[] coeff = new double[halfSegList.size()];
        if (bestNode < 0) return coeff;
        for (int m = 0; m < coeff.length; m++) {
            int[] segs = halfSegList.get(m), hn = halfNodeList.get(m);
            boolean[] away = halfAwayList.get(m);
            for (int h = 0; h < segs.length; h++) {
                if (segs[h] != bestSeg || hn[h] != bestNode) continue;
                // "away" halves carry +1 out of the node along the segment; sense maps that onto the wire direction.
                coeff[m] += (away[h] ? 1 : -1) * sense;
            }
        }
        return coeff;
    }

    /** Whether p lies on the wire's line, and (if {@code within}) between its endpoints. */
    private static boolean onLine(Wire w, double[] p, boolean within) {
        double dx = w.x2() - w.x1(), dy = w.y2() - w.y1(), dz = w.z2() - w.z1(), l2 = dx * dx + dy * dy + dz * dz;
        double t = ((p[0] - w.x1()) * dx + (p[1] - w.y1()) * dy + (p[2] - w.z1()) * dz) / l2;
        if (within && (t < -1e-9 || t > 1 + 1e-9)) return false;
        double cx = w.x1() + dx * t - p[0], cy = w.y1() + dy * t - p[1], cz = w.z1() + dz * t - p[2];
        return cx * cx + cy * cy + cz * cz <= TOL * TOL * 4;
    }

    @Override
    public String toString() {
        return "AntennaMesh[" + segCount + " segments, " + basisCount + " unknowns, " + nodeCount + " nodes, f="
                + designFrequency + " Hz, ground=" + model.ground().type() + "]";
    }
}
