package com.example.evanscomputermod.radio.antenna.solver;

import java.util.ArrayList;
import java.util.List;

/**
 * An antenna: straight wires, one feed, optional lumped loads, and the ground.
 *
 * <p>Wires whose endpoints coincide (within {@link #JOIN_TOLERANCE}) are
 * electrically joined; the mesh enforces current continuity (Kirchhoff) at
 * every junction. With a ground plane present, a wire endpoint at z = 0 is
 * connected to the ground. Nothing else touches: wires that cross without
 * sharing an endpoint are insulated from each other.
 */
public record AntennaModel(List<Wire> wires, Feed feed, List<Load> loads, Ground ground) {
    /** Endpoints closer than this (metres) are the same node. */
    public static final double JOIN_TOLERANCE = 1e-5;

    public AntennaModel {
        wires = List.copyOf(wires);
        loads = loads == null ? List.of() : List.copyOf(loads);
        if (ground == null) ground = Ground.NONE;
        validate(wires, feed, loads, ground);
    }

    public AntennaModel(List<Wire> wires, Feed feed, Ground ground) { this(wires, feed, List.of(), ground); }

    public static Builder builder() { return new Builder(); }

    public double totalWireLength() {
        double sum = 0;
        for (Wire w : wires) sum += w.length();
        return sum;
    }

    private static void validate(List<Wire> wires, Feed feed, List<Load> loads, Ground ground) {
        if (wires.isEmpty()) throw new AntennaGeometryException("antenna has no wires");
        if (feed == null) throw new AntennaGeometryException("antenna has no feed");
        for (int i = 0; i < wires.size(); i++) {
            Wire w = wires.get(i);
            double[] c = {w.x1(), w.y1(), w.z1(), w.x2(), w.y2(), w.z2(), w.radius(), w.resistivity()};
            for (double v : c) if (!Double.isFinite(v)) throw new AntennaGeometryException("wire " + i + " has a non-finite value");
            double len = w.length();
            if (len < 10 * JOIN_TOLERANCE) throw new AntennaGeometryException("wire " + i + " has zero length");
            if (!(w.radius() > 0)) throw new AntennaGeometryException("wire " + i + " radius must be positive");
            if (w.radius() * 2 >= len) throw new AntennaGeometryException("wire " + i + " is too thick for its length (thin-wire limit)");
            if (w.resistivity() < 0) throw new AntennaGeometryException("wire " + i + " has negative resistivity");
            if (ground.present()) {
                if (Math.min(w.z1(), w.z2()) < -JOIN_TOLERANCE) throw new AntennaGeometryException("wire " + i + " is below the ground plane");
                if ((w.z1() + w.z2()) / 2 < w.radius()) throw new AntennaGeometryException("wire " + i + " lies on the ground plane");
            }
        }
        if (feed.wire() >= wires.size()) throw new AntennaGeometryException("feed on missing wire " + feed.wire());
        for (Load l : loads) if (l.wire() >= wires.size()) throw new AntennaGeometryException("load on missing wire " + l.wire());
    }

    public static final class Builder {
        private final List<Wire> wires = new ArrayList<>();
        private final List<Load> loads = new ArrayList<>();
        private Feed feed;
        private Ground ground = Ground.NONE;

        /** Adds a wire and returns its index. */
        public int wire(Wire w) {
            wires.add(w);
            return wires.size() - 1;
        }

        public int wire(double x1, double y1, double z1, double x2, double y2, double z2, double radius, double resistivity) {
            return wire(Wire.of(x1, y1, z1, x2, y2, z2, radius, resistivity));
        }

        public Builder feed(Feed f) { feed = f; return this; }

        public Builder load(Load l) { loads.add(l); return this; }

        public Builder ground(Ground g) { ground = g; return this; }

        public AntennaModel build() { return new AntennaModel(wires, feed, loads, ground); }
    }
}
