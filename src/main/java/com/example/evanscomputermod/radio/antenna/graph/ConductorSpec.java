package com.example.evanscomputermod.radio.antenna.graph;

/**
 * Electrical properties of one conductor kind (a wire tier, a fine wire, a
 * vanilla iron bar...). Pure data: the Minecraft side reads it from the
 * {@code evanscomputermod:rf_conductor} block data map and falls back to
 * {@link RfDefaults}.
 *
 * @param name           display name used when this conductor is the weakest link ("copper wire")
 * @param radius         electrical radius in metres (not the model thickness)
 * @param resistivity    bulk resistivity, Ω·m
 * @param currentRatingA continuous RMS current the conductor carries without overheating, A
 * @param coronaVoltage  peak voltage an <i>uninsulated</i> open end of this conductor holds before corona/arcing, V
 * @param oxidizes       true for bare copper, which weathers like vanilla copper (see {@link #oxidized})
 */
public record ConductorSpec(String name, double radius, double resistivity, double currentRatingA, double coronaVoltage,
        boolean oxidizes) {

    /** Resistivity multiplier per oxidation stage (unaffected, exposed, weathered, oxidized). */
    public static final double[] OXIDATION_FACTOR = {1.0, 1.6, 2.6, 4.0};

    public ConductorSpec {
        if (!(radius > 0) || resistivity < 0 || !(currentRatingA > 0) || !(coronaVoltage > 0))
            throw new IllegalArgumentException("bad conductor spec " + name);
    }

    /**
     * The same conductor at an oxidation stage 0..3. Surface oxide raises the
     * skin-effect resistance (RF current flows in the surface), modelled as a
     * resistivity multiplier; waxing stops the stage from advancing.
     */
    public ConductorSpec oxidized(int stage) {
        if (!oxidizes || stage <= 0) return this;
        int s = Math.min(stage, OXIDATION_FACTOR.length - 1);
        return new ConductorSpec(name, radius, resistivity * OXIDATION_FACTOR[s], currentRatingA, coronaVoltage, true);
    }
}
