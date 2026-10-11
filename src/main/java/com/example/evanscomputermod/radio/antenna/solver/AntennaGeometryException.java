package com.example.evanscomputermod.radio.antenna.solver;

/** Thrown for geometry the solver cannot model (zero-length wires, bad radii, feeds on open ends, too many segments...). */
public class AntennaGeometryException extends IllegalArgumentException {
    public AntennaGeometryException(String message) { super(message); }
}
