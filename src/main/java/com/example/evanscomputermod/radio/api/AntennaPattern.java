package com.example.evanscomputermod.radio.api;

/**
 * An antenna's far-field gain, in its own local frame (+Y up, +Z forward).
 * Link budgets rotate directions into this frame with the endpoint's {@link Pose}
 * and read gain here; the antenna solver produces these, fixed antennas
 * (whips, Wi-Fi panels, dishes) implement them directly.
 */
public interface AntennaPattern {

    /** Gain in dBi towards a local unit direction. */
    double gainDbi(double lx, double ly, double lz);

    /**
     * Unit polarization vector of the radiated E-field towards a local direction,
     * as {x, y, z}; vertical dipoles return roughly (0, 1, 0).
     */
    double[] polarization(double lx, double ly, double lz);

    /** Peak gain over all directions (used for culling). */
    double peakGainDbi();

    /** Feedline / mismatch / tuner loss between radio and antenna, in dB. */
    default double feedLossDb() {
        return 0;
    }

    /** An ideal isotropic antenna with vertical polarization. */
    AntennaPattern ISOTROPIC = new AntennaPattern() {
        @Override public double gainDbi(double lx, double ly, double lz) { return 0; }
        @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {0, 1, 0}; }
        @Override public double peakGainDbi() { return 0; }
    };

    /** A vertical half-wave dipole (2.15 dBi, sin²θ pattern), the default whip. */
    AntennaPattern VERTICAL_DIPOLE = new AntennaPattern() {
        @Override public double gainDbi(double lx, double ly, double lz) {
            double len = Math.sqrt(lx * lx + ly * ly + lz * lz);
            if (len == 0) return 2.15;
            double c = ly / len;                 // cos θ from the element axis
            double s2 = Math.max(1e-6, 1 - c * c);
            // Half-wave dipole: (cos(π/2 cosθ)/sinθ)², peak 1.64 (2.15 dBi).
            double f = Math.cos(Math.PI / 2 * c);
            return 10 * Math.log10(Math.max(1e-6, 1.64 * f * f / s2));
        }
        @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {0, 1, 0}; }
        @Override public double peakGainDbi() { return 2.15; }
    };
}
