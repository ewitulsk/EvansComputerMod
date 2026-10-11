package com.example.evanscomputermod.radio.antenna.graph;

import com.example.evanscomputermod.radio.antenna.solver.Ground;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

/**
 * The conductor graph hanging off one feed point, in <b>Minecraft
 * coordinates</b> (metres, y up), extracted from the world by the walker and
 * turned into a solver {@code AntennaModel} by {@link AntennaModelBuilder}.
 * Pure data, so it can be built in unit tests and handed to a worker thread.
 *
 * <ul>
 *   <li>The feed is a short conductor from {@code feedA} to {@code feedB} (the feed point block's two
 *       faces along its axis) with the voltage source in its middle. For a {@link #monopole} the
 *       feed point sits on the ground and {@code feedA} lies on the ground surface.</li>
 *   <li>Each {@link Edge} is a straight conductor: half a block wire (centre to face), a fine-wire
 *       run, or a touching conductive block. Edges join where their endpoints coincide.</li>
 *   <li>{@link #insulatedPoints} are conductor nodes held by an insulator (or another feed point):
 *       an open end there is limited by the insulator's voltage rating, not corona.</li>
 *   <li>{@link #groundY}: the ground surface height (top of the first solid block below the feed),
 *       NaN for free space (nothing below, e.g. an airship).</li>
 * </ul>
 */
public final class AntennaGraph {

    public record Point(double x, double y, double z) {
        public double distance(Point o) {
            double dx = x - o.x, dy = y - o.y, dz = z - o.z;
            return Math.sqrt(dx * dx + dy * dy + dz * dz);
        }

        /** Grid key (1/1024 m) for joining coincident points. */
        long key() {
            long kx = Math.round(x * 1024), ky = Math.round(y * 1024), kz = Math.round(z * 1024);
            return (kx * 73_856_093L) ^ (ky * 19_349_663L) ^ (kz * 83_492_791L);
        }

        boolean near(Point o) { return distance(o) < 1e-3; }
    }

    /** A straight conductor. {@code label} names it in reports ("copper wire at (3, 70, -2)"). */
    public record Edge(Point a, Point b, ConductorSpec spec, String label, boolean fineWire) {
        public Edge {
            Objects.requireNonNull(a);
            Objects.requireNonNull(b);
            Objects.requireNonNull(spec);
        }

        public double length() { return a.distance(b); }
    }

    /**
     * A conductor point {@code at} held by an insulator rated for
     * {@code voltageRating} volts peak; {@code where} is the insulator itself.
     */
    public record Insulated(Point at, String label, double voltageRating, Point where) {}

    public final Point feedA, feedB;
    public final ConductorSpec feedSpec;
    public final String feedLabel;
    public final double feedVoltageRating;
    public final boolean monopole;
    public final List<Edge> edges;
    public final List<Insulated> insulatedPoints;
    public final double groundY;
    public final Ground ground;
    public final String groundName;
    /** Number of blocks the walker visited (for reports and caps). */
    public final int blockCount;
    /** True if the walker stopped at its block cap (the graph is incomplete). */
    public final boolean truncated;

    private AntennaGraph(Builder b) {
        feedA = b.feedA;
        feedB = b.feedB;
        feedSpec = b.feedSpec;
        feedLabel = b.feedLabel;
        feedVoltageRating = b.feedVoltageRating;
        monopole = b.monopole;
        edges = List.copyOf(b.edges);
        insulatedPoints = List.copyOf(b.insulated);
        groundY = b.groundY;
        ground = b.ground;
        groundName = b.groundName;
        blockCount = b.blockCount;
        truncated = b.truncated;
        truncationNote = b.truncationNote;
    }

    /**
     * Why the walk stopped early (which limit was hit), or "" when the whole antenna was walked.
     * Shown by the analyzer and the {@code antenna} program: the results then cover only part of it.
     */
    public final String truncationNote;

    public static Builder builder(Point feedA, Point feedB) { return new Builder(feedA, feedB); }

    public boolean hasGround() { return !Double.isNaN(groundY) && ground.present(); }

    /** True when nothing conducts beyond the feed point itself: "no antenna". */
    public boolean empty() { return edges.isEmpty(); }

    public boolean hasFineWire() {
        for (Edge e : edges) if (e.fineWire) return true;
        return false;
    }

    /** Total conductor length including the feed gap, metres. */
    public double totalLength() {
        double sum = feedA.distance(feedB);
        for (Edge e : edges) sum += e.length();
        return sum;
    }

    /**
     * Longest tip-to-tip path through the feed (the electrical length of a
     * dipole), metres. A monopole counts its image: twice its height.
     */
    public double pathLength() {
        Map<Long, List<Integer>> at = new HashMap<>();
        List<Point> nodes = new ArrayList<>();
        int[][] ends = new int[edges.size()][2];
        for (int i = 0; i < edges.size(); i++) {
            ends[i][0] = node(at, nodes, edges.get(i).a());
            ends[i][1] = node(at, nodes, edges.get(i).b());
        }
        int a = node(at, nodes, feedA), b = node(at, nodes, feedB);
        double[] da = farthest(nodes.size(), ends, a), db = farthest(nodes.size(), ends, b);
        double ma = 0, mb = 0;
        for (int i = 0; i < nodes.size(); i++) {
            if (Double.isFinite(da[i]) && !Double.isFinite(db[i])) ma = Math.max(ma, da[i]);
            if (Double.isFinite(db[i]) && !Double.isFinite(da[i])) mb = Math.max(mb, db[i]);
            if (Double.isFinite(da[i]) && Double.isFinite(db[i])) {   // a loop: both sides reach it
                ma = Math.max(ma, da[i]);
                mb = Math.max(mb, db[i]);
            }
        }
        double len = ma + mb + feedA.distance(feedB);
        return monopole ? 2 * len : len;
    }

