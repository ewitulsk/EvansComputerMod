package com.example.evanscomputermod.radio.wifi80211.crypto;

import javax.crypto.Cipher;
import javax.crypto.spec.SecretKeySpec;
import java.security.GeneralSecurityException;

/**
 * Raw AES block cipher (JDK AES/ECB/NoPadding) for one key. CCM and key wrap
 * are built on top of single-block operations. Not thread-safe.
 */
final class Aes {

    private final Cipher enc;
    private final Cipher dec;

    Aes(byte[] key) {
        if (key.length != 16 && key.length != 24 && key.length != 32) {
            throw new IllegalArgumentException("AES key must be 16, 24 or 32 bytes");
        }
        try {
            SecretKeySpec spec = new SecretKeySpec(key, "AES");
            enc = Cipher.getInstance("AES/ECB/NoPadding");
            enc.init(Cipher.ENCRYPT_MODE, spec);
            dec = Cipher.getInstance("AES/ECB/NoPadding");
            dec.init(Cipher.DECRYPT_MODE, spec);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException("AES unavailable", e);
        }
    }

    /** Encrypts the 16-byte block at {@code in[inOff]} into {@code out[outOff]}. */
    void encrypt(byte[] in, int inOff, byte[] out, int outOff) {
        try {
            enc.doFinal(in, inOff, 16, out, outOff);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    void decrypt(byte[] in, int inOff, byte[] out, int outOff) {
        try {
            dec.doFinal(in, inOff, 16, out, outOff);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }
}
