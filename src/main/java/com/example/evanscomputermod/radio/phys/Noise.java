package com.example.evanscomputermod.radio.phys;

/**
 * Receiver noise and weather effects. Everything is deterministic and parameterised.
 *
 * <p><b>Floor.</b> N = &minus;174 + 10 log10 B + 10 log10(f<sub>rx</sub> + &Sigma; f<sub>a</sub>) dBm,
 * where f<sub>rx</sub> = 10<sup>NF/10</sup> is the receiver noise factor and f<sub>a</sub> =
 * 10<sup>F<sub>a</sub>/10</sup> the external noise factors above kT0B (ITU-R P.372 convention,
 * antenna at 290 K folded into f<sub>rx</sub>). With no external noise this is exactly
 * &minus;174 + 10 log B + NF.
 *
 * <p><b>External noise F<sub>a</sub> (dB above kT0B).</b>
 * <ul>
 * <li>Man-made, ITU-R P.372 Table 1: F<sub>a</sub> = c &minus; d log10 f<sub>MHz</sub> (f clamped to &ge; 0.3 MHz).</li>
 * <li>Galactic, P.372: 52 &minus; 23 log10 f<sub>MHz</sub>, only above 10 MHz (below, the ionosphere screens it).</li>
 * <li>Atmospheric (lightning worldwide), fitted to the P.372 mid-latitude curves:
 *     night 85 &minus; 35 log10 f<sub>MHz</sub> up to 10 MHz, falling 60 dB/decade above;
 *     by day the D layer removes up to 25 dB below ~5 MHz (scaled by daylight).</li>
 * <li>Thunderstorm overhead: a local storm adds impulsive noise 15 dB above the night atmospheric level.</li>
 * </ul>
 *
 * <p><b>Rain.</b> Specific attenuation &gamma; = k R<sup>&alpha;</sup> dB/km with k, &alpha;
 * interpolated from ITU-R P.838-3 (horizontal polarization; log k and &alpha; linear in log f).
 * The spec applies rain fade above 10 GHz; P.838 values below are tiny but returned as-is from 1 GHz.
 */
public final class Noise {
    /** kT0 at 290 K, dBm/Hz. */
    public static final double KT0_DBM_PER_HZ = -174.0;
    /** Minecraft rain and thunderstorm rain rates used by callers, mm/h. */
    public static final double RAIN_RATE_MM_PER_H = 10.0;
    public static final double THUNDER_RAIN_RATE_MM_PER_H = 50.0;
    /** Local thunderstorm excess over night-time atmospheric noise, dB. */
    public static final double THUNDERSTORM_EXCESS_DB = 15.0;

    /** P.372 man-made noise environments: F<sub>a</sub> = c &minus; d log10 f<sub>MHz</sub>. */
    public enum Environment {
        CITY(76.8, 27.7),
        RESIDENTIAL(72.5, 27.7),
        RURAL(67.2, 27.7),
        QUIET_RURAL(53.6, 28.6),
        /** No man-made noise at all (deep wilderness, ocean). */
        NONE(Double.NEGATIVE_INFINITY, 0);

        public final double c, d;

        Environment(double c, double d) {
            this.c = c;
            this.d = d;
        }
    }

    // ITU-R P.838-3 coefficients, horizontal polarization.
    private static final double[] RAIN_F_GHZ = {1, 1.5, 2, 2.5, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15, 20, 25, 30, 35, 40,
            45, 50, 60, 70, 80, 90, 100};
    private static final double[] RAIN_K = {0.0000259, 0.0000443, 0.0000847, 0.0001321, 0.0001390, 0.0001071,
            0.0002162, 0.0007056, 0.001915, 0.004115, 0.007535, 0.01217, 0.02386, 0.04481, 0.09164, 0.1571,
            0.2403, 0.3374, 0.4431, 0.5521, 0.6600, 0.8606, 1.0315, 1.1704, 1.2807, 1.3671};
    private static final double[] RAIN_ALPHA = {0.9691, 1.0185, 1.0664, 1.1209, 1.2322, 1.6009, 1.6969, 1.5900,
            1.4810, 1.3905, 1.3155, 1.2571, 1.1825, 1.1233, 1.0568, 0.9991, 0.9485, 0.9047, 0.8673, 0.8355,
            0.8084, 0.7656, 0.7345, 0.7115, 0.6944, 0.6815};

    private Noise() {}

    /** Thermal floor &minus;174 + 10 log10 B + NF, dBm. */
    public static double thermalDbm(double bandwidthHz, double noiseFigureDb) {
        return KT0_DBM_PER_HZ + 10.0 * Math.log10(bandwidthHz) + noiseFigureDb;
    }

    public static double manMadeFaDb(double freqHz, Environment env) {
        double fm = Math.max(0.3, freqHz / 1e6);
        return env.c - env.d * Math.log10(fm);
    }

