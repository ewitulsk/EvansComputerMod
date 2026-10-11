package com.example.evanscomputermod.radio.phys;

/**
 * Modulations with their AWGN bit error rate, from standard closed forms
 * (Proakis, <i>Digital Communications</i>; Sklar), Gray coding, no FEC.
 *
 * <p>{@link #berAt(double)} takes the SINR measured in the receiver bandwidth and
 * assumes one symbol per 1/B (Nyquist signalling), so E<sub>s</sub>/N<sub>0</sub> = SINR &middot; G
 * where G is the processing gain (1, or 2<sup>SF</sup> chips per symbol for LoRa chirps), and
 * E<sub>b</sub>/N<sub>0</sub> = E<sub>s</sub>/N<sub>0</sub> / bits-per-symbol.
 *
 * <ul>
 * <li>BPSK, QPSK: P<sub>b</sub> = Q(&radic;(2E<sub>b</sub>/N<sub>0</sub>)) (9.6 dB &rarr; 10<sup>-5</sup>).</li>
 * <li>DBPSK: &frac12; e<sup>&minus;E<sub>b</sub>/N<sub>0</sub></sup>.</li>
 * <li>DQPSK: P<sub>s</sub> &asymp; 2Q(&radic;(2E<sub>s</sub>/N<sub>0</sub>) sin(&pi;/(&radic;2&middot;4))), P<sub>b</sub> = P<sub>s</sub>/2.</li>
 * <li>Square M-QAM: P<sub>b</sub> &asymp; (4/k)(1 &minus; 1/&radic;M) Q(&radic;(3k/(M&minus;1) &middot; E<sub>b</sub>/N<sub>0</sub>)).</li>
 * <li>FSK (binary, non-coherent), OOK (non-coherent, average E<sub>b</sub>): &frac12; e<sup>&minus;E<sub>b</sub>/2N<sub>0</sub></sup>.</li>
 * <li>AFSK (Bell 202 tones through an audio channel): non-coherent FSK with a 3 dB implementation loss.</li>
 * <li>CHIRP_LORA (SF12): non-coherent M-ary orthogonal union bound
 *     P<sub>s</sub> &le; (M&minus;1)/2 &middot; e<sup>&minus;E<sub>s</sub>/2N<sub>0</sub></sup>, P<sub>b</sub> = P<sub>s</sub> &middot; (M/2)/(M&minus;1),
 *     M = 2<sup>SF</sup>; decodes near &minus;20 dB SNR like real LoRa SF12.</li>
 * <li>FM_ANALOG, AM_ANALOG: analog voice; for data they carry AFSK, and AM loses a
 *     further 4.8 dB (envelope detection efficiency 1/3 at full modulation).
 *     {@link #audioSnrDb} gives the voice SNR.</li>
 * </ul>
 */
public enum Modulation {
    BPSK(1, 1),
    DBPSK(1, 1),
    QPSK(2, 1),
    DQPSK(2, 1),
    QAM16(4, 1),
    QAM64(6, 1),
    QAM256(8, 1),
    FSK(1, 1),
    AFSK(1, 1),
    OOK(1, 1),
    CHIRP_LORA(12, 1 << 12),
    FM_ANALOG(1, 1),
    AM_ANALOG(1, 1);

    /** Default LoRa spreading factor of {@link #CHIRP_LORA}. */
    public static final int LORA_SF = 12;
    /** FM threshold: below this carrier-to-noise ratio the discriminator collapses, dB. */
    public static final double FM_THRESHOLD_DB = 10.0;
    /** FM improvement 3&beta;&sup2;(&beta;+1) for narrow-band FM (&beta; = 5 kHz / 3 kHz), dB. */
    public static final double FM_IMPROVEMENT_DB = 10.0 * Math.log10(3.0 * (5.0 / 3) * (5.0 / 3) * (5.0 / 3 + 1));

    private final int bitsPerSymbol;
    private final double processingGain;

    Modulation(int bitsPerSymbol, double processingGain) {
        this.bitsPerSymbol = bitsPerSymbol;
        this.processingGain = processingGain;
    }

    public int bitsPerSymbol() {
        return bitsPerSymbol;
    }

