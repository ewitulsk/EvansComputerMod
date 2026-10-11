package com.example.evanscomputermod.radio.antenna.solver;

/**
 * A lumped series impedance (Ω) inserted in a wire at the mesh node nearest
 * {@code position}, e.g. a loading coil, trap or terminating resistor.
 */
public record Load(int wire, double position, Complex impedance) {
    public Load {
        if (wire < 0) throw new AntennaGeometryException("load wire index " + wire);
        if (!(position >= 0 && position <= 1)) throw new AntennaGeometryException("load position must be in [0, 1]");
        if (impedance == null || !impedance.isFinite()) throw new AntennaGeometryException("load impedance");
    }

    public static Load resistor(int wire, double position, double ohms) { return new Load(wire, position, Complex.real(ohms)); }

    /** A fixed R + jX (the same at every frequency). */
    public static Load series(int wire, double position, double resistance, double reactance) {
        return new Load(wire, position, Complex.of(resistance, reactance));
    }
}
