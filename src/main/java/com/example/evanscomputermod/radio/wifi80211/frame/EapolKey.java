package com.example.evanscomputermod.radio.wifi80211.frame;

import java.util.Arrays;

/**
 * EAPOL-Key frame (IEEE 802.1X-2004 header + 802.11-2016 12.7.2 RSN key
 * descriptor). {@link #encode()} produces the whole EAPOL PDU, which is what
 * the MIC covers and what travels after the LLC/SNAP header (EtherType 0x888E).
 *
 * <pre>
 * 0  version  1  type(3)  2  body length (BE)
 * 4  descriptor type (2 = RSN)
 * 5  key information (BE)    7  key length (BE)   9  replay counter (8, BE)
 * 17 key nonce (32)          49 EAPOL-Key IV (16) 65 key RSC (8)
 * 73 reserved (8)            81 key MIC (16)      97 key data length (BE)
 * 99 key data
 * </pre>
 */
public record EapolKey(int protocolVersion, int descriptorType, int keyInfo, int keyLength, long replayCounter,
                       byte[] nonce, byte[] iv, byte[] rsc, byte[] mic, byte[] keyData) {

    public static final int EAPOL_TYPE_KEY = 3;
    public static final int DESC_RSN = 2;
    public static final int MIC_OFFSET = 81;
    public static final int HEADER_LEN = 99;

    // Key Information bits
    public static final int KI_VERSION_MASK = 0x0007;
    public static final int KI_VERSION_HMAC_SHA1_AES = 2;
    public static final int KI_PAIRWISE = 0x0008;
    public static final int KI_INSTALL = 0x0040;
    public static final int KI_ACK = 0x0080;
    public static final int KI_MIC = 0x0100;
    public static final int KI_SECURE = 0x0200;
    public static final int KI_ERROR = 0x0400;
    public static final int KI_REQUEST = 0x0800;
    public static final int KI_ENCRYPTED_DATA = 0x1000;

    /** 4-way handshake message 1..4 and group key message 1..2 Key Information values. */
    public static final int KI_M1 = KI_VERSION_HMAC_SHA1_AES | KI_PAIRWISE | KI_ACK;
    public static final int KI_M2 = KI_VERSION_HMAC_SHA1_AES | KI_PAIRWISE | KI_MIC;
    public static final int KI_M3 = KI_VERSION_HMAC_SHA1_AES | KI_PAIRWISE | KI_INSTALL | KI_ACK | KI_MIC | KI_SECURE | KI_ENCRYPTED_DATA;
    public static final int KI_M4 = KI_VERSION_HMAC_SHA1_AES | KI_PAIRWISE | KI_MIC | KI_SECURE;
    public static final int KI_G1 = KI_VERSION_HMAC_SHA1_AES | KI_ACK | KI_MIC | KI_SECURE | KI_ENCRYPTED_DATA;
    public static final int KI_G2 = KI_VERSION_HMAC_SHA1_AES | KI_MIC | KI_SECURE;

    public EapolKey {
        nonce = fixed(nonce, 32, "nonce");
        iv = fixed(iv, 16, "IV");
        rsc = fixed(rsc, 8, "RSC");
        mic = fixed(mic, 16, "MIC");
        keyData = keyData == null ? new byte[0] : keyData.clone();
    }

    /** A version-2 RSN EAPOL-Key with zero IV, reserved and MIC fields. */
    public static EapolKey of(int keyInfo, int keyLength, long replayCounter, byte[] nonce, byte[] rsc, byte[] keyData) {
        return new EapolKey(2, DESC_RSN, keyInfo, keyLength, replayCounter, nonce, null, rsc, null, keyData);
    }

    private static byte[] fixed(byte[] v, int len, String name) {
        if (v == null) return new byte[len];
        if (v.length != len) throw new IllegalArgumentException(name + " must be " + len + " bytes");
        return v.clone();
    }

    @Override public byte[] nonce() { return nonce.clone(); }
    @Override public byte[] iv() { return iv.clone(); }
    @Override public byte[] rsc() { return rsc.clone(); }
    @Override public byte[] mic() { return mic.clone(); }
    @Override public byte[] keyData() { return keyData.clone(); }

    public boolean has(int bits) {
        return (keyInfo & bits) == bits;
    }

    public int descriptorVersion() {
        return keyInfo & KI_VERSION_MASK;
    }

    public EapolKey withMic(byte[] newMic) {
        return new EapolKey(protocolVersion, descriptorType, keyInfo, keyLength, replayCounter, nonce, iv, rsc, newMic, keyData);
    }

    /** The whole EAPOL PDU. */
    public byte[] encode() {
        int bodyLen = HEADER_LEN - 4 + keyData.length;
        byte[] b = new byte[4 + bodyLen];
        b[0] = (byte) protocolVersion;
        b[1] = EAPOL_TYPE_KEY;
        b[2] = (byte) (bodyLen >>> 8);
        b[3] = (byte) bodyLen;
        b[4] = (byte) descriptorType;
        b[5] = (byte) (keyInfo >>> 8);
        b[6] = (byte) keyInfo;
        b[7] = (byte) (keyLength >>> 8);
        b[8] = (byte) keyLength;
        for (int i = 0; i < 8; i++) b[9 + i] = (byte) (replayCounter >>> (56 - 8 * i));
        System.arraycopy(nonce, 0, b, 17, 32);
        System.arraycopy(iv, 0, b, 49, 16);
        System.arraycopy(rsc, 0, b, 65, 8);
        System.arraycopy(mic, 0, b, MIC_OFFSET, 16);
        b[97] = (byte) (keyData.length >>> 8);
        b[98] = (byte) keyData.length;
        System.arraycopy(keyData, 0, b, HEADER_LEN, keyData.length);
        return b;
    }

    /** The encoded PDU with the MIC field zeroed, i.e. the MIC input. */
    public byte[] encodeForMic() {
        byte[] b = encode();
        Arrays.fill(b, MIC_OFFSET, MIC_OFFSET + 16, (byte) 0);
        return b;
    }

    /**
     * Parses an EAPOL PDU that carries an EAPOL-Key frame. Returns {@code null} for
     * other EAPOL packet types. Trailing bytes beyond the body length are ignored.
     */
    public static EapolKey parse(byte[] b) {
        if (b.length < 4) throw new IllegalArgumentException("EAPOL header truncated");
        if ((b[1] & 0xFF) != EAPOL_TYPE_KEY) return null;
        int bodyLen = (b[2] & 0xFF) << 8 | (b[3] & 0xFF);
        if (b.length < 4 + bodyLen || bodyLen < HEADER_LEN - 4) throw new IllegalArgumentException("EAPOL-Key truncated");
        int kdl = (b[97] & 0xFF) << 8 | (b[98] & 0xFF);
        if (HEADER_LEN + kdl > 4 + bodyLen) throw new IllegalArgumentException("key data overruns EAPOL body");
        long rc = 0;
        for (int i = 0; i < 8; i++) rc = (rc << 8) | (b[9 + i] & 0xFF);
        return new EapolKey(b[0] & 0xFF, b[4] & 0xFF, (b[5] & 0xFF) << 8 | (b[6] & 0xFF), (b[7] & 0xFF) << 8 | (b[8] & 0xFF),
                rc, Arrays.copyOfRange(b, 17, 49), Arrays.copyOfRange(b, 49, 65), Arrays.copyOfRange(b, 65, 73),
                Arrays.copyOfRange(b, MIC_OFFSET, MIC_OFFSET + 16), Arrays.copyOfRange(b, HEADER_LEN, HEADER_LEN + kdl));
    }

    /** 6-byte PN as an 8-byte Key RSC field (little-endian, PN0 first). */
    public static byte[] rscFromPn(long pn) {
        byte[] r = new byte[8];
        for (int i = 0; i < 6; i++) r[i] = (byte) (pn >>> (8 * i));
        return r;
    }

    public static long pnFromRsc(byte[] rsc) {
        long pn = 0;
        for (int i = 5; i >= 0; i--) pn = (pn << 8) | (rsc[i] & 0xFF);
        return pn;
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof EapolKey k && Arrays.equals(encode(), k.encode());
    }

    @Override
    public int hashCode() {
        return Arrays.hashCode(encode());
    }

    @Override
    public String toString() {
        return String.format("EapolKey[info=0x%04x, len=%d, replay=%d, data=%dB]", keyInfo, keyLength, replayCounter, keyData.length);
    }
}
