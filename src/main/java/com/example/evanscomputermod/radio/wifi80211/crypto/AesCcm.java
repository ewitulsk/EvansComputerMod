package com.example.evanscomputermod.radio.wifi80211.crypto;

import java.security.MessageDigest;

/**
 * AES-CCM (RFC 3610 / NIST SP 800-38C), built from CBC-MAC and CTR mode over
 * the raw AES block. The length field size L is {@code 15 - nonce.length};
 * CCMP uses a 13-byte nonce (L = 2) and an 8-byte MIC (M = 8).
 */
public final class AesCcm {

    private AesCcm() {}

    /** Returns ciphertext followed by the {@code micLen}-byte encrypted MIC. */
    public static byte[] encrypt(byte[] key, byte[] nonce, byte[] aad, byte[] plain, int micLen) {
        check(nonce, micLen, plain.length);
        Aes aes = new Aes(key);
        byte[] tag = cbcMac(aes, nonce, aad, plain, 0, plain.length, micLen);
        byte[] out = new byte[plain.length + micLen];
        ctr(aes, nonce, plain, 0, plain.length, out, 0);
        byte[] s0 = counterBlock(aes, nonce, 0);
        for (int i = 0; i < micLen; i++) out[plain.length + i] = (byte) (tag[i] ^ s0[i]);
        return out;
    }

    /** Returns the plaintext, or {@code null} if the MIC does not verify. */
    public static byte[] decrypt(byte[] key, byte[] nonce, byte[] aad, byte[] cipherAndMic, int micLen) {
        int len = cipherAndMic.length - micLen;
        if (len < 0) return null;
        check(nonce, micLen, len);
        Aes aes = new Aes(key);
        byte[] plain = new byte[len];
        ctr(aes, nonce, cipherAndMic, 0, len, plain, 0);
        byte[] tag = cbcMac(aes, nonce, aad, plain, 0, len, micLen);
        byte[] s0 = counterBlock(aes, nonce, 0);
        byte[] received = new byte[micLen];
        for (int i = 0; i < micLen; i++) received[i] = (byte) (cipherAndMic[len + i] ^ s0[i]);
        return MessageDigest.isEqual(tag, received) ? plain : null;
    }

    private static void check(byte[] nonce, int micLen, int msgLen) {
        if (nonce.length < 7 || nonce.length > 13) throw new IllegalArgumentException("CCM nonce must be 7..13 bytes");
        if (micLen < 4 || micLen > 16 || (micLen & 1) != 0) throw new IllegalArgumentException("bad CCM MIC length");
        int l = 15 - nonce.length;
        if (l < 4 && msgLen >= (1 << (8 * l))) throw new IllegalArgumentException("message too long for CCM L=" + l);
    }

    private static byte[] cbcMac(Aes aes, byte[] nonce, byte[] aad, byte[] msg, int off, int len, int micLen) {
        int l = 15 - nonce.length;
        byte[] x = new byte[16];
        x[0] = (byte) ((aad.length > 0 ? 0x40 : 0) | (((micLen - 2) / 2) << 3) | (l - 1));
        System.arraycopy(nonce, 0, x, 1, nonce.length);
        for (int i = 0; i < l; i++) x[15 - i] = (byte) (len >>> (8 * i));
        aes.encrypt(x, 0, x, 0);
        if (aad.length > 0) {
            if (aad.length >= 0xFF00) throw new IllegalArgumentException("CCM AAD too long");
            byte[] a = new byte[2 + aad.length];
            a[0] = (byte) (aad.length >>> 8);
            a[1] = (byte) aad.length;
            System.arraycopy(aad, 0, a, 2, aad.length);
            absorb(aes, x, a, 0, a.length);
        }
        absorb(aes, x, msg, off, len);
        byte[] tag = new byte[micLen];
        System.arraycopy(x, 0, tag, 0, micLen);
        return tag;
    }

    /** CBC-MAC over data, zero-padded to a whole number of blocks. */
    private static void absorb(Aes aes, byte[] x, byte[] data, int off, int len) {
        for (int p = 0; p < len; p += 16) {
            int n = Math.min(16, len - p);
            for (int i = 0; i < n; i++) x[i] ^= data[off + p + i];
            aes.encrypt(x, 0, x, 0);
        }
    }

    private static byte[] counterBlock(Aes aes, byte[] nonce, int counter) {
        int l = 15 - nonce.length;
        byte[] a = new byte[16];
        a[0] = (byte) (l - 1);
        System.arraycopy(nonce, 0, a, 1, nonce.length);
        for (int i = 0; i < l && i < 4; i++) a[15 - i] = (byte) (counter >>> (8 * i));
        aes.encrypt(a, 0, a, 0);
        return a;
    }

    private static void ctr(Aes aes, byte[] nonce, byte[] in, int inOff, int len, byte[] out, int outOff) {
        int counter = 1;
        for (int p = 0; p < len; p += 16, counter++) {
            byte[] s = counterBlock(aes, nonce, counter);
            int n = Math.min(16, len - p);
            for (int i = 0; i < n; i++) out[outOff + p + i] = (byte) (in[inOff + p + i] ^ s[i]);
        }
    }
}
