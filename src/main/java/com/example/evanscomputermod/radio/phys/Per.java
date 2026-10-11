package com.example.evanscomputermod.radio.phys;

/** Packet error rate from bit error rate, assuming independent bit errors and no FEC. */
public final class Per {
    private Per() {}

    /** PER = 1 &minus; (1 &minus; BER)<sup>bits</sup>, computed stably for tiny BER. */
    public static double packetErrorRate(double ber, int bits) {
        if(bits <= 0 || ber <= 0) return 0;
        if(ber >= 1) return 1;
        return -Math.expm1(bits * Math.log1p(-ber));
    }

    /** PER of a packet of {@code bytes} bytes. */
    public static double packetErrorRateBytes(double ber, int bytes) {
        return packetErrorRate(ber, bytes * 8);
    }

    /**
     * One deterministic roll: true when the packet is lost. {@code seed} should
     * mix the link, the frame and the world airtime so replays agree.
     */
    public static boolean lost(double per, long seed) {
        double u = (Fading.mix64(seed) >>> 11) * 0x1.0p-53;
        return u < per;
    }
}