    public static double galacticFaDb(double freqHz) {
        double fm = freqHz / 1e6;
        return fm < 10 ? Double.NEGATIVE_INFINITY : 52.0 - 23.0 * Math.log10(fm);
    }

    /** Night-time atmospheric noise F<sub>a</sub>, dB. */
    public static double atmosphericNightFaDb(double freqHz) {
        double fm = Math.max(1e-3, freqHz / 1e6);
        if(fm <= 10) return 85.0 - 35.0 * Math.log10(fm);
        return 50.0 - 60.0 * Math.log10(fm / 10.0);
    }

    /** Atmospheric noise at {@code dayTime}: D-layer daytime absorption removes up to 25 dB at low HF/MF. */
    public static double atmosphericFaDb(double freqHz, long dayTime) {
        double fr = freqHz / 5e6;
        return atmosphericNightFaDb(freqHz) - 25.0 * Ionosphere.daylight(dayTime) / (1.0 + fr * fr);
    }

    /** Extra impulsive noise of a thunderstorm in the area, dB above kT0B. */
    public static double thunderstormFaDb(double freqHz) {
        return atmosphericNightFaDb(freqHz) + THUNDERSTORM_EXCESS_DB;
    }

    /** Power sum of all external noise, dB above kT0B (-infinity when there is none). */
    public static double externalFaDb(double freqHz, Environment env, long dayTime, boolean thundering) {
        double sum = Units.dbToLinear(manMadeFaDb(freqHz, env)) + Units.dbToLinear(galacticFaDb(freqHz))
                + Units.dbToLinear(atmosphericFaDb(freqHz, dayTime));
        if(thundering) sum += Units.dbToLinear(thunderstormFaDb(freqHz));
        return Units.linearToDb(sum);
    }

    /** Total noise floor, dBm, for a receiver of noise figure {@code noiseFigureDb}. */
    public static double floorDbm(double bandwidthHz, double noiseFigureDb, double freqHz, Environment env,
            long dayTime, boolean thundering) {
        double fa = Units.dbToLinear(externalFaDb(freqHz, env, dayTime, thundering));
        return KT0_DBM_PER_HZ + 10.0 * Math.log10(bandwidthHz) + Units.linearToDb(Units.dbToLinear(noiseFigureDb) + fa);
    }

    /** Adds an external factor in dB to a thermal floor in dBm for the same bandwidth: power sum. */
    public static double addDbm(double aDbm, double bDbm) {
        return Units.mwToDbm(Units.dbmToMw(aDbm) + Units.dbmToMw(bDbm));
    }

    /**
     * Peak noise power of one nearby lightning stroke, dBm, in a receiver of
     * bandwidth {@code bandwidthHz}. A return stroke's radiated spectrum falls as
     * 1/f above ~10 kHz and its field as 1/d; this is referenced to &minus;20 dBm in
     * 10 kHz at 1 MHz and 1 km (game-level parameterisation of the sferic spectrum).
     */
    public static double lightningImpulseDbm(double freqHz, double bandwidthHz, double distanceM) {
        double d = Math.max(1.0, distanceM);
        double f = Math.max(1e4, freqHz);
        return -20.0 + 10.0 * Math.log10(bandwidthHz / 1e4) - 20.0 * Math.log10(f / 1e6) - 20.0 * Math.log10(d / 1000.0);
    }

    /** ITU-R P.838-3 specific rain attenuation &gamma; = kR<sup>&alpha;</sup>, dB/km (0 below 1 GHz or without rain). */
    public static double rainSpecificAttenuationDbPerKm(double freqHz, double rainRateMmPerH) {
        double fg = freqHz / 1e9;
        if(fg < 1 || rainRateMmPerH <= 0) return 0;
        int n = RAIN_F_GHZ.length;
        double k, a;
        if(fg >= RAIN_F_GHZ[n - 1]) {
            k = RAIN_K[n - 1];
            a = RAIN_ALPHA[n - 1];
        } else {
            int i = 0;
            while(RAIN_F_GHZ[i + 1] < fg) i++;
            double t = Math.log(fg / RAIN_F_GHZ[i]) / Math.log(RAIN_F_GHZ[i + 1] / RAIN_F_GHZ[i]);
            k = Math.exp(Math.log(RAIN_K[i]) + t * (Math.log(RAIN_K[i + 1]) - Math.log(RAIN_K[i])));
            a = RAIN_ALPHA[i] + t * (RAIN_ALPHA[i + 1] - RAIN_ALPHA[i]);
        }
        return k * Math.pow(rainRateMmPerH, a);
    }

    /** Rain fade over {@code pathM} of rain, dB. */
    public static double rainFadeDb(double freqHz, double rainRateMmPerH, double pathM) {
        return rainSpecificAttenuationDbPerKm(freqHz, rainRateMmPerH) * Math.max(0, pathM) / 1000.0;
    }
}
