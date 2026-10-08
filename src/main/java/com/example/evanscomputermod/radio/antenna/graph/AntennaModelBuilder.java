package com.example.evanscomputermod.radio.antenna.graph;

import com.example.evanscomputermod.radio.antenna.solver.AntennaModel;
import com.example.evanscomputermod.radio.antenna.solver.Feed;
import com.example.evanscomputermod.radio.antenna.solver.Ground;
import com.example.evanscomputermod.radio.antenna.solver.Wire;

import java.util.ArrayList;
import java.util.List;

/**
 * Turns an {@link AntennaGraph} (Minecraft frame: x east, y up, z south)
 * into a solver {@link AntennaModel} (z up, ground plane at z = 0).
 *
 * <p>The mapping is right-handed: solver (x, y, z) = (east, north, up) =
 * (mcX − fx, −(mcZ − fz), mcY − ground). Coordinates are taken relative to
 * the feed so the numbers stay small. Wire 0 is always the feed gap; wire
 * {@code i + 1} is graph edge {@code i}.
 */
public final class AntennaModelBuilder {
    private AntennaModelBuilder() {}

    /** The model plus what each wire is, for naming the weakest link. */
    public record Built(AntennaModel model, List<ConductorSpec> specs, List<String> labels, double zOffset,
            double originX, double originZ) {}

    public static Built build(AntennaGraph g) {
        double ox = (g.feedA.x() + g.feedB.x()) / 2, oz = (g.feedA.z() + g.feedB.z()) / 2;
        double minY = Math.min(g.feedA.y(), g.feedB.y());
        for (AntennaGraph.Edge e : g.edges) minY = Math.min(minY, Math.min(e.a().y(), e.b().y()));

        Ground ground = Ground.NONE;
        double zRef = (g.feedA.y() + g.feedB.y()) / 2;
        boolean monopole = false;
        if (g.hasGround()) {
            ground = g.ground;
            if (g.monopole && Math.abs(Math.min(g.feedA.y(), g.feedB.y()) - g.groundY) < 1e-6 && minY >= g.groundY - 1e-6) {
                zRef = g.groundY;
                monopole = true;
            } else {
                // Everything must stay above the plane; an antenna hanging below the feed's ground (off a
                // cliff edge, say) lowers the plane to just under its lowest point.
                zRef = Math.min(g.groundY, minY - 0.5);
            }
        }

        List<Wire> wires = new ArrayList<>();
        List<ConductorSpec> specs = new ArrayList<>();
        List<String> labels = new ArrayList<>();
        AntennaGraph.Point fa = g.feedA, fb = g.feedB;
        if (monopole && fb.y() < fa.y()) { AntennaGraph.Point t = fa; fa = fb; fb = t; }   // base first
        wires.add(wire(fa, fb, g.feedSpec, ox, oz, zRef));
        specs.add(g.feedSpec);
        labels.add(g.feedLabel);
        for (AntennaGraph.Edge e : g.edges) {
            wires.add(wire(e.a(), e.b(), e.spec(), ox, oz, zRef));
            specs.add(e.spec());
            labels.add(e.label());
        }
        Feed feed = monopole ? Feed.base(0) : Feed.center(0);
        return new Built(new AntennaModel(wires, feed, ground), List.copyOf(specs), List.copyOf(labels), zRef, ox, oz);
    }

    private static Wire wire(AntennaGraph.Point a, AntennaGraph.Point b, ConductorSpec spec, double ox, double oz, double zRef) {
        return Wire.of(a.x() - ox, -(a.z() - oz), a.y() - zRef, b.x() - ox, -(b.z() - oz), b.y() - zRef,
                spec.radius(), spec.resistivity());
    }

    /** A Minecraft-frame direction (x east, y up, z south) in the solver frame. */
    public static double[] toSolver(double mx, double my, double mz) {
        return new double[] {mx, -mz, my};
    }

    /** A solver-frame direction in the Minecraft frame. */
    public static double[] toMinecraft(double sx, double sy, double sz) {
        return new double[] {sx, sz, -sy};
    }
}
