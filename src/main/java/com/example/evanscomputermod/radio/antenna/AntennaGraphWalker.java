package com.example.evanscomputermod.radio.antenna;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioContent;
import com.example.evanscomputermod.radio.antenna.graph.AntennaGraph;
import com.example.evanscomputermod.radio.antenna.graph.ConductorSpec;
import com.example.evanscomputermod.radio.antenna.graph.RfDefaults;
import com.example.evanscomputermod.radio.antenna.solver.Ground;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.conductor.RadioConductors;
import com.example.evanscomputermod.sensor.SensorSable;
import com.example.evanscomputermod.sensor.wire.BaseWireEntity;
import com.example.evanscomputermod.sensor.wire.BlockWireEndpoint;
import com.example.evanscomputermod.sensor.wire.BlockWireEntity;
import com.example.evanscomputermod.sensor.wire.BlockWireEntityEndpoint;
import com.example.evanscomputermod.sensor.wire.IWireEndpoint;
import com.example.evanscomputermod.sensor.wire.JunctionWireEndpoint;
import com.example.evanscomputermod.sensor.wire.WireConnections;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.tags.BiomeTags;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/**
 * Extracts the {@link AntennaGraph} hanging off a feed point (main thread).
 *
 * <ul>
 *   <li>Block wires: 1 m segments along block centres, each split at the shared face into two half
 *       edges carrying their own block's spec (so mixed tiers, oxidation and water are per block).</li>
 *   <li>Blocks in {@code #evanscomputermod:rf_conductors} touching the graph join it (and detune it),
 *       up to {@link #MAX_FOREIGN} of them.</li>
 *   <li>Insulators and other feed points break the graph; the conductor touching one is recorded as
 *       insulated (its open end is limited by the insulator's voltage rating).</li>
 *   <li>Fine Wire runs attached to the feed point's lugs follow junctions and wire-to-wire joins;
 *       every straight run becomes an edge (the mesh splits it at λ/20).</li>
 *   <li>Never crosses a Sable sub-level boundary; ground is searched within the feed's own
 *       sub-level, so an airship without hull below its antenna is in free space.</li>
 * </ul>
 */
public final class AntennaGraphWalker {
    public static final int MAX_BLOCKS = 1024;
    public static final int MAX_FOREIGN = 64;
    public static final int MAX_FINE_WIRES = 256;

    private AntennaGraphWalker() {}

    /** The graph plus the blocks it covers (for the invalidation index). */
    public record Walk(AntennaGraph graph, Set<BlockPos> blocks) {}

