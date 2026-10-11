package com.example.evanscomputermod.radio.antenna.solver;

/**
 * A delta-gap voltage source on a wire. The solver's unknowns live on mesh
 * nodes, so the gap is placed at the node nearest {@code position}
 * (0 = the wire's first endpoint, 1 = its second). Positive voltage drives
 * current from the first endpoint towards the second. With an auto-split wire
 * the mesh puts a node exactly at the requested position.
 *
 * <p>A feed at an endpoint that touches the ground plane is the base feed of
 * a monopole; a feed at a free (unconnected, ungrounded) end is rejected.
 */
public record Feed(int wire, double position, Complex voltage) {
    public Feed {
        if (wire < 0) throw new AntennaGeometryException("feed wire index " + wire);
        if (!(position >= 0 && position <= 1)) throw new AntennaGeometryException("feed position must be in [0, 1]");
        if (voltage == null || !voltage.isFinite() || voltage.abs() == 0) throw new AntennaGeometryException("feed voltage");
    }

    public static Feed center(int wire) { return new Feed(wire, 0.5, Complex.ONE); }

    /** Feed at the wire's first endpoint, e.g. the base of a vertical touching the ground. */
    public static Feed base(int wire) { return new Feed(wire, 0, Complex.ONE); }

    public static Feed at(int wire, double position) { return new Feed(wire, position, Complex.ONE); }
}
