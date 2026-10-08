package com.example.evanscomputermod.radio.antenna.solver;

import java.util.List;

/** Reference geometries shared by the solver tests. Wavelength is 1 m at {@link #F}. */
final class TestAntennas {
    static final double F = AntennaSolver.C0; // λ = 1 m

    private TestAntennas() {}

    /** Centre-fed vertical dipole of length l (in wavelengths at F) in free space. */
    static AntennaModel dipole(double l, double radius, int segments) {
        return new AntennaModel(List.of(new Wire(0, 0, -l / 2, 0, 0, l / 2, radius, Wire.PERFECT, segments)), Feed.center(0), Ground.NONE);
    }

    /** Base-fed vertical monopole of height h over the given ground. */
    static AntennaModel monopole(double h, double radius, int segments, Ground ground) {
        return new AntennaModel(List.of(new Wire(0, 0, 0, 0, 0, h, radius, Wire.PERFECT, segments)), Feed.base(0), ground);
    }

    /** Square loop of the given perimeter in the xz plane, fed at the middle of its bottom side. */
    static AntennaModel squareLoop(double perimeter, double radius, int segmentsPerSide) {
        double s = perimeter / 4;
        var b = AntennaModel.builder();
        b.wire(new Wire(-s / 2, 0, 0, s / 2, 0, 0, radius, Wire.PERFECT, segmentsPerSide));
        b.wire(new Wire(s / 2, 0, 0, s / 2, 0, s, radius, Wire.PERFECT, segmentsPerSide));
        b.wire(new Wire(s / 2, 0, s, -s / 2, 0, s, radius, Wire.PERFECT, segmentsPerSide));
        b.wire(new Wire(-s / 2, 0, s, -s / 2, 0, 0, radius, Wire.PERFECT, segmentsPerSide));
        return b.feed(Feed.center(0)).build();
    }

    /** 3-element Yagi along +x: reflector 0.5λ, driven 0.47λ, director 0.44λ, 0.2λ spacing, a = 0.001λ. */
    static AntennaModel yagi() {
        var b = AntennaModel.builder();
        b.wire(new Wire(0, 0, -0.25, 0, 0, 0.25, 1e-3, Wire.PERFECT, 21));
        int driven = b.wire(new Wire(0.2, 0, -0.235, 0.2, 0, 0.235, 1e-3, Wire.PERFECT, 21));
        b.wire(new Wire(0.4, 0, -0.22, 0.4, 0, 0.22, 1e-3, Wire.PERFECT, 21));
        return b.feed(Feed.center(driven)).build();
    }

    /** Fraction of input power radiated, by integrating the pattern (trapezoid in θ). */
    static double integratedGain(GainPattern p, boolean upperHemisphereOnly) {
        int nt = upperHemisphereOnly ? GainPattern.THETA_COUNT / 2 + 1 : GainPattern.THETA_COUNT;
        double d = Math.toRadians(GainPattern.STEP_DEG), sum = 0;
        for (int t = 0; t < nt; t++) {
            double w = (t == 0 || t == nt - 1) ? 0.5 : 1, st = Math.sin(Math.toRadians(GainPattern.thetaDeg(t)));
            for (int q = 0; q < GainPattern.PHI_COUNT; q++) sum += p.gain(t, q) * st * w;
        }
        return sum * d * d / (4 * Math.PI);
    }
}
