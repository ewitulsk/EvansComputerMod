package com.example.evanscomputermod.radio.phys;

/**
 * Inductive-coupling path loss for the 13.56 MHz near field (RFID/NFC): the magnetic
 * field of a small loop falls as 1/d&sup3; inside &lambda;/2&pi; (3.5 m at 13.56 MHz), so
 * coupled power falls 60 dB per decade,
 * L = {@value #L0_DB} + 60 log10(max(d, {@value #REF_DISTANCE_M}) / {@value #REF_DISTANCE_M}) dB.
 *
 * <p>Constants, for the spec's passive card (reader 0..10 dBm, card wake threshold
 * &minus;45 dBm, 20 dB backscatter loss, reader sensitivity &minus;90 dBm):
 * <ul>
 *   <li>{@link #REF_DISTANCE_M} = 0.25 m: a card touching the reader; closer is no better.</li>
 *   <li>{@link #L0_DB} = &minus;3 dB at the reference distance (coils nearly critically coupled;
 *       the medium clamps path loss at 0 dB). The loss is then 33 dB at 1 m, 44 dB at 1.5 m,
 *       62 dB at 3 m and 69 dB at 4 m: a 0 dBm reader into a &minus;60 dBm receiver closes at
 *       1.5 m (16 dB spare) and fails by 3 m; a 0 dBm reader wakes a card to 1.6 m and reads
 *       its reply to 1.1 m (10 dBm: 2.3 m and 1.3 m).</li>
 * </ul>
 * Frequency is accepted for symmetry with {@link FreeSpace} and ignored: the band is 14 kHz wide.
 *
 * <p>The medium pairs this with the receiver's thermal floor alone (no HF atmospheric or
 * man-made noise, which would sit 40 dB above kT0B here): a coil coupled to another coil is
 * not a far-field antenna, and the &minus;90 dBm reader sensitivity above assumes as much.
 */
public final class NearField {
    /** Reference distance of the loss law, metres (a card on the reader). */
    public static final double REF_DISTANCE_M = 0.25;
    /** Loss at the reference distance, dB. */
    public static final double L0_DB = -3;
    /** Slope: magnetic dipole field, dB per decade of distance. */
    public static final double SLOPE_DB_PER_DECADE = 60;

    private NearField() {}

    /** Near-field coupling loss in dB (may be slightly negative inside the reference distance). */
    public static double lossDb(double distanceM, double freqHz) {
        return L0_DB + SLOPE_DB_PER_DECADE * Math.log10(Math.max(distanceM, REF_DISTANCE_M) / REF_DISTANCE_M);
    }

    /** Distance at which the near-field loss reaches {@code lossDb}, metres (the reference distance at or below L0). */
    public static double distanceForLossM(double lossDb, double freqHz) {
        if (lossDb <= L0_DB) return REF_DISTANCE_M;
        return REF_DISTANCE_M * Math.pow(10.0, (lossDb - L0_DB) / SLOPE_DB_PER_DECADE);
    }
}
