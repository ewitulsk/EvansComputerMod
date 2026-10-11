package com.example.evanscomputermod.radio.amp;

/**
 * Power amplifier, tuner and feedline arithmetic (spec, Power). Pure: no
 * Minecraft types, unit-tested.
 *
 * <ul>
 *   <li><b>Gain:</b> a tier reaches its rating from a full 5 W exciter
 *       (100 W: 13 dB, 1 kW: 23 dB, 10 kW: 33 dB); output = min(tier, drive × gain).</li>
 *   <li><b>Efficiency:</b> 50%: DC in = 2 × RF out, FE/t = 2 × RF × duty / wattsPerFe;
 *       the other half is heat.</li>
 *   <li><b>Brownout:</b> with only a fraction {@code s} of the FE it needs, the
 *       amplified output scales by {@code s}; an unpowered amplifier passes the
 *       exciter straight through (bypass relay). No damage.</li>
 *   <li><b>Reflected power:</b> ρ = Γ² with Γ = (SWR − 1)/(SWR + 1), seen at the
 *       amplifier through the feedline's round-trip loss. Protected tiers (1 kW,
 *       10 kW) fold back so reflected power stays under 10% of the tier; the
 *       100 W tier has no protection and absorbs it as heat.</li>
 *   <li><b>Tuner:</b> the amplifier sees SWR ≈ 1, but the antenna still accepts
 *       only (1 − ρ) of the power: the mismatch moves into the tuner as heat.
 *       A tuner can't fix an inefficient antenna.</li>
 * </ul>
 */
public final class AmpModel {

    /** Full exciter drive, W (37 dBm). */
    public static final double FULL_DRIVE_W = 5;
    /** DC-to-RF efficiency. */
    public static final double EFFICIENCY = 0.5;
    /** Protected tiers keep reflected power under this fraction of the tier. */
    public static final double REFLECTED_LIMIT = 0.10;
    /** An amplifier dissipates this many times its tier continuously (its own waste heat at full output plus 25%). */
    public static final double HEAT_RATING = 1.25;
    /** Tuner dissipation rating, W (the mismatch power it can absorb continuously). */
    public static final double TUNER_RATING_W = 600;
    /** The highest SWR a tuner can match; worse (or no antenna) passes straight through. */
    public static final double TUNER_MAX_SWR = 30;

    public enum Tier {
        W100("amplifier_100w", 100, false),
        KW1("amplifier_1kw", 1_000, true),
        KW10("amplifier_10kw", 10_000, true);

        public final String id;
        public final double ratedW;
        /** Folds back on high SWR instead of absorbing the reflected power. */
        public final boolean protectedOutput;

        Tier(String id, double ratedW, boolean protectedOutput) {
            this.id = id;
            this.ratedW = ratedW;
            this.protectedOutput = protectedOutput;
        }

        /** Linear power gain. */
        public double gain() {
            return ratedW / FULL_DRIVE_W;
        }

        public double gainDb() {
            return 10 * Math.log10(gain());
        }
    }

    /** One transmit's amplifier operating point. */
    public record Point(double driveW, double outW, double amplifiedW, double reflectedW, double heatW,
                        boolean foldback, double supply) {
        /** FE per tick drawn at this point while transmitting the whole tick. */
        public double fePerTick(double wattsPerFe) {
            return feDrawPerTick(amplifiedW, 1, wattsPerFe);
        }
    }

    private AmpModel() {}

    public static double dbmToW(double dbm) {
        return Math.pow(10, dbm / 10) / 1000;
    }

    public static double wToDbm(double w) {
        return 10 * Math.log10(Math.max(1e-15, w) * 1000);
    }

    public static double dbToFraction(double lossDb) {
        return Math.pow(10, -lossDb / 10);
    }

    /** Reflected fraction ρ = Γ² for an SWR (1 for ∞ / no antenna). */
    public static double rho(double swr) {
        if (Double.isNaN(swr) || swr <= 1) return 0;
        if (Double.isInfinite(swr)) return 1;
        double g = (swr - 1) / (swr + 1);
        return g * g;
    }

