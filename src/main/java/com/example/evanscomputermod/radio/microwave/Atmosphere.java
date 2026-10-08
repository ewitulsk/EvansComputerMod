package com.example.evanscomputermod.radio.microwave;

import com.example.evanscomputermod.radio.phys.Noise;

/**
 * Atmospheric loss on a microwave hop: gaseous absorption (oxygen around 60 GHz,
 * the 22 GHz water-vapour line) plus rain fade (ITU-R P.838 via
 * {@link Noise#rainFadeDb}) above 10 GHz.
 *
 * <p>Gas model: a sea-level fit to ITU-R P.676 — oxygen 15 dB/km at 60 GHz
 * falling off as a Gaussian of 5.5 GHz width (≈11 dB/km at 57 GHz, ≈5 at 66),
 * water vapour 0.2 dB/km at 22.2 GHz (7.5 g/m³), and a 0.008 dB/km dry-air floor.
 */
public final class Atmosphere {
    private Atmosphere() {}

    /** Minecraft rain, mm/h (the {@link Noise} constants: rain 10, thunderstorm 50). */
    public static double rainRateMmPerH(boolean raining, boolean thundering) {
        return thundering ? Noise.THUNDER_RAIN_RATE_MM_PER_H : raining ? Noise.RAIN_RATE_MM_PER_H : 0;
    }

    /** Gaseous specific attenuation, dB/km. */
    public static double gasDbPerKm(double freqHz) {
        double f = freqHz / 1e9;
        double oxygen = 15.0 * Math.exp(-Math.pow((f - 60.0) / 5.5, 2));
        double vapour = 0.2 * Math.exp(-Math.pow((f - 22.235) / 3.0, 2));
        return oxygen + vapour + 0.008;
    }

    /** Rain fade over the path, dB: none at or below 10 GHz (the spec's threshold). */
    public static double rainDb(double freqHz, double rainRateMmPerH, double pathM) {
        return freqHz > 10e9 ? Noise.rainFadeDb(freqHz, rainRateMmPerH, pathM) : 0;
    }

    /** Total atmospheric loss on a hop of {@code pathM} metres, dB. */
    public static double lossDb(double freqHz, double pathM, double rainRateMmPerH) {
        return gasDbPerKm(freqHz) * Math.max(0, pathM) / 1000.0 + rainDb(freqHz, rainRateMmPerH, pathM);
    }
}
