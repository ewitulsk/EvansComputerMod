package com.example.evanscomputermod.radio.phys;

/**
 * Result of an ionospheric (skywave) path evaluation.
 *
 * @param available          whether any skywave mode reaches the receiver
 * @param hops               number of ionospheric hops (0 when unavailable)
 * @param lossDb             total basic transmission loss, dB (+infinity when unavailable)
 * @param absorptionDb       D-layer absorption over all hops, dB (included in lossDb)
 * @param mufHz              maximum usable frequency for the hop length used (0 when no sky)
 * @param skipDistanceBlocks skip distance at this frequency, blocks (0 when f &le; foF2)
 * @param elevationRad       take-off elevation angle of the ray
 */
public record SkywavePath(boolean available, int hops, double lossDb, double absorptionDb, double mufHz,
        double skipDistanceBlocks, double elevationRad) {
    public static final SkywavePath NONE = new SkywavePath(false, 0, Double.POSITIVE_INFINITY, 0, 0,
            Double.POSITIVE_INFINITY, 0);

    static SkywavePath none(double mufHz, double skipBlocks) {
        return new SkywavePath(false, 0, Double.POSITIVE_INFINITY, 0, mufHz, skipBlocks, 0);
    }
}
