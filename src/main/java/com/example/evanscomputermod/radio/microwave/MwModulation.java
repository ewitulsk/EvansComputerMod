package com.example.evanscomputermod.radio.microwave;

import com.example.evanscomputermod.radio.medium.BasicRadioMedium;

/**
 * Adaptive coding and modulation for microwave links. Rate = channel width ×
 * bits per symbol × 0.85 (roll-off and FEC overhead), so a 112 MHz 24 GHz
 * channel at 4096-QAM carries ~1.1 Gbit/s and a 1 GHz 60 GHz channel ~10.
 * The SINR each needs for 10% PER on a 1000-bit frame lives in
 * {@link BasicRadioMedium#requiredSinrDb}, the one table the medium rolls against.
 */
public enum MwModulation {
    BPSK("MW-BPSK", 1),
    QPSK("MW-QPSK", 2),
    QAM16("MW-QAM16", 4),
    QAM64("MW-QAM64", 6),
    QAM256("MW-QAM256", 8),
    QAM1024("MW-QAM1024", 10),
    QAM4096("MW-QAM4096", 12);

    /** Extra SINR kept in hand when picking a modulation from feedback, dB. */
    public static final double MARGIN_DB = 3;

    public final String id;
    public final int bitsPerSymbol;

    MwModulation(String id, int bitsPerSymbol) {
        this.id = id;
        this.bitsPerSymbol = bitsPerSymbol;
    }

    public double requiredSinrDb() {
        return BasicRadioMedium.requiredSinrDb(id);
    }

    public double bitRate(double bandwidthHz) {
        return bandwidthHz * bitsPerSymbol * 0.85;
    }

    /** The fastest modulation that decodes a {@code bits}-long frame at {@code sinrDb} with {@link #MARGIN_DB} to spare (BPSK at worst). */
    public static MwModulation choose(double sinrDb, int bits) {
        if (Double.isNaN(sinrDb)) return BPSK;
        double lengthPenalty = bits > 0 ? Math.log10(Math.max(1, bits / 1000.0)) : 0;
        MwModulation best = BPSK;
        for (MwModulation m : values())
            if (m.requiredSinrDb() + lengthPenalty + MARGIN_DB <= sinrDb) best = m;
        return best;
    }

    public static MwModulation byId(String id) {
        for (MwModulation m : values()) if (m.id.equals(id)) return m;
        return null;
    }
}
