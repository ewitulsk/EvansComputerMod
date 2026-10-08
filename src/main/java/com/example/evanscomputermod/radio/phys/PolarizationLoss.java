package com.example.evanscomputermod.radio.phys;

/**
 * Polarization mismatch loss. Between linear antennas whose E-planes differ by
 * &theta; the received power scales by cos&sup2;&theta;, so L = &minus;20 log10|cos &theta;|.
 * Real antennas and paths have finite cross-polar discrimination, so the loss is
 * capped at {@link #CROSS_POL_CAP_DB} (~20 dB, as the spec states for vertical to
 * horizontal line of sight). Linear to circular is 3.01 dB; opposite circular
 * hands hit the cap.
 *
 * <p>On skywave, Faraday rotation in the ionosphere scrambles the arriving
 * plane, so the loss is drawn from a uniformly random angle (mean 3 dB in power).
 */
public final class PolarizationLoss {
    public static final double CROSS_POL_CAP_DB = 20.0;
    /** 10 log10 2: linear against circular, and the mean power loss of a random plane. */
    public static final double LINEAR_TO_CIRCULAR_DB = 10.0 * Math.log10(2.0);

    private PolarizationLoss() {}

    /** Loss between two linear polarizations {@code angleBetweenRad} apart, dB. */
    public static double db(double angleBetweenRad) {
        double c = Math.abs(Math.cos(angleBetweenRad));
        if(c <= 0) return CROSS_POL_CAP_DB;
        return Math.min(CROSS_POL_CAP_DB, -20.0 * Math.log10(c));
    }

    /** Loss between nominal polarizations; linear pairs use their nominal tilts. */
    public static double db(Polarization tx, Polarization rx) {
        if(tx.isLinear() && rx.isLinear()) return db(tx.tiltRad() - rx.tiltRad());
        if(tx.isLinear() != rx.isLinear()) return LINEAR_TO_CIRCULAR_DB;
        return tx == rx ? 0 : CROSS_POL_CAP_DB;
    }

    /** Loss between linear antennas with explicit tilts from horizontal (e.g. rotated by a Sable sub-level). */
    public static double linearDb(double txTiltRad, double rxTiltRad) {
        return db(txTiltRad - rxTiltRad);
    }

    /**
     * Skywave loss for a linear receiver: the arriving plane is random (Faraday
     * rotation), drawn deterministically from {@code seed}. Circular receivers
     * always see {@link #LINEAR_TO_CIRCULAR_DB}.
     */
    public static double skywaveDb(Polarization rx, long seed) {
        if(rx.isCircular()) return LINEAR_TO_CIRCULAR_DB;
        double u = (Fading.mix64(seed ^ 0x5DEECE66DL) >>> 11) * 0x1.0p-53;
        return db(u * Math.PI);
    }
}
