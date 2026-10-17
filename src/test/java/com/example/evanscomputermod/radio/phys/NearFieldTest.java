package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

/** The 13.56 MHz inductive-coupling loss law against the passive card's link budget. */
public class NearFieldTest {
    static final double F = 13.56e6;

    @Test
    void lossIsMonotoneAndSixtyDbPerDecade() {
        double prev = Double.NEGATIVE_INFINITY;
        for(double d = 0; d <= 10; d += 0.01) {
            double l = NearField.lossDb(d, F);
            assertTrue(l >= prev, "monotone at " + d);
            prev = l;
        }
        assertEquals(NearField.L0_DB, NearField.lossDb(0, F), 1e-12, "flat inside the reference distance");
        assertEquals(NearField.L0_DB, NearField.lossDb(NearField.REF_DISTANCE_M, F), 1e-12);
        assertEquals(60, NearField.lossDb(10, F) - NearField.lossDb(1, F), 1e-9, "a decade of distance");
        assertEquals(NearField.lossDb(1, F), NearField.lossDb(1, 14e6), 1e-12, "frequency is ignored");
    }

    @Test
    void zeroDbmReaderIntoMinusSixtyClosesAtOnePointFiveAndFailsByThree() {
        double budget = 0 - (-60);
        assertTrue(NearField.lossDb(1.5, F) < budget - 10, "closes at 1.5 m with margin: " + NearField.lossDb(1.5, F));
        assertTrue(NearField.lossDb(3, F) > budget, "fails by 3 m: " + NearField.lossDb(3, F));
        // The spec's card: wake at −45 dBm, reply 20 dB under the field, reader sensitivity −90 dBm.
        double l1 = NearField.lossDb(1, F);
        assertTrue(0 - l1 >= -45, "a 0 dBm reader wakes a card at 1 m");
        assertTrue(0 - 2 * l1 - 20 >= -90, "and hears its reply at 1 m");
        double l4 = NearField.lossDb(4, F);
        assertTrue(10 - l4 < -45, "a 10 dBm reader does not wake a card at 4 m");
    }

    @Test
    void distanceForLossInvertsLoss() {
        for(double d = 0.3; d <= 10; d += 0.1) assertEquals(d, NearField.distanceForLossM(NearField.lossDb(d, F), F), 1e-9);
        assertEquals(NearField.REF_DISTANCE_M, NearField.distanceForLossM(NearField.L0_DB - 5, F), 1e-12, "clamped at the reference distance");
        assertEquals(2.5, NearField.distanceForLossM(NearField.L0_DB + 60, F), 1e-9, "one decade out");
    }
}