    public static double swrOf(double rho) {
        double g = Math.sqrt(Math.max(0, Math.min(1, rho)));
        return g >= 1 ? Double.POSITIVE_INFINITY : (1 + g) / (1 - g);
    }

    /** Mismatch loss for a reflected fraction, dB. */
    public static double mismatchLossDb(double rho) {
        return -10 * Math.log10(Math.max(1e-12, 1 - rho));
    }

    /** The reflected fraction seen at the amplifier: the antenna's, attenuated by the feedline loss both ways. */
    public static double rhoThroughLine(double rhoAntenna, double lineLossDb) {
        return rhoAntenna * dbToFraction(2 * lineLossDb);
    }

    /** min(tier, drive × gain). */
    public static double targetW(Tier tier, double driveW) {
        return Math.min(tier.ratedW, Math.max(0, driveW) * tier.gain());
    }

    /**
     * The amplifier's operating point.
     *
     * @param supply   fraction of the FE it needs that is available, 0..1 (brownout)
     * @param rhoAtAmp reflected fraction seen at its output
     * @param limitW   extra output limit (arc foldback), +∞ for none
     */
    public static Point operate(Tier tier, double driveW, double supply, double rhoAtAmp, double limitW) {
        double s = Math.max(0, Math.min(1, supply));
        double amplified = targetW(tier, driveW) * s;
        boolean foldback = false;
        if (tier.protectedOutput) {
            double cap = limitW;
            if (rhoAtAmp > 0) cap = Math.min(cap, REFLECTED_LIMIT * tier.ratedW / rhoAtAmp);
            if (amplified > cap) {
                amplified = Math.max(0, cap);
                foldback = true;
            }
        }
        // Bypass: below the drive level (unpowered) the exciter passes straight through.
        double out = Math.max(Math.max(0, driveW), amplified);
        if (amplified < driveW) amplified = 0;
        double reflected = out * Math.max(0, Math.min(1, rhoAtAmp));
        double heat = amplified * (1 / EFFICIENCY - 1) + (amplified > 0 ? reflected : 0);
        return new Point(driveW, out, amplified, reflected, heat, foldback, s);
    }

    /** FE per tick for {@code amplifiedW} of RF out over a duty fraction of the tick: 2 × RF / wattsPerFe. */
    public static double feDrawPerTick(double amplifiedW, double duty, double wattsPerFe) {
        return amplifiedW / EFFICIENCY * Math.max(0, Math.min(1, duty)) / wattsPerFe;
    }

    /** Supply fraction available from {@code storedFe} for a tick at {@code targetW}. */
    public static double supplyFraction(double storedFe, double targetW, double wattsPerFe) {
        double need = feDrawPerTick(targetW, 1, wattsPerFe);
        if (need <= 0) return storedFe > 0 ? 1 : 0;
        return Math.max(0, Math.min(1, storedFe / need));
    }

    /** Thermal load of an amplifier dissipating {@code heatW}. */
    public static double ampLoad(Tier tier, double heatW) {
        return heatW / (HEAT_RATING * tier.ratedW);
    }

    /** Power the tuner absorbs: the mismatch it hides from the amplifier. */
    public static double tunerHeatW(double powerIntoTunerW, double rhoAntenna) {
        return powerIntoTunerW * Math.max(0, Math.min(1, rhoAntenna));
    }

    /** True if a tuner can match an antenna with this SWR. */
    public static boolean tunerMatches(double swr) {
        return swr >= 1 && swr <= TUNER_MAX_SWR;
    }

    /** Power the antenna accepts from {@code forwardW} at the start of a line with {@code lineLossDb}. */
    public static double acceptedW(double forwardW, double lineLossDb, double rhoAntenna) {
        return forwardW * dbToFraction(lineLossDb) * (1 - Math.max(0, Math.min(1, rhoAntenna)));
    }
}
