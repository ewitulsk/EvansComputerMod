package com.example.evanscomputermod.radio.hazard;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

/** The hazard thermal model: exact, reproducible time to failure; never fails at or below rating. */
public class ThermalModelTest {

    /** Steps tick by tick from θ₀ under a constant load until failure (or a cap). */
    private static long simulate(double theta0, double load, double tau, long cap) {
        double theta = theta0;
        for (long n = 1; n <= cap; n++) {
            theta = ThermalModel.step(theta, load, ThermalModel.TICK_SECONDS, tau);
            if (theta >= 1) return n;
        }
        return Long.MAX_VALUE;
    }

    @Test
    void closedFormTimeToFailure() {
        // L = 2 from cold: t = τ ln 2.
        assertEquals(10 * Math.log(2), ThermalModel.timeToFailure(0, 2, 10), 1e-12);
        // Copper wire at 20x its rating: t = 20 s · ln(20/19) ≈ 1.026 s.
        assertEquals(20 * Math.log(20.0 / 19.0), ThermalModel.Part.WIRE.timeToFailure(0, 20), 1e-12);
        // From part-way warm it is shorter: τ ln((L-θ0)/(L-1)).
        assertEquals(30 * Math.log((4 - 0.5) / 3.0), ThermalModel.timeToFailure(0.5, 4, 30), 1e-12);
        assertEquals(0, ThermalModel.timeToFailure(1.0, 0.5, 30));
    }

    @Test
    void atOrBelowRatingNeverFails() {
        assertEquals(Double.POSITIVE_INFINITY, ThermalModel.timeToFailure(0, 1.0, 20));
        assertEquals(Double.POSITIVE_INFINITY, ThermalModel.timeToFailure(0, 0.6, 20));
        assertEquals(Long.MAX_VALUE, ThermalModel.ticksToFailure(0, 1.0, 20));
        // A part held at its rating approaches but never reaches the failure point.
        assertEquals(Long.MAX_VALUE, simulate(0, 1.0, 2, 200_000));
    }

    @Test
    void steppedSimulationFailsExactlyOnPredictedTick() {
        double[] loads = {1.01, 1.3, 2, 5, 20, 400};
        double[] taus = {2, 20, 30};
        double[] starts = {0, 0.3, 0.69};
        for (double tau : taus)
            for (double l : loads)
                for (double t0 : starts) {
                    long predicted = ThermalModel.ticksToFailure(t0, l, tau);
                    long simulated = simulate(t0, l, tau, 10_000_000);
                    assertEquals(predicted, simulated, "τ=" + tau + " L=" + l + " θ0=" + t0);
                    // The tick count brackets the closed-form time.
                    double t = ThermalModel.timeToFailure(t0, l, tau);
                    assertTrue(predicted * ThermalModel.TICK_SECONDS >= t - 1e-9, "after closed form");
                    assertTrue((predicted - 1) * ThermalModel.TICK_SECONDS < t + 1e-9, "within one tick");
                }
    }

    @Test
    void reproducible() {
        long a = simulate(0, 7.5, 20, 1_000_000), b = simulate(0, 7.5, 20, 1_000_000);
        assertEquals(a, b);
        assertEquals(ThermalModel.ticksToFailure(0, 7.5, 20), ThermalModel.ticksToFailure(0, 7.5, 20));
        // 20 * ln(7.5/6.5) = 2.862 s -> tick 58.
        assertEquals(58, a);
    }

    @Test
    void coolsBackWhenIdle() {
        double theta = 0.9;
        for (int i = 0; i < 20 * 60; i++) theta = ThermalModel.Part.WIRE.step(theta, 0);
        assertEquals(0.9 * Math.exp(-60.0 / 20), theta, 1e-9);
        assertEquals(20 + 0.5 * 180, ThermalModel.Part.WIRE.celsius(0.5), 1e-9);
    }

    @Test
    void describesTimes() {
        assertEquals("never", ThermalModel.describeSeconds(Double.POSITIVE_INFINITY));
        assertEquals("1.0 s", ThermalModel.describeSeconds(1.026));
        assertEquals("12 s", ThermalModel.describeSeconds(12.2));
        assertEquals("3 min 20 s", ThermalModel.describeSeconds(200));
    }
}