    private double[] farthest(int n, int[][] ends, int start) {
        // Dijkstra over a small graph (O(n·m) is fine at the block cap).
        double[] d = new double[n];
        java.util.Arrays.fill(d, Double.POSITIVE_INFINITY);
        d[start] = 0;
        boolean changed = true;
        for (int iter = 0; iter < n && changed; iter++) {
            changed = false;
            for (int i = 0; i < ends.length; i++) {
                double l = edges.get(i).length();
                int u = ends[i][0], v = ends[i][1];
                if (d[u] + l < d[v] - 1e-9) { d[v] = d[u] + l; changed = true; }
                if (d[v] + l < d[u] - 1e-9) { d[u] = d[v] + l; changed = true; }
            }
        }
        return d;
    }

    private static int node(Map<Long, List<Integer>> at, List<Point> nodes, Point p) {
        List<Integer> bucket = at.computeIfAbsent(p.key(), k -> new ArrayList<>());
        for (int i : bucket) if (nodes.get(i).near(p)) return i;
        nodes.add(p);
        bucket.add(nodes.size() - 1);
        return nodes.size() - 1;
    }

    /**
     * A structural fingerprint: equal graphs (same geometry, specs, ground
     * and insulators) give equal fingerprints, so a block update next to an
     * antenna that changes nothing electrical doesn't trigger a re-solve.
     */
    public String fingerprint() {
        StringBuilder sb = new StringBuilder();
        sb.append(feedA).append(feedB).append(feedSpec).append(monopole).append(groundY).append(ground).append('|');
        for (Edge e : edges) sb.append(e.a()).append(e.b()).append(e.spec()).append(';');
        for (Insulated i : insulatedPoints) sb.append(i.at()).append(i.voltageRating()).append(';');
        return Integer.toHexString(sb.toString().hashCode()) + ":" + sb.length();
    }

    public static final class Builder {
        private Point feedA;
        private final Point feedB;
        private ConductorSpec feedSpec = RfDefaults.FEED_POINT;
        private String feedLabel = "feed point";
        private double feedVoltageRating = RfDefaults.FEED_POINT_VOLTS;
        private boolean monopole;
        private final List<Edge> edges = new ArrayList<>();
        private final List<Insulated> insulated = new ArrayList<>();
        private double groundY = Double.NaN;
        private Ground ground = Ground.NONE;
        private String groundName = "free space";
        private int blockCount;
        private boolean truncated;
        private String truncationNote = "";

        private Builder(Point feedA, Point feedB) {
            this.feedA = feedA;
            this.feedB = feedB;
        }

        public Builder feed(ConductorSpec spec, String label, double voltageRating) {
            feedSpec = spec;
            feedLabel = label;
            feedVoltageRating = voltageRating;
            return this;
        }

        public Builder monopole(boolean m) { monopole = m; return this; }

        /** Moves the feed's first terminal (a monopole's base goes down to the ground surface). */
        public Builder feedA(Point p) { feedA = p; return this; }

        public Builder edge(Point a, Point b, ConductorSpec spec, String label) {
            return edge(a, b, spec, label, false);
        }

        public Builder edge(Point a, Point b, ConductorSpec spec, String label, boolean fineWire) {
            if (a.distance(b) > 1e-4) edges.add(new Edge(a, b, spec, label, fineWire));
            return this;
        }

        public Builder insulated(Point at, String label, double voltageRating) {
            return insulated(at, label, voltageRating, at);
        }

        public Builder insulated(Point at, String label, double voltageRating, Point where) {
            insulated.add(new Insulated(at, label, voltageRating, where));
            return this;
        }

        /** Ground surface at height {@code y} (NaN or {@link Ground#NONE} for free space). */
        public Builder ground(double y, Ground g, String name) {
            groundY = y;
            ground = g == null ? Ground.NONE : g;
            groundName = name;
            return this;
        }

        public Builder blocks(int count, boolean truncated) {
            blockCount = count;
            this.truncated = truncated;
            return this;
        }

        /** The walk hit a limit: {@code note} says which (it marks the graph truncated). */
        public Builder truncated(String note) {
            truncated = true;
            truncationNote = note == null ? "" : note;
            return this;
        }

        public int edgeCount() { return edges.size(); }

        public AntennaGraph build() { return new AntennaGraph(this); }
    }

    /**
     * A straight run of {@code blocks} block wires from a feed point face
     * along one axis, as the walker would extract it: centre-to-face half
     * edges per block. For tests and scenario previews.
     */
    public static void blockRun(Builder b, Point face, double dx, double dy, double dz, int blocks, ConductorSpec spec) {
        Point prev = face;
        for (int i = 0; i < blocks; i++) {
            Point centre = new Point(face.x + dx * (i + 0.5), face.y + dy * (i + 0.5), face.z + dz * (i + 0.5));
            Point far = new Point(face.x + dx * (i + 1), face.y + dy * (i + 1), face.z + dz * (i + 1));
            String label = spec.name() + " at (block " + (i + 1) + ")";
            b.edge(prev, centre, spec, label);
            if (i < blocks - 1) b.edge(centre, far, spec, label);
            prev = far;
        }
    }
}
