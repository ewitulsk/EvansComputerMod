package com.example.evanscomputermod.radio.sdr;

/** SDR hardware tiers (spec, SDR block: Hardware tiers and realism). Rate caps are further limited by server config. */
public enum SdrTier {
    BASIC(0.5e6, 1700e6, 48_000, 8, false, 0),
    STANDARD(10e3, 6e9, 250_000, 12, true, 37),
    ADVANCED(1e3, 6e9, 1_000_000, 16, true, 37);

    public final double minHz, maxHz;
    public final int maxRate;
    public final int adcBits;
    public final boolean canTransmit;
    /** Exciter output cap, dBm (37 dBm = 5 W). */
    public final double maxTxDbm;

    SdrTier(double minHz, double maxHz, int maxRate, int adcBits, boolean canTransmit, double maxTxDbm) {
        this.minHz = minHz;
        this.maxHz = maxHz;
        this.maxRate = maxRate;
        this.adcBits = adcBits;
        this.canTransmit = canTransmit;
        this.maxTxDbm = maxTxDbm;
    }

    public String id() {
        return "sdr_" + name().toLowerCase(java.util.Locale.ROOT);
    }
}