    /** Walks from {@code feed}; null if there is no feed point there. */
    @Nullable
    public static Walk walk(Level level, BlockPos feed) {
        BlockState fs = level.getBlockState(feed);
        if (!(fs.getBlock() instanceof FeedPointBlock)) return null;
        Direction neg = FeedPointBlock.armSide(fs, false), pos = FeedPointBlock.armSide(fs, true);
        Vec3 c = Vec3.atCenterOf(feed);
        // With block wire on neither lug the feed is "compact": its lugs count as electrically short, the
        // gap is 1/8 m and Fine Wire elements are measured from the lug tips (so VHF/UHF dipoles fit).
        boolean compact = !blockArm(level, feed, fs, neg) && !blockArm(level, feed, fs, pos);
        double lug = compact ? 1 / 16.0 : 0.5;
        AntennaGraph.Point fa = point(c.add(Vec3.atLowerCornerOf(neg.getNormal()).scale(lug)));
        AntennaGraph.Point fb = point(c.add(Vec3.atLowerCornerOf(pos.getNormal()).scale(lug)));
        AntennaGraph.Builder b = AntennaGraph.builder(fa, fb)
                .feed(RadioConductors.spec(fs), "feed point at " + at(feed), RadioConductors.voltageRating(fs));

        Set<BlockPos> visited = new LinkedHashSet<>();
        visited.add(feed);
        ArrayDeque<BlockPos> queue = new ArrayDeque<>();
        Set<Long> edgeKeys = new HashSet<>();
        int foreign = 0;
        boolean truncated = false;
        for (Direction side : new Direction[] {neg, pos}) {
            if (!fs.getValue(ConductorBlock.property(side))) continue;
            BlockPos n = feed.relative(side);
            BlockState ns = level.getBlockState(n);
            if (!RadioConductors.conducts(ns)) continue;
            b.edge(side == neg ? fa : fb, point(Vec3.atCenterOf(n)), RadioConductors.spec(ns), label(ns, n));
            if (visited.add(n)) queue.add(n);
            if (!(ns.getBlock() instanceof ConductorBlock)) foreign++;
        }
        while (!queue.isEmpty()) {
            if (visited.size() >= MAX_BLOCKS) { truncated = true; break; }
            BlockPos cur = queue.poll();
            BlockState cs = level.getBlockState(cur);
            ConductorSpec spec = RadioConductors.spec(cs);
            Vec3 cc = Vec3.atCenterOf(cur);
            for (Direction d : Direction.values()) {
                BlockPos n = cur.relative(d);
                if (n.equals(feed)) continue;   // the arm we came in on (or a conductor beside the coax port)
                BlockState ns = level.getBlockState(n);
                boolean joined = joined(level, cur, cs, d, n, ns);
                if (!joined) continue;
                if (ns.getBlock() instanceof ConductorBlock nc && nc.role() != ConductorBlock.Role.CONDUCTOR) {
                    if (nc.role() == ConductorBlock.Role.INSULATOR || nc.role() == ConductorBlock.Role.FEED)
                        b.insulated(point(cc), label(ns, n), RadioConductors.voltageRating(ns));
                    continue;
                }
                if (!SensorSable.sameSubLevel(level, cc, Vec3.atCenterOf(n))) continue;
                boolean isForeign = !(ns.getBlock() instanceof ConductorBlock);
                if (isForeign && !visited.contains(n) && foreign >= MAX_FOREIGN) { truncated = true; continue; }
                long key = Math.min(cur.asLong(), n.asLong()) * 31 + Math.max(cur.asLong(), n.asLong());
                if (edgeKeys.add(key)) {
                    AntennaGraph.Point face = point(cc.add(Vec3.atLowerCornerOf(d.getNormal()).scale(0.5)));
                    b.edge(point(cc), face, spec, label(cs, cur));
                    b.edge(face, point(Vec3.atCenterOf(n)), RadioConductors.spec(ns), label(ns, n));
                }
                if (visited.add(n)) {
                    queue.add(n);
                    if (isForeign) foreign++;
                }
            }
        }

        fineWires(level, feed, fa, fb, compact ? Vec3.atLowerCornerOf(neg.getNormal()).scale(lug - 0.5) : Vec3.ZERO,
                compact ? Vec3.atLowerCornerOf(pos.getNormal()).scale(lug - 0.5) : Vec3.ZERO, b);

        // Ground: the first solid (or water) block below the feed, inside the feed's own structure.
        Ground ground = Ground.NONE;
        String groundName = "free space";
        double groundY = Double.NaN;
        Vec3 fc = Vec3.atCenterOf(feed);
        for (BlockPos p = feed.below(); p.getY() >= level.getMinBuildHeight(); p = p.below()) {
            if (visited.contains(p)) continue;
            if (!SensorSable.sameSubLevel(level, fc, Vec3.atCenterOf(p))) break;
            BlockState s = level.getBlockState(p);
            FluidState fl = s.getFluidState();
            if (!fl.isEmpty() && fl.is(net.minecraft.tags.FluidTags.WATER) && !(s.getBlock() instanceof ConductorBlock)) {
                boolean sea = level.getBiome(p).is(BiomeTags.IS_OCEAN);
                ground = sea ? Ground.SEA_WATER : Ground.FRESH_WATER;
                groundName = sea ? "sea water" : "fresh water";
                groundY = p.getY() + 1;
                break;
            }
            if (s.blocksMotion() && !(s.getBlock() instanceof ConductorBlock)) {
                if (s.is(RadioContent.RF_GOOD_GROUND)) {
                    boolean metal = s.is(RadioContent.RF_CONDUCTORS);
                    ground = metal ? Ground.PERFECT : Ground.WET;
                    groundName = metal ? "metal" : "wet ground";
                } else if (s.is(BlockTags.SAND) || s.is(net.minecraft.world.level.block.Blocks.SANDSTONE)) {
                    ground = Ground.POOR;
                    groundName = "dry sand";
                } else {
                    ground = Ground.AVERAGE;
                    groundName = "average soil";
                }
                groundY = p.getY() + 1;
                break;
            }
        }
        if (!Double.isNaN(groundY))
            groundName += String.format(java.util.Locale.ROOT, ", %.0f m below", feed.getY() + 0.5 - groundY);
        b.ground(groundY, ground, groundName);

        // A vertical feed sitting on the ground with nothing on its lower lug feeds a monopole.
        boolean lowerFree = !fs.getValue(ConductorBlock.property(Direction.DOWN))
                && WireConnections.get(level, new BlockWireEndpoint(feed, 0)).isEmpty();
        boolean monopole = fs.getValue(FeedPointBlock.AXIS) == Direction.Axis.Y && lowerFree
                && !Double.isNaN(groundY) && groundY == feed.getY();
        b.monopole(monopole);
        if (monopole) b.feedA(new AntennaGraph.Point(c.x, feed.getY(), c.z));
        b.blocks(visited.size(), truncated);
        return new Walk(b.build(), visited);
    }

