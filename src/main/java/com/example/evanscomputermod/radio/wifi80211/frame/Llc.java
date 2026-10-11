package com.example.evanscomputermod.radio.wifi80211.frame;

import java.util.Arrays;

/**
 * LLC/SNAP encapsulation of Ethernet payloads in 802.11 data frames
 * (RFC 1042, with the 802.1H bridge-tunnel OUI for AARP and IPX as Linux does).
 * 802.3 frames (length instead of EtherType) already carry an LLC header and
 * travel as-is.
 */
public final class Llc {

    private static final int ETHERTYPE_AARP = 0x80F3;
    private static final int ETHERTYPE_IPX = 0x8137;

    private Llc() {}

    /** Result of decapsulation: the EtherType (or 802.3 length) and payload for the Ethernet frame. */
    public record Decap(int etherType, byte[] payload) {}

    public static byte[] encap(int etherType, byte[] payload) {
        if (etherType <= 1500) return payload.clone();
        byte[] b = new byte[8 + payload.length];
        b[0] = (byte) 0xAA;
        b[1] = (byte) 0xAA;
        b[2] = 0x03;
        if (etherType == ETHERTYPE_AARP || etherType == ETHERTYPE_IPX) b[5] = (byte) 0xF8;
        b[6] = (byte) (etherType >>> 8);
        b[7] = (byte) etherType;
        System.arraycopy(payload, 0, b, 8, payload.length);
        return b;
    }

    /** Never returns null; a body without SNAP becomes an 802.3 frame (length, raw LLC payload). */
    public static Decap decap(byte[] body) {
        if (body.length >= 8 && (body[0] & 0xFF) == 0xAA && (body[1] & 0xFF) == 0xAA && body[2] == 0x03
                && body[3] == 0 && body[4] == 0 && (body[5] == 0 || (body[5] & 0xFF) == 0xF8)) {
            int type = (body[6] & 0xFF) << 8 | (body[7] & 0xFF);
            return new Decap(type, Arrays.copyOfRange(body, 8, body.length));
        }
        return new Decap(body.length, body.clone());
    }
}
