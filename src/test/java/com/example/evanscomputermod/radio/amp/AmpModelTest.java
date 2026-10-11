package com.example.evanscomputermod.radio.amp;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.hazard.ThermalModel;
import org.junit.jupiter.api.Test;

/** Amplifier and tuner arithmetic: FE draw, brownout, foldback, reflected heat and tuner loss accounting. */
public class AmpModelTest {
    static final double WPF = 5;   // RadioConfig default: 1 FE/t = 5 W

    @Test
    void feDrawIsTwiceRfOverWattsPerFe() {
        // Spec: 40 / 400 / 4000 FE/t at full output for the three tiers.
        for (AmpModel.Tier t : AmpModel.Tier.values()) {
            AmpModel.Point p = AmpModel.operate(t, AmpModel.FULL_DRIVE_W, 1, 0, Double.POSITIVE_INFINITY);
            assertEquals(t.ratedW, p.outW(), 1e-9, t.id);
            assertEquals(2 * t.ratedW / WPF, p.fePerTick(WPF), 1e-9, t.id);
            assertEquals(t.ratedW, p.heatW(), 1e-9, "50% efficient: waste heat = RF out");
        }
        assertEquals(40, AmpModel.feDrawPerTick(100, 1, WPF), 1e-12);
        // Only while transmitting: draw scales with airtime.
        assertEquals(10, AmpModel.feDrawPerTick(100, 0.25, WPF), 1e-12);
        assertEquals(0, AmpModel.feDrawPerTick(100, 0, WPF), 1e-12);
        // A different conversion rate.
        assertEquals(20, AmpModel.feDrawPerTick(100, 1, 10), 1e-12);
    }

    @Test
    void gainSaturatesAtTier() {
        assertEquals(13.0103, AmpModel.Tier.W100.gainDb(), 1e-3);
        assertEquals(23.0103, AmpModel.Tier.KW1.gainDb(), 1e-3);
        assertEquals(50, AmpModel.targetW(AmpModel.Tier.W100, 2.5), 1e-9);
        assertEquals(100, AmpModel.targetW(AmpModel.Tier.W100, 50), 1e-9);
    }

    @Test
    void brownoutScalesOutputWithoutDamage() {
        AmpModel.Point half = AmpModel.operate(AmpModel.Tier.KW1, 5, 0.5, 0, Double.POSITIVE_INFINITY);
        assertEquals(500, half.outW(), 1e-9);
        assertEquals(200, half.fePerTick(WPF), 1e-9);
        assertFalse(half.foldback());
        // Unpowered: the exciter passes straight through (bypass), nothing drawn, no heat.
        AmpModel.Point off = AmpModel.operate(AmpModel.Tier.KW1, 5, 0, 0, Double.POSITIVE_INFINITY);
        assertEquals(5, off.outW(), 1e-9);
        assertEquals(0, off.amplifiedW(), 1e-9);
        assertEquals(0, off.fePerTick(WPF), 1e-9);
        assertEquals(0, off.heatW(), 1e-9);
        // Supply fraction from a buffer: 20 FE for a 40 FE/t tick = 50%.
        assertEquals(0.5, AmpModel.supplyFraction(20, 100, WPF), 1e-12);
        assertEquals(1, AmpModel.supplyFraction(16_000, 100, WPF), 1e-12);
        assertEquals(0, AmpModel.supplyFraction(0, 100, WPF), 1e-12);
    }

    @Test
    void protectedTiersFoldBackOnSwr() {
        // SWR 3:1 -> rho 0.25: a 1 kW amp keeps reflected power at 10% of 1 kW = 100 W -> 400 W out.
        double rho3 = AmpModel.rho(3);
        assertEquals(0.25, rho3, 1e-12);
        AmpModel.Point p = AmpModel.operate(AmpModel.Tier.KW1, 5, 1, rho3, Double.POSITIVE_INFINITY);
        assertTrue(p.foldback());
        assertEquals(400, p.outW(), 1e-9);
        assertEquals(100, p.reflectedW(), 1e-9);
        // SWR 1.5:1 (rho 0.04) is within the limit: no foldback.
        AmpModel.Point ok = AmpModel.operate(AmpModel.Tier.KW1, 5, 1, AmpModel.rho(1.5), Double.POSITIVE_INFINITY);
        assertFalse(ok.foldback());
        assertEquals(1000, ok.outW(), 1e-9);
        // No antenna (rho 1): 10 kW tier folds back to 1 kW, never above its reflected limit.
        AmpModel.Point open = AmpModel.operate(AmpModel.Tier.KW10, 5, 1, 1, Double.POSITIVE_INFINITY);
        assertEquals(1000, open.outW(), 1e-9);
        // Arc foldback limit.
        assertEquals(300, AmpModel.operate(AmpModel.Tier.KW1, 5, 1, 0, 300).outW(), 1e-9);
        // Protected amps never overheat.
        for (double swr : new double[] {1, 2, 3, 10, Double.POSITIVE_INFINITY}) {
            AmpModel.Point q = AmpModel.operate(AmpModel.Tier.KW1, 5, 1, AmpModel.rho(swr), Double.POSITIVE_INFINITY);
            assertTrue(AmpModel.ampLoad(AmpModel.Tier.KW1, q.heatW()) <= 1, "SWR " + swr);
        }
    }