    /** Whether {@code cur} and its neighbour {@code n} (on side {@code d}) are electrically or mechanically joined. */
    private static boolean joined(Level level, BlockPos cur, BlockState cs, Direction d, BlockPos n, BlockState ns) {
        if (cs.getBlock() instanceof ConductorBlock) return cs.getValue(ConductorBlock.property(d));
        // A foreign conductor (iron bars...) joins conductor blocks that connect to it, and other foreign conductors.
        if (ns.getBlock() instanceof ConductorBlock) return ns.getValue(ConductorBlock.property(d.getOpposite()));
        return ns.is(RadioContent.RF_CONDUCTORS);
    }

    // ------------------------------------------------------------ fine wire

    private static boolean blockArm(Level level, BlockPos feed, BlockState fs, Direction side) {
        return fs.getValue(ConductorBlock.property(side)) && RadioConductors.conducts(level.getBlockState(feed.relative(side)));
    }

    /** Fine Wire runs from the two lugs; {@code shiftA/B} move a compact feed's runs onto its short lugs. */
    private static void fineWires(Level level, BlockPos feed, AntennaGraph.Point fa, AntennaGraph.Point fb, Vec3 shiftA, Vec3 shiftB,
                                  AntennaGraph.Builder b) {
        Set<BaseWireEntity> seen = new HashSet<>();
        for (int t = 0; t < 2; t++) {
            AntennaGraph.Point lug = t == 0 ? fa : fb;
            Vec3 shift = t == 0 ? shiftA : shiftB;
            BlockWireEndpoint start = new BlockWireEndpoint(feed, t);
            ArrayDeque<Object[]> todo = new ArrayDeque<>();
            for (BaseWireEntity w : WireConnections.get(level, start)) todo.add(new Object[] {w, start, lug});
            while (!todo.isEmpty() && seen.size() < MAX_FINE_WIRES) {
                Object[] job = todo.poll();
                if (!(job[0] instanceof BlockWireEntity w) || !seen.add(w)) continue;
                IWireEndpoint from = (IWireEndpoint) job[1];
                List<Vec3> pts = new ArrayList<>();
                for (Vec3 v : polyline(w)) pts.add(v.add(shift));
                AntennaGraph.Point prev = (AntennaGraph.Point) job[2];
                // Walk the wire away from where we came in: by endpoint identity, else by distance.
                boolean reversed = from != null ? from.equals(w.getEndpoint2())
                        : point(pts.get(pts.size() - 1)).distance(prev) < point(pts.get(0)).distance(prev);
                if (reversed) java.util.Collections.reverse(pts);
                for (Vec3 v : pts) {
                    AntennaGraph.Point p = point(v);
                    b.edge(prev, p, RfDefaults.FINE_WIRE, "fine wire", true);
                    prev = p;
                }
                IWireEndpoint other = reversed ? w.getEndpoint1() : w.getEndpoint2();
                if (other instanceof JunctionWireEndpoint j) {
                    for (BaseWireEntity next : WireConnections.get(level, j)) if (!seen.contains(next)) todo.add(new Object[] {next, j, prev});
                } else if (other instanceof BlockWireEntityEndpoint bwe) {
                    BlockWireEntity next = bwe.getEntity(level);
                    if (next != null && !seen.contains(next)) todo.add(new Object[] {next, null, prev});
                }
                // A BlockWireEndpoint on another host (or this feed's other lug) ends the run there.
            }
        }
    }

    private static List<Vec3> polyline(BlockWireEntity w) {
        List<Vec3> pts = new ArrayList<>();
        Vec3 p = w.position();
        pts.add(p);
        for (BlockWireEntity.Point s : w.segments) {
            p = p.add(s.vector());
            pts.add(p);
        }
        return pts;
    }

    // ------------------------------------------------------------ helpers

    private static AntennaGraph.Point point(Vec3 v) {
        return new AntennaGraph.Point(v.x, v.y, v.z);
    }

    static String at(BlockPos p) {
        return "(" + p.getX() + ", " + p.getY() + ", " + p.getZ() + ")";
    }

    private static String label(BlockState s, BlockPos p) {
        return RadioConductors.spec(s).name() + " at " + at(p);
    }
}
//?}
