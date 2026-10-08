package com.example.evanscomputermod.radio.wifi80211.crypto;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

import javax.crypto.Mac;
import javax.crypto.SecretKeyFactory;
import javax.crypto.spec.PBEKeySpec;
import javax.crypto.spec.SecretKeySpec;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.util.Arrays;

/**
 * WPA2-PSK key hierarchy (IEEE 802.11-2016 12.7.1): PSK to PMK, the 802.11i
 * PRF, the pairwise transient key and the EAPOL-Key MIC (key descriptor
 * version 2, HMAC-SHA1-128).
 */
public final class Wpa2Crypto {

    public static final int PMK_LEN = 32;
    public static final int NONCE_LEN = 32;
    public static final int MIC_LEN = 16;
    public static final String PAIRWISE_LABEL = "Pairwise key expansion";

    private Wpa2Crypto() {}

    /** True if {@code passphrase} is a valid WPA2 passphrase (8..63 printable ASCII) or 64 hex digits. */
    public static boolean isValidPassphrase(String passphrase) {
        if (passphrase == null) return false;
        if (passphrase.length() == 64) return passphrase.chars().allMatch(c -> Character.digit(c, 16) >= 0);
        if (passphrase.length() < 8 || passphrase.length() > 63) return false;
        return passphrase.chars().allMatch(c -> c >= 32 && c <= 126);
    }

    /**
     * PMK = PBKDF2-HMAC-SHA1(passphrase, SSID, 4096, 256). A 64-hex-digit
     * string is taken as the raw PSK, as wpa_supplicant does.
     */
    public static byte[] pmk(String passphrase, byte[] ssid) {
        if (!isValidPassphrase(passphrase)) {
            throw new IllegalArgumentException("passphrase must be 8..63 printable ASCII characters or 64 hex digits");
        }
        if (ssid.length < 1 || ssid.length > 32) throw new IllegalArgumentException("SSID must be 1..32 bytes");
        if (passphrase.length() == 64) return hex(passphrase);
        try {
            PBEKeySpec spec = new PBEKeySpec(passphrase.toCharArray(), ssid, 4096, PMK_LEN * 8);
            return SecretKeyFactory.getInstance("PBKDF2WithHmacSHA1").generateSecret(spec).getEncoded();
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException("PBKDF2WithHmacSHA1 unavailable", e);
        }
    }

    public static byte[] pmk(String passphrase, String ssid) {
        return pmk(passphrase, ssid.getBytes(StandardCharsets.UTF_8));
    }

    /**
     * PRF-n(K, A, B) = HMAC-SHA1(K, A || 0 || B || i) for i = 0, 1, ..., truncated to n bits.
     */
    public static byte[] prf(byte[] key, String label, byte[] data, int bits) {
        if (bits <= 0 || bits % 8 != 0) throw new IllegalArgumentException("PRF length must be a positive multiple of 8 bits");
        byte[] a = label.getBytes(StandardCharsets.US_ASCII);
        Mac mac = hmacSha1(key);
        byte[] out = new byte[bits / 8];
        int pos = 0;
        for (int i = 0; pos < out.length; i++) {
            mac.update(a);
            mac.update((byte) 0);
            mac.update(data);
            mac.update((byte) i);
            byte[] h = mac.doFinal();
            int n = Math.min(h.length, out.length - pos);
            System.arraycopy(h, 0, out, pos, n);
            pos += n;
        }
        return out;
    }

    /**
     * PTK = PRF-384(PMK, "Pairwise key expansion",
     * Min(AA,SPA) || Max(AA,SPA) || Min(ANonce,SNonce) || Max(ANonce,SNonce)).
     */
    public static Ptk derivePtk(byte[] pmk, MacAddress aa, MacAddress spa, byte[] anonce, byte[] snonce) {
        if (anonce.length != NONCE_LEN || snonce.length != NONCE_LEN) throw new IllegalArgumentException("nonces are 32 bytes");
        byte[] data = new byte[6 + 6 + NONCE_LEN + NONCE_LEN];
        MacAddress lo = aa.compareTo(spa) <= 0 ? aa : spa;
        MacAddress hi = lo == aa ? spa : aa;
        lo.write(data, 0);
        hi.write(data, 6);
        boolean anonceFirst = Arrays.compareUnsigned(anonce, snonce) <= 0;
        System.arraycopy(anonceFirst ? anonce : snonce, 0, data, 12, NONCE_LEN);
        System.arraycopy(anonceFirst ? snonce : anonce, 0, data, 12 + NONCE_LEN, NONCE_LEN);
        byte[] ptk = prf(pmk, PAIRWISE_LABEL, data, 384);
        return new Ptk(Arrays.copyOfRange(ptk, 0, 16), Arrays.copyOfRange(ptk, 16, 32), Arrays.copyOfRange(ptk, 32, 48));
    }

    /** HMAC-SHA1 over the whole EAPOL frame (MIC field zeroed by the caller), truncated to 128 bits. */
    public static byte[] eapolMic(byte[] kck, byte[] eapolFrameWithZeroMic) {
        byte[] h = hmacSha1(kck).doFinal(eapolFrameWithZeroMic);
        return Arrays.copyOf(h, MIC_LEN);
    }

    public static boolean micEquals(byte[] a, byte[] b) {
        return MessageDigest.isEqual(a, b);
    }

    static Mac hmacSha1(byte[] key) {
        try {
            Mac mac = Mac.getInstance("HmacSHA1");
            mac.init(new SecretKeySpec(key, "HmacSHA1"));
            return mac;
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException("HmacSHA1 unavailable", e);
        }
    }

    private static byte[] hex(String s) {
        byte[] out = new byte[s.length() / 2];
        for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        return out;
    }
}
