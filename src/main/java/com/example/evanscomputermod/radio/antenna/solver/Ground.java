package com.example.evanscomputermod.radio.antenna.solver;

/**
 * Ground below the antenna, as a plane at z = 0.
 *
 * <ul>
 *   <li>{@link Type#NONE}: free space.</li>
 *   <li>{@link Type#PERFECT}: perfectly conducting plane, exact image method.</li>
 *   <li>{@link Type#REAL}: lossy ground by the reflection-coefficient approximation (as NEC's RCA,
 *       simplified): each image interaction is the perfect-ground image scaled by a Fresnel
 *       coefficient Γ evaluated at the grazing angle between the two segments' centres, using the
 *       complex permittivity ε<sub>c</sub> = ε<sub>r</sub> − jσ/(ωε<sub>0</sub>). Γ blends the
 *       vertical (TM) and horizontal (TE) coefficients by the segments' vertical direction cosine.
 *       The far field applies the exact TM/TE coefficients to the θ/φ components at each
 *       elevation. Surface-wave and near-ground losses are not modelled beyond that; the radiated
 *       power is the space wave integrated over the upper hemisphere, so η includes ground
 *       reflection loss.</li>
 * </ul>
 */
public record Ground(Type type, double relativePermittivity, double conductivity) {
    public enum Type { NONE, PERFECT, REAL }

    public static final Ground NONE = new Ground(Type.NONE, 1, 0);
    public static final Ground PERFECT = new Ground(Type.PERFECT, 1, Double.POSITIVE_INFINITY);
    /** ITU-R P.527 style presets. */
    public static final Ground AVERAGE = real(13, 0.005);
    public static final Ground POOR = real(4, 0.001);
    public static final Ground WET = real(30, 0.02);
    public static final Ground SEA_WATER = real(80, 5);
    public static final Ground FRESH_WATER = real(80, 0.01);

    public Ground {
        if (type == null) throw new AntennaGeometryException("ground type");
        if (type == Type.REAL && !(relativePermittivity >= 1 && conductivity >= 0 && Double.isFinite(relativePermittivity)
                && Double.isFinite(conductivity))) throw new AntennaGeometryException("real ground needs εr ≥ 1 and σ ≥ 0");
    }

    public static Ground real(double relativePermittivity, double conductivity) {
        return new Ground(Type.REAL, relativePermittivity, conductivity);
    }

    public boolean present() { return type != Type.NONE; }
}
