package com.example.evanscomputermod.radio.phys;

/**
 * Attenuation of one block (1 m) of a material, scaled with frequency as
 * &alpha;(f) = &alpha;<sub>ref</sub> &middot; (f/f<sub>ref</sub>)<sup>k</sup> dB per block.
 *
 * <p>For a low-loss dielectric &alpha; &asymp; 8.686 &middot; (&sigma;/2) &middot; &radic;(&mu;0/&epsilon;) and
 * ITU-R P.2040 gives &sigma; = c&middot;f<sup>d</sup>, so k is the conductivity exponent d.
 * The power law alone underestimates loss at VLF/LF where conduction dominates,
 * so a material may also name a {@link Ground} medium: the block then attenuates
 * by at least the skin-depth loss of that medium ({@link SkinDepth}). Both terms
 * are non-decreasing in frequency, so {@link #dbPerBlock} is too (for k &ge; 0).
 */
public record MaterialAttenuation(double dbPerBlockAtRef, double refFreqHz, double exponentK, Ground lowFrequencyMedium) {
    /** Edge of one block, metres. */
    public static final double BLOCK_M = 1.0;

    public MaterialAttenuation(double dbPerBlockAtRef, double refFreqHz, double exponentK) {
        this(dbPerBlockAtRef, refFreqHz, exponentK, null);
    }

    public MaterialAttenuation {
        if(!(dbPerBlockAtRef >= 0)) throw new IllegalArgumentException("attenuation must be >= 0");
        if(!(refFreqHz > 0)) throw new IllegalArgumentException("reference frequency must be > 0");
    }

    /** Attenuation of one block at {@code freqHz}, dB. */
    public double dbPerBlock(double freqHz) {
        if(freqHz <= 0) return 0;
        double law = dbPerBlockAtRef == 0 ? 0 : dbPerBlockAtRef * Math.pow(freqHz / refFreqHz, exponentK);
        if(lowFrequencyMedium == null) return law;
        return Math.max(law, SkinDepth.attenuationDbPerM(freqHz, lowFrequencyMedium) * BLOCK_M);
    }

    /** Attenuation through {@code blocks} (may be fractional, e.g. a ray's path length in the block). */
    public double lossDb(double blocks, double freqHz) {
        return blocks <= 0 ? 0 : dbPerBlock(freqHz) * blocks;
    }
}
