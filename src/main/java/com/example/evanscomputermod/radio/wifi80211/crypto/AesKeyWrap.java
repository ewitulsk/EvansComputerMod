package com.example.evanscomputermod.radio.wifi80211.crypto;

import java.security.MessageDigest;

/**
 * AES Key Wrap (RFC 3394) with the default IV A6A6A6A6A6A6A6A6. 802.11 uses it
 * with the KEK to protect the Key Data field of EAPOL-Key frames (GTK in M3
 * and in group key message 1).
 */
public final class AesKeyWrap {

    private static final byte[] DEFAULT_IV = {
            (byte) 0xA6, (byte) 0xA6, (byte) 0xA6, (byte) 0xA6,
            (byte) 0xA6, (byte) 0xA6, (byte) 0xA6, (byte) 0xA6};

    private AesKeyWrap() {}

    /** Wraps {@code plain} (a multiple of 8 bytes, at least 16) and returns 8 bytes more. */
    public static byte[] wrap(byte[] kek, byte[] plain) {
        if (plain.length < 16 || plain.length % 8 != 0) {
            throw new IllegalArgumentException("key wrap input must be a multiple of 8 bytes, >= 16");
        }
        Aes aes = new Aes(kek);
        int n = plain.length / 8;
        byte[] a = DEFAULT_IV.clone();
        byte[] r = plain.clone();
        byte[] b = new byte[16];
        for (int j = 0; j <= 5; j++) {
            for (int i = 1; i <= n; i++) {
                System.arraycopy(a, 0, b, 0, 8);
                System.arraycopy(r, (i - 1) * 8, b, 8, 8);
                aes.encrypt(b, 0, b, 0);
                long t = (long) n * j + i;
                for (int k = 0; k < 8; k++) a[k] = (byte) (b[k] ^ (t >>> (56 - 8 * k)));
                System.arraycopy(b, 8, r, (i - 1) * 8, 8);
            }
        }
        byte[] out = new byte[plain.length + 8];
        System.arraycopy(a, 0, out, 0, 8);
        System.arraycopy(r, 0, out, 8, plain.length);
        return out;
    }

    /** Unwraps; returns {@code null} if the integrity check value does not match. */
    public static byte[] unwrap(byte[] kek, byte[] wrapped) {
        if (wrapped.length < 24 || wrapped.length % 8 != 0) return null;
        Aes aes = new Aes(kek);
        int n = wrapped.length / 8 - 1;
        byte[] a = new byte[8];
        System.arraycopy(wrapped, 0, a, 0, 8);
        byte[] r = new byte[n * 8];
        System.arraycopy(wrapped, 8, r, 0, n * 8);
        byte[] b = new byte[16];
        for (int j = 5; j >= 0; j--) {
            for (int i = n; i >= 1; i--) {
                long t = (long) n * j + i;
                for (int k = 0; k < 8; k++) b[k] = (byte) (a[k] ^ (t >>> (56 - 8 * k)));
                System.arraycopy(r, (i - 1) * 8, b, 8, 8);
                aes.decrypt(b, 0, b, 0);
                System.arraycopy(b, 0, a, 0, 8);
                System.arraycopy(b, 8, r, (i - 1) * 8, 8);
            }
        }
        return MessageDigest.isEqual(a, DEFAULT_IV) ? r : null;
    }
}
