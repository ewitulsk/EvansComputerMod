package com.example.evanscomputermod.radio.api;

/**
 * One transmission on the shared medium: what spectrum it occupies, when (on the
 * world airtime clock, microseconds), at what power, and what it carries.
 *
 * <ul>
 *   <li>{@link Kind#FRAME}: a packet radio frame ({@code payload} = raw bytes, e.g.
 *       an 802.11 MPDU); delivered to tuned receivers after a packet-error roll.</li>
 *   <li>{@link Kind#IQ}: SDR output; {@code iq} holds interleaved float I/Q at
 *       {@code sampleRateHz}, centred on the channel.</li>
 *   <li>{@link Kind#ENERGY}: occupies the channel without a decodable payload
 *       (noise bursts, jammers).</li>
 * </ul>
 *
 * @param modulation name of the modulation/MCS (e.g. "OFDM-54", "DSSS-1", "AFSK1200"),
 *                   used by receivers to map SINR to bit error rate
 */
public record Emission(
        Kind kind,
        Channel channel,
        double powerDbm,
        long startMicros,
        long durationMicros,
        String modulation,
        double bitRate,
        byte[] payload,
        float[] iq,
        double sampleRateHz) {

    public enum Kind { FRAME, IQ, ENERGY }

    public long endMicros() {
        return startMicros + durationMicros;
    }

    public boolean overlapsInTime(Emission o) {
        return startMicros < o.endMicros() && o.startMicros < endMicros();
    }

    public static Emission frame(Channel ch, double powerDbm, long startMicros, long durationMicros,
                                 String modulation, double bitRate, byte[] payload) {
        return new Emission(Kind.FRAME, ch, powerDbm, startMicros, durationMicros, modulation, bitRate, payload, null, 0);
    }

    public static Emission energy(Channel ch, double powerDbm, long startMicros, long durationMicros) {
        return new Emission(Kind.ENERGY, ch, powerDbm, startMicros, durationMicros, "ENERGY", 0, null, null, 0);
    }

    public static Emission iq(Channel ch, double powerDbm, long startMicros, float[] iq, double sampleRateHz) {
        long dur = (long) Math.ceil(iq.length / 2.0 / sampleRateHz * 1e6);
        return new Emission(Kind.IQ, ch, powerDbm, startMicros, dur, "IQ", 0, null, iq, sampleRateHz);
    }
}
