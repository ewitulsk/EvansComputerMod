package com.example.evanscomputermod.radio.cellular;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.SecureRandom;
import java.util.HexFormat;
import java.util.Locale;
import java.util.regex.Pattern;

/**
 * What a SIM card holds: the carrier it belongs to, its subscriber number (IMSI)
 * and a 128-bit secret key shared with that carrier (Ki). Stored on the SIM item
 * as a data component; the carrier keeps the same key in its subscriber table.
 *
 * <p>Authentication is a challenge proof in both directions, keyed by Ki
 * (HMAC-SHA256, truncated to 8 bytes): the device proves it holds the key in its
 * attach request (over the tower's id, the IMSI, its MAC and a fresh nonce), and
 * the tower proves the carrier knows the key in its accept, so neither a forged
 * SIM nor a fake tower gets far. Pure.
 */
public record SimCard(String carrier, long imsi, String keyHex) {
    /** Carrier ids: 1-24 characters of a-z, 0-9, '-' and '_'. */
    public static final Pattern CARRIER_ID = Pattern.compile("[a-z0-9_-]{1,24}");
    public static final int KEY_BYTES = 16, PROOF_BYTES = 8;

    public SimCard {
        if (carrier == null || !CARRIER_ID.matcher(carrier).matches()) throw new IllegalArgumentException("bad carrier id: " + carrier);
        if (keyHex == null || keyHex.length() != KEY_BYTES * 2) throw new IllegalArgumentException("bad SIM key");
        keyHex = keyHex.toLowerCase(Locale.ROOT);
        HexFormat.of().parseHex(keyHex);
    }

    public byte[] key() {
        return HexFormat.of().parseHex(keyHex);
    }

    /** The IMSI as the 15 digits phones show. */
    public String imsiText() {
        return formatImsi(imsi);
    }

    public static String formatImsi(long imsi) {
        return String.format(Locale.ROOT, "%015d", imsi);
    }

    /** Lower-case a carrier name and check it; null when it isn't a valid id. */
    public static String normaliseCarrier(String s) {
        if (s == null) return null;
        String c = s.trim().toLowerCase(Locale.ROOT);
        return CARRIER_ID.matcher(c).matches() ? c : null;
    }

    public static String newKeyHex(SecureRandom rng) {
        byte[] k = new byte[KEY_BYTES];
        rng.nextBytes(k);
        return HexFormat.of().formatHex(k);
    }

    // ------------------------------------------------------------ proofs

    /** The device's attach proof: HMAC(Ki, "attach" | cell | imsi | mac | nonce), 8 bytes. */
    public static byte[] attachProof(byte[] key, int cellKey, long imsi, byte[] mac, long nonce) {
        ByteBuffer b = ByteBuffer.allocate(6 + 4 + 8 + 6 + 8);
        b.put("attach".getBytes(StandardCharsets.US_ASCII)).putInt(cellKey).putLong(imsi).put(mac, 0, 6).putLong(nonce);
        return hmac(key, b.array());
    }

    /** The tower's accept proof: HMAC(Ki, "accept" | cell | imsi | nonce | rnti), 8 bytes. */
    public static byte[] acceptProof(byte[] key, int cellKey, long imsi, long nonce, int rnti) {
        ByteBuffer b = ByteBuffer.allocate(6 + 4 + 8 + 8 + 2);
        b.put("accept".getBytes(StandardCharsets.US_ASCII)).putInt(cellKey).putLong(imsi).putLong(nonce).putShort((short) rnti);
        return hmac(key, b.array());
    }

    static byte[] hmac(byte[] key, byte[] data) {
        try {
            Mac m = Mac.getInstance("HmacSHA256");
            m.init(new SecretKeySpec(key, "HmacSHA256"));
            byte[] full = m.doFinal(data);
            byte[] out = new byte[PROOF_BYTES];
            System.arraycopy(full, 0, out, 0, PROOF_BYTES);
            return out;
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Constant-time comparison of two proofs. */
    public static boolean proofEquals(byte[] a, byte[] b) {
        return a != null && b != null && java.security.MessageDigest.isEqual(a, b);
    }
}