    /** Chips per symbol (linear), 1 except for chirp spread spectrum. */
    public double processingGain() {
        return processingGain;
    }

    public boolean isAnalog() {
        return this == FM_ANALOG || this == AM_ANALOG;
    }

    /** Raw bit rate in a bandwidth of {@code bandwidthHz} with Nyquist signalling, bit/s. */
    public double bitRateBps(double bandwidthHz) {
        return bandwidthHz * bitsPerSymbol / processingGain;
    }

    /** Bit error rate at E<sub>b</sub>/N<sub>0</sub> = {@code ebN0Db}. */
    public double berAtEbN0(double ebN0Db) {
        double g = Units.dbToLinear(ebN0Db);
        double ber = switch(this) {
            case BPSK, QPSK -> Special.q(Math.sqrt(2 * g));
            case DBPSK -> 0.5 * Math.exp(-g);
            case DQPSK -> Special.q(Math.sqrt(2 * 2 * g) * Math.sin(Math.PI / (Math.sqrt(2) * 4)));
            case QAM16, QAM64, QAM256 -> {
                int k = bitsPerSymbol;
                double m = 1 << k;
                yield 4.0 / k * (1 - 1 / Math.sqrt(m)) * Special.q(Math.sqrt(3.0 * k / (m - 1) * g));
            }
            case FSK, OOK -> 0.5 * Math.exp(-g / 2);
            case AFSK, FM_ANALOG -> 0.5 * Math.exp(-g / 4);
            case AM_ANALOG -> 0.5 * Math.exp(-g / 4 / 3);
            case CHIRP_LORA -> loraBer(g * LORA_SF, LORA_SF);
        };
        return Math.min(0.5, ber);
    }

    /** Bit error rate at a SINR (dB) measured in the receiver bandwidth. */
    public double berAt(double sinrDb) {
        double esN0Db = sinrDb + 10.0 * Math.log10(processingGain);
        return berAtEbN0(esN0Db - 10.0 * Math.log10(bitsPerSymbol));
    }

    /** Packet error rate at {@code sinrDb} for a packet of {@code bits} bits. */
    public double perAt(double sinrDb, int bits) {
        return Per.packetErrorRate(berAt(sinrDb), bits);
    }

    /** Lowest SINR (dB, &plusmn;0.01) giving at most {@code targetBer}; for culling and sensitivity. */
    public double requiredSinrDb(double targetBer) {
        double lo = -60, hi = 80;
        if(berAt(hi) > targetBer) return Double.POSITIVE_INFINITY;
        while(hi - lo > 0.01) {
            double mid = (lo + hi) / 2;
            if(berAt(mid) > targetBer) lo = mid;
            else hi = mid;
        }
        return hi;
    }

    /**
     * LoRa-style chirp BER for a spreading factor {@code sf} at symbol energy
     * E<sub>s</sub>/N<sub>0</sub> = {@code esN0} (linear).
     */
    public static double loraBer(double esN0, int sf) {
        double m = Math.pow(2, sf);
        double ps = Math.min(1.0, Math.exp(Math.log((m - 1) / 2) - esN0 / 2));
        return Math.min(0.5, ps * (m / 2) / (m - 1));
    }

    /** LoRa BER at SINR {@code snrDb} in the chirp bandwidth for spreading factor {@code sf}. */
    public static double loraBerAtSnr(double snrDb, int sf) {
        return loraBer(Units.dbToLinear(snrDb) * Math.pow(2, sf), sf);
    }

    /**
     * Demodulated audio SNR of an analog mode at carrier-to-noise ratio {@code cnrDb}.
     * FM: CNR + FM improvement above threshold, collapsing 3 dB per dB below it.
     * AM (DSB, m = 1): CNR &minus; 4.8 dB. Digital modes return the CNR.
     */
    public double audioSnrDb(double cnrDb) {
        return switch(this) {
            case FM_ANALOG -> cnrDb >= FM_THRESHOLD_DB ? cnrDb + FM_IMPROVEMENT_DB
                    : FM_THRESHOLD_DB + FM_IMPROVEMENT_DB - 3.0 * (FM_THRESHOLD_DB - cnrDb);
            case AM_ANALOG -> cnrDb - 10.0 * Math.log10(3.0);
            default -> cnrDb;
        };
    }
}