    @Test
    void unprotectedTierBurnsOutIntoBadMatch() {
        // 100 W into an open line: it keeps 100 W out and absorbs all 100 W reflected.
        AmpModel.Point open = AmpModel.operate(AmpModel.Tier.W100, 5, 1, 1, Double.POSITIVE_INFINITY);
        assertFalse(open.foldback());
        assertEquals(100, open.outW(), 1e-9);
        assertEquals(200, open.heatW(), 1e-9);
        double load = AmpModel.ampLoad(AmpModel.Tier.W100, open.heatW());
        assertEquals(1.6, load, 1e-12);
        // Burns out after τ ln(1.6/0.6) = 29.4 s of continuous key-down.
        assertEquals(30 * Math.log(1.6 / 0.6), ThermalModel.Part.AMPLIFIER.timeToFailure(0, load), 1e-9);
        // Up to SWR 3:1 it survives indefinitely.
        AmpModel.Point fine = AmpModel.operate(AmpModel.Tier.W100, 5, 1, AmpModel.rho(3), Double.POSITIVE_INFINITY);
        assertTrue(AmpModel.ampLoad(AmpModel.Tier.W100, fine.heatW()) <= 1);
    }

    @Test
    void reflectionThroughLossyLine() {
        // A 3 dB line halves the reflection each way: rho/4 at the amplifier.
        assertEquals(0.25 / 4, AmpModel.rhoThroughLine(0.25, 10 * Math.log10(2)), 1e-12);
        assertEquals(3, AmpModel.swrOf(0.25), 1e-12);
        assertEquals(Double.POSITIVE_INFINITY, AmpModel.swrOf(1));
        assertEquals(1, AmpModel.rho(Double.POSITIVE_INFINITY));
        assertEquals(0, AmpModel.rho(1));
    }

    @Test
    void tunerMovesMismatchIntoItselfButCannotAddPower() {
        double fwd = 1000, rhoAnt = AmpModel.rho(5);      // a short whip, SWR 5:1 -> rho 0.444
        // Without a tuner the amplifier sees the reflection (and a protected one folds back).
        AmpModel.Point bare = AmpModel.operate(AmpModel.Tier.KW1, 5, 1, rhoAnt, Double.POSITIVE_INFINITY);
        assertTrue(bare.foldback());
        // With a tuner it sees a match and delivers full power...
        AmpModel.Point tuned = AmpModel.operate(AmpModel.Tier.KW1, 5, 1, 0, Double.POSITIVE_INFINITY);
        assertEquals(fwd, tuned.outW(), 1e-9);
        assertEquals(0, tuned.reflectedW(), 1e-9);
        // ...but the antenna still accepts only (1 - rho); the rest heats the tuner. Energy is conserved.
        double heat = AmpModel.tunerHeatW(fwd, rhoAnt), accepted = AmpModel.acceptedW(fwd, 0, rhoAnt);
        assertEquals(fwd * rhoAnt, heat, 1e-9);
        assertEquals(fwd, heat + accepted, 1e-9);
        assertEquals(AmpModel.mismatchLossDb(rhoAnt), 10 * Math.log10(fwd / accepted), 1e-9);
        // Over the tuner's 600 W rating it would overheat; at 1 kW into SWR 5 (444 W) it doesn't.
        assertTrue(heat < AmpModel.TUNER_RATING_W);
        assertTrue(AmpModel.tunerHeatW(fwd, AmpModel.rho(20)) > AmpModel.TUNER_RATING_W);
        // It can't match an open line.
        assertFalse(AmpModel.tunerMatches(Double.POSITIVE_INFINITY));
        assertTrue(AmpModel.tunerMatches(20));
    }
}
