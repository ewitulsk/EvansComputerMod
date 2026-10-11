package com.example.evanscomputermod.radio.phys;

import java.util.SplittableRandom;

/**
 * Small-scale fading as a complex channel gain h with E|h|&sup2; = 1.
 *
 * <p>Rician with factor K (LOS to scattered power): h = &radic;(K/(K+1)) + &radic;(1/(K+1)) &middot; (x + jy)/&radic;2,
 * x, y ~ N(0, 1). K = 0 is Rayleigh (no line of sight). Draws are deterministic:
 * either from a caller's {@link SplittableRandom}, or allocation-free from a
 * {@code long} seed through SplitMix64 (the generator inside SplittableRandom).
 *
 * <p>Coherence time follows Clarke's model, T<sub>c</sub> &asymp; 0.423 / f<sub>D</sub> with
 * Doppler spread f<sub>D</sub> = v/&lambda; for relative speed v. A static link
 * (v = 0) never changes; a moving player or Sable ship re-draws every T<sub>c</sub>,
 * via {@link #blockSeed}.
 */
public final class Fading {
    private static final long GOLDEN = 0x9E3779B97F4A7C15L;
    /** Default Rician K for line-of-sight links, dB ("mild by default"). */
    public static final double DEFAULT_LOS_K_DB = 9.0;

    private Fading() {}

    /** SplitMix64 finalizer (Steele, Lea, Flood 2014); also the output function of {@link SplittableRandom}. */
    public static long mix64(long z) {
        z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
        z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
        return z ^ (z >>> 31);
    }

    /** Combines two seeds (e.g. a link id and a coherence-block index). */
    public static long combine(long a, long b) {
        return mix64(a + GOLDEN * (b + 1));
    }

    /** Coherence time T<sub>c</sub> = 0.423 &lambda; / v, seconds; +infinity when not moving. */
    public static double coherenceTimeS(double relativeSpeedMps, double freqHz) {
        double fd = Math.abs(relativeSpeedMps) / Units.wavelengthM(freqHz);
        return fd <= 0 ? Double.POSITIVE_INFINITY : 0.423 / fd;
    }

    /** Seed of the coherence block containing {@code timeS}: constant within one T<sub>c</sub>. */
    public static long blockSeed(long linkSeed, double timeS, double coherenceTimeS) {
        long block = Double.isInfinite(coherenceTimeS) || coherenceTimeS <= 0 ? 0 : (long) Math.floor(timeS / coherenceTimeS);
        return combine(linkSeed, block);
    }

    public static double kFromDb(double kDb) {
        return Units.dbToLinear(kDb);
    }

    /** Rician channel phasor drawn from {@code rng}; K linear (0 = Rayleigh). */
    public static Complex ricianPhasor(double kLinear, SplittableRandom rng) {
        double u1 = 1.0 - rng.nextDouble();
        double u2 = rng.nextDouble();
        return phasor(kLinear, u1, u2);
    }

    /** Rician channel phasor from a seed, allocation-free except the result. */
    public static Complex ricianPhasor(double kLinear, long seed) {
        return phasor(kLinear, unitOpen(seed, 1), unitOpen(seed, 2));
    }

    /** Rician power gain 10 log10|h|&sup2;, dB (negative is a fade). Allocation-free. */
    public static double ricianGainDb(double kLinear, long seed) {
        double u1 = unitOpen(seed, 1), u2 = unitOpen(seed, 2);
        double r = Math.sqrt(-2.0 * Math.log(u1));
        double t = 2.0 * Math.PI * u2;
        double los = Math.sqrt(kLinear / (kLinear + 1));
        double s = Math.sqrt(1.0 / (kLinear + 1)) / Math.sqrt(2.0);
        double re = los + s * r * Math.cos(t);
        double im = s * r * Math.sin(t);
        return 10.0 * Math.log10(Math.max(1e-12, re * re + im * im));
    }

    public static double ricianGainDb(double kLinear, SplittableRandom rng) {
        return ricianPhasor(kLinear, rng).powerDb();
    }

    /** Rayleigh power gain, dB. */
    public static double rayleighGainDb(long seed) {
        return ricianGainDb(0, seed);
    }

    public static Complex rayleighPhasor(long seed) {
        return ricianPhasor(0, seed);
    }

    /** Uniform in (0, 1] from a seed and stream index. */
    private static double unitOpen(long seed, int stream) {
        return ((mix64(seed + GOLDEN * stream) >>> 11) + 1) * 0x1.0p-53;
    }

    private static Complex phasor(double k, double u1, double u2) {
        double r = Math.sqrt(-2.0 * Math.log(u1));
        double t = 2.0 * Math.PI * u2;
        double s = Math.sqrt(1.0 / (k + 1)) / Math.sqrt(2.0);
        return new Complex(Math.sqrt(k / (k + 1)) + s * r * Math.cos(t), s * r * Math.sin(t));
    }
}
