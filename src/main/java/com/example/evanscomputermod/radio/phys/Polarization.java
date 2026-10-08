package com.example.evanscomputermod.radio.phys;

/** Wave polarization of an antenna or path. Linear tilt is measured from horizontal. */
public enum Polarization {
    VERTICAL(Math.PI / 2),
    HORIZONTAL(0),
    CIRCULAR_RIGHT(Double.NaN),
    CIRCULAR_LEFT(Double.NaN);

    private final double tiltRad;

    Polarization(double tiltRad) {
        this.tiltRad = tiltRad;
    }

    public boolean isLinear() {
        return !Double.isNaN(tiltRad);
    }

    public boolean isCircular() {
        return !isLinear();
    }

    /** Nominal tilt from horizontal for linear polarizations; NaN for circular. */
    public double tiltRad() {
        return tiltRad;
    }
}
