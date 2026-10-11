package com.example.evanscomputermod.radio.antenna.solver;

/**
 * A straight conductor between two endpoints, in metres. The solver uses a
 * right-handed frame with <b>z up</b>; any ground plane lies at z = 0. The
 * integration layer maps Minecraft's y-up world coordinates onto this frame.
 *
 * @param radius      wire radius in metres (the electrical radius, not the model thickness)
 * @param resistivity bulk resistivity in Ω·m ({@link #COPPER}, {@link #IRON}, {@link #GOLD}); 0 = lossless
 * @param segments    explicit segment count, or {@link #AUTO} to split at the mesh's design frequency
 */
public record Wire(double x1, double y1, double z1, double x2, double y2, double z2,
        double radius, double resistivity, int segments) {
    public static final int AUTO = 0;
    public static final double COPPER = 1.68e-8;
    public static final double IRON = 9.7e-8;
    public static final double GOLD = 2.44e-8;
    public static final double ALUMINIUM = 2.65e-8;
    public static final double PERFECT = 0;

    public Wire {
        if (segments < 0) throw new AntennaGeometryException("negative segment count");
    }

    public static Wire of(double x1, double y1, double z1, double x2, double y2, double z2, double radius, double resistivity) {
        return new Wire(x1, y1, z1, x2, y2, z2, radius, resistivity, AUTO);
    }

    public Wire withSegments(int count) { return new Wire(x1, y1, z1, x2, y2, z2, radius, resistivity, count); }

    public double length() { return Math.sqrt((x2 - x1) * (x2 - x1) + (y2 - y1) * (y2 - y1) + (z2 - z1) * (z2 - z1)); }
}
