package com.example.evanscomputermod.radio.antenna.solver;

/**
 * The solved antenna at one frequency.
 *
 * <p>Currents and voltages are peak amplitudes (phasor magnitudes) for 1 W of
 * input power; at P watts multiply by √P. Divide by √2 for RMS (heating).
 *
 * @param frequencyHz            solve frequency
 * @param feedImpedance          impedance at the feed, Ω
 * @param efficiency             radiation efficiency η = P_rad / P_in = R_rad / (R_rad + R_loss)
 * @param lossPowerFraction      ohmic (wire and load) loss as a fraction of input power
 * @param pattern                gain pattern, or {@code null} if the solve skipped it
 * @param peakGainDbi            peak IEEE gain, NaN without a pattern
 * @param peakThetaDeg           θ of the peak, NaN without a pattern
 * @param peakPhiDeg             φ of the peak, NaN without a pattern
 * @param segmentCurrentPerWatt  current amplitude at each mesh segment's midpoint (A at 1 W)
 * @param peakCurrentPerWatt     largest current amplitude anywhere on the antenna (A at 1 W)
 * @param peakEndVoltagePerWatt  largest wire potential amplitude at or just inside an open wire end, from the
 *                               end charge (V at 1 W, relative to infinity / the ground plane). Antennas without
 *                               open ends (loops) report the largest node potential instead
 */
public record AntennaResult(double frequencyHz, Complex feedImpedance, double efficiency, double lossPowerFraction,
        GainPattern pattern, double peakGainDbi, double peakThetaDeg, double peakPhiDeg, double[] segmentCurrentPerWatt,
        double peakCurrentPerWatt, double peakEndVoltagePerWatt) {
    public static final double DEFAULT_Z0 = 50;

    public AntennaResult {
        segmentCurrentPerWatt = segmentCurrentPerWatt.clone();
    }

    @Override
    public double[] segmentCurrentPerWatt() { return segmentCurrentPerWatt.clone(); }

    /** Voltage reflection coefficient against a real reference impedance. */
    public Complex reflectionCoefficient(double z0) {
        Complex z = feedImpedance;
        return z.sub(Complex.real(z0)).div(z.add(Complex.real(z0)));
    }

    /** SWR against a real reference impedance (∞ for a total mismatch). */
    public double swr(double z0) { return swr(feedImpedance, Complex.real(z0)); }

    public double swr() { return swr(DEFAULT_Z0); }

    /** SWR of a load against a (possibly complex) source impedance, using the power-wave reflection coefficient. */
    public static double swr(Complex load, Complex z0) {
        Complex g = load.sub(z0.conj()).div(load.add(z0));
        double m = g.abs();
        return m >= 1 ? Double.POSITIVE_INFINITY : (1 + m) / (1 - m);
    }

    /** Fraction of forward power reflected at the feed against z0. */
    public double mismatchLoss(double z0) {
        double m = reflectionCoefficient(z0).abs();
        return m * m;
    }

    public double radiationResistance() { return feedImpedance.re() * efficiency; }

    public double lossResistance() { return feedImpedance.re() * (1 - efficiency); }

    /** Peak directivity (gain without losses) in dBi, NaN without a pattern. */
    public double peakDirectivityDbi() {
        return efficiency > 0 ? peakGainDbi - 10 * Math.log10(efficiency) : Double.NaN;
    }

    public boolean hasPattern() { return pattern != null; }

    public double peakCurrent(double watts) { return peakCurrentPerWatt * Math.sqrt(watts); }

    public double peakEndVoltage(double watts) { return peakEndVoltagePerWatt * Math.sqrt(watts); }
}
