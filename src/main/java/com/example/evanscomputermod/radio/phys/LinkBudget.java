package com.example.evanscomputermod.radio.phys;

/**
 * The spec's link budget:
 * P<sub>rx</sub> = P<sub>tx</sub> + G<sub>tx</sub> + G<sub>rx</sub> &minus; L<sub>path</sub> &minus; L<sub>obs</sub>
 * &minus; L<sub>diff</sub> &minus; L<sub>pol</sub> &minus; L<sub>feed</sub> &minus; F,
 * and SINR = P<sub>rx</sub> / (N + &Sigma; I<sub>k</sub>&middot;ACI<sub>k</sub>).
 * All losses are positive dB; {@code fadeLossDb} is F (use &minus;{@link Fading#ricianGainDb}).
 */
public record LinkBudget(double txPowerDbm, double txGainDbi, double rxGainDbi, double pathLossDb,
        double obstructionDb, double diffractionDb, double polarizationDb, double feedLossDb, double fadeLossDb) {

    public static Builder builder() {
        return new Builder();
    }

    /** Received power, dBm. */
    public double rxPowerDbm() {
        return txPowerDbm + txGainDbi + rxGainDbi - pathLossDb - obstructionDb - diffractionDb - polarizationDb
                - feedLossDb - fadeLossDb;
    }

    /** Total loss between transmitter output and receiver input, net of antenna gains, dB. */
    public double netLossDb() {
        return txPowerDbm - rxPowerDbm();
    }

    /** SINR of this link against {@code noiseDbm} plus a summed interference power (mW, already ACI-weighted). */
    public double sinrDb(double noiseDbm, double interferenceMwSum) {
        return sinrDbFor(rxPowerDbm(), noiseDbm, interferenceMwSum);
    }

    /** SINR against noise plus each interferer's ACI-weighted power in mW. */
    public double sinrDb(double noiseDbm, double... interferenceMw) {
        double sum = 0;
        for(double i : interferenceMw) sum += i;
        return sinrDbFor(rxPowerDbm(), noiseDbm, sum);
    }

    /** SINR = S / (N + I), dB. */
    public static double sinrDbFor(double signalDbm, double noiseDbm, double interferenceMwSum) {
        double n = Units.dbmToMw(noiseDbm) + Math.max(0, interferenceMwSum);
        return signalDbm - Units.mwToDbm(n);
    }

    /** Link margin over a receiver sensitivity, dB. */
    public double marginDb(double sensitivityDbm) {
        return rxPowerDbm() - sensitivityDbm;
    }

    /** Mutable builder; all terms default to 0. */
    public static final class Builder {
        private double txPowerDbm, txGainDbi, rxGainDbi, pathLossDb, obstructionDb, diffractionDb, polarizationDb,
                feedLossDb, fadeLossDb;

        public Builder txPowerDbm(double v) { txPowerDbm = v; return this; }
        public Builder txPowerW(double w) { txPowerDbm = Units.wToDbm(w); return this; }
        public Builder txGainDbi(double v) { txGainDbi = v; return this; }
        public Builder rxGainDbi(double v) { rxGainDbi = v; return this; }
        public Builder pathLossDb(double v) { pathLossDb = v; return this; }
        public Builder obstructionDb(double v) { obstructionDb = v; return this; }
        public Builder diffractionDb(double v) { diffractionDb = v; return this; }
        public Builder polarizationDb(double v) { polarizationDb = v; return this; }
        public Builder feedLossDb(double v) { feedLossDb = v; return this; }
        public Builder fadeLossDb(double v) { fadeLossDb = v; return this; }

        /** Takes path, diffraction and obstruction terms from a {@link PathLossModel.Result}. */
        public Builder path(PathLossModel.Result r) {
            obstructionDb = r.obstructionDb();
            diffractionDb = r.diffractionDb();
            pathLossDb = r.totalDb() - r.obstructionDb() - r.diffractionDb();
            return this;
        }

        public LinkBudget build() {
            return new LinkBudget(txPowerDbm, txGainDbi, rxGainDbi, pathLossDb, obstructionDb, diffractionDb,
                    polarizationDb, feedLossDb, fadeLossDb);
        }
    }
}
