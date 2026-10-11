package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.Antenna;
import org.jetbrains.annotations.Nullable;

/**
 * The power budget of one transmission through a {@link TransmitChain} at
 * one frequency: drive, amplifier operating point, reflections, tuner heat,
 * power entering each feedline run and the power the antenna accepts.
 *
 * <p>The emission handed to the medium carries {@link #emissionDbm}; the
 * endpoint's pattern subtracts the whole chain's matched line loss plus the
 * antenna's mismatch loss (its {@code feedLossDb}), so what radiates is the
 * amplifier's output minus the runs after it and the mismatch. Lines before
 * the amplifier only lower its drive, so their loss is added back to the
 * emission figure to cancel the pattern's share of it.
 *
 * @param lineInW power entering each hop's feedline run, W
 */
public record ChainBudget(double hz, double driveW, double forwardW, double preAmpLossDb, @Nullable AmpModel.Point amp,
                          double rhoAntenna, double rhoAtAmp, boolean tunerMatched, double tunerHeatW, double[] lineInW,
                          double antennaInW, double acceptedW, double emissionDbm) {

    /** Reflected power at the amplifier, W (0 without one). */
    public double reflectedW() {
        return amp == null ? 0 : amp.reflectedW();
    }

    /** SWR the amplifier (or exciter) sees. */
    public double swrAtAmp() {
        return AmpModel.swrOf(rhoAtAmp);
    }

    /**
     * Works out the budget.
     *
     * @param tier   the first amplifier's tier (null: no amplifier in the chain)
     * @param supply that amplifier's FE supply fraction
     * @param limitW its extra output limit (arc foldback), +∞ for none
     */
    public static ChainBudget compute(TransmitChain chain, @Nullable Antenna antenna, double hz, double driveW,
                                      @Nullable AmpModel.Tier tier, double supply, double limitW) {
        var hops = chain.hops();
        int n = hops.size();
        boolean fed = chain.feed() != null;
        double rhoAnt = fed && antenna != null && antenna.present() ? AmpModel.rho(antenna.swrAt(hz)) : 1;
        // Backward pass: reflection seen at each device's output (index k+1 = device of hop k; 0 = exciter).
        double[] outRho = new double[n + 1];
        double[] tunerOut = new double[n];
        double r = fed ? rhoAnt : 1;
        boolean tunerMatched = false;
        for (int j = n - 1; j >= 0; j--) {
            TransmitChain.Hop h = hops.get(j);
            outRho[j + 1] = h.kind() == TransmitChain.Kind.FEED || h.kind() == TransmitChain.Kind.OPEN ? 0 : r;
            if (h.kind() == TransmitChain.Kind.TUNER) {
                tunerOut[j] = r;
                if (AmpModel.tunerMatches(AmpModel.swrOf(r))) {
                    r = 0;
                    tunerMatched = true;
                }
            }
            r = AmpModel.rhoThroughLine(r, h.line().lossDb(hz));
            if (j == 0) outRho[0] = r;
        }
        // Forward pass.
        double p = driveW, preAmp = 0, tunerHeat = 0, antennaIn = 0, rhoAtAmp = outRho[0];
        double[] lineIn = new double[n];
        AmpModel.Point point = null;
        for (int j = 0; j < n; j++) {
            TransmitChain.Hop h = hops.get(j);
            lineIn[j] = p;
            p *= AmpModel.dbToFraction(h.line().lossDb(hz));
            switch (h.kind()) {
                case AMPLIFIER -> {
                    if (point == null && tier != null) {
                        preAmp = chain.lossThroughDb(j, hz);
                        rhoAtAmp = outRho[j + 1];
                        point = AmpModel.operate(tier, p, supply, rhoAtAmp, limitW);
                        p = point.outW();
                    }
                }
                case TUNER -> {
                    if (AmpModel.tunerMatches(AmpModel.swrOf(tunerOut[j]))) tunerHeat += AmpModel.tunerHeatW(p, tunerOut[j]);
                }
                case FEED -> antennaIn = p;
                case OPEN -> antennaIn = 0;
            }
        }
        double forward = point != null ? point.outW() : driveW;
        double accepted = fed ? antennaIn * (1 - rhoAnt) : 0;
        double emission = point != null ? AmpModel.wToDbm(forward) + preAmp : AmpModel.wToDbm(driveW);
        return new ChainBudget(hz, driveW, forward, preAmp, point, rhoAnt, rhoAtAmp, tunerMatched, tunerHeat, lineIn,
                antennaIn, accepted, emission);
    }
}
//?}
