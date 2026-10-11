package com.example.evanscomputermod.radio.phys;

/**
 * Unit conversions and physical constants for the propagation library. The world
 * uses 1 block = 1 m and real wavelengths, so every quantity here is SI and every
 * method names its unit (Hz, m, s, dB, dBm, mW, W).
 */
public final class Units {
    /** Speed of light in vacuum, m/s (exact). */
    public static final double SPEED_OF_LIGHT_MPS = 299_792_458.0;
    /** Vacuum permeability, H/m (classical 4&pi;&times;10<sup>-7</sup>; the 2019 SI value differs by 10<sup>-10</sup>). */
    public static final double MU0_H_PER_M = 4e-7 * Math.PI;
    /** Vacuum permittivity, F/m, from 1 / (&mu;0 c&sup2;). */
    public static final double EPS0_F_PER_M = 1.0 / (MU0_H_PER_M * SPEED_OF_LIGHT_MPS * SPEED_OF_LIGHT_MPS);
    /** Boltzmann constant, J/K (exact). */
    public static final double BOLTZMANN_J_PER_K = 1.380649e-23;
    /** dB per neper of amplitude attenuation: 20 / ln 10. */
    public static final double DB_PER_NEPER = 20.0 / Math.log(10.0);

    private Units() {}

    public static double dbmToMw(double dbm) {
        return Math.pow(10.0, dbm / 10.0);
    }

    /** Milliwatts to dBm; zero or negative power is {@link Double#NEGATIVE_INFINITY}. */
    public static double mwToDbm(double mw) {
        return mw <= 0 ? Double.NEGATIVE_INFINITY : 10.0 * Math.log10(mw);
    }

    public static double dbmToW(double dbm) {
        return dbmToMw(dbm) * 1e-3;
    }

    public static double wToDbm(double w) {
        return mwToDbm(w * 1e3);
    }

    /** Power ratio in dB to a linear power ratio. */
    public static double dbToLinear(double db) {
        return Math.pow(10.0, db / 10.0);
    }

    /** Linear power ratio to dB; zero is {@link Double#NEGATIVE_INFINITY}. */
    public static double linearToDb(double linear) {
        return linear <= 0 ? Double.NEGATIVE_INFINITY : 10.0 * Math.log10(linear);
    }

    /** Power ratio in dB to an amplitude (field, voltage) ratio. */
    public static double dbToAmplitude(double db) {
        return Math.pow(10.0, db / 20.0);
    }

    public static double amplitudeToDb(double amplitude) {
        return amplitude <= 0 ? Double.NEGATIVE_INFINITY : 20.0 * Math.log10(amplitude);
    }

    /** Free-space wavelength &lambda; = c / f, metres. */
    public static double wavelengthM(double freqHz) {
        return SPEED_OF_LIGHT_MPS / freqHz;
    }

    public static double frequencyHz(double wavelengthM) {
        return SPEED_OF_LIGHT_MPS / wavelengthM;
    }

    /** Angular frequency &omega; = 2&pi;f, rad/s. */
    public static double angularFrequency(double freqHz) {
        return 2.0 * Math.PI * freqHz;
    }

    /** One-way propagation delay over {@code distanceM}, seconds. */
    public static double propagationDelayS(double distanceM) {
        return distanceM / SPEED_OF_LIGHT_MPS;
    }

    /** Thermal noise power kTB in dBm for a temperature in kelvin and a bandwidth in Hz. */
    public static double thermalNoiseDbm(double temperatureK, double bandwidthHz) {
        return wToDbm(BOLTZMANN_J_PER_K * temperatureK * bandwidthHz);
    }
}
