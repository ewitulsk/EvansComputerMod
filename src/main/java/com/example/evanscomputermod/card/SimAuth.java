package com.example.evanscomputermod.card;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Locale;
import java.util.regex.Pattern;

/**
 * The SIM smart card's authentication algorithm and the parsing rules for what
 * is written to a card. Pure (no Minecraft), so the carrier core ({@code cored},
 * Rust) can reproduce it bit for bit from its database copy of the key.
 *
 * <h2>Derivation (version 1)</h2>
 * <pre>
 *   K    = the card's secret key, 16 bytes
 *   RAND = the challenge, 16 bytes
 *   RES  = HMAC-SHA256(K, "ECM-SIM-RES" || RAND)[0..16)   16 bytes, the card's answer
 *   KS   = HMAC-SHA256(K, "ECM-SIM-KS"  || RAND)           32 bytes, session key material
 * </pre>
 * The labels are the ASCII bytes without a terminator; {@code ||} is
 * concatenation. The network compares RES with its own computation
 * (constant time) and both sides then hold KS without it ever crossing the air:
 * the cellular protocol uses KS[0..16) as the encryption key and KS[16..32) as
 * the integrity key. The key K itself never leaves the card.
 */
public final class SimAuth {
    public static final int KEY_BYTES = 16;
    public static final int CHALLENGE_BYTES = 16;
    public static final int RES_BYTES = 16;
    public static final int SESSION_BYTES = 32;

    static final byte[] RES_LABEL = "ECM-SIM-RES".getBytes(StandardCharsets.US_ASCII);
    static final byte[] KS_LABEL = "ECM-SIM-KS".getBytes(StandardCharsets.US_ASCII);

    /** Carrier ids: 1-24 characters of a-z, 0-9, '-' and '_' (same as v1). */
    public static final Pattern CARRIER_ID = Pattern.compile("[a-z0-9_-]{1,24}");
    /** IMSIs: 6 to 15 decimal digits (written as 15 digits with leading zeros). */
    public static final Pattern IMSI = Pattern.compile("[0-9]{6,15}");

    private SimAuth() {}

    /** What a card answers to a challenge. */
    public record Response(byte[] res, byte[] sessionKey) {
        public String resHex() {
            return HexFormat.of().formatHex(res);
        }

        public String sessionKeyHex() {
            return HexFormat.of().formatHex(sessionKey);
        }
    }

    /** The card's answer to {@code challenge} with key {@code key} (see the class comment). */
    public static Response respond(byte[] key, byte[] challenge) {
        checkKey(key);
        if (challenge == null || challenge.length != CHALLENGE_BYTES)
            throw new IllegalArgumentException("the challenge must be " + CHALLENGE_BYTES + " bytes");
        byte[] fullRes = hmac(key, RES_LABEL, challenge);
        byte[] res = new byte[RES_BYTES];
        System.arraycopy(fullRes, 0, res, 0, RES_BYTES);
        return new Response(res, hmac(key, KS_LABEL, challenge));
    }

    /** Constant-time check of a card's answer against the expected one. */
    public static boolean verify(byte[] key, byte[] challenge, byte[] res) {
        return res != null && MessageDigest.isEqual(respond(key, challenge).res(), res);
    }

    static byte[] hmac(byte[] key, byte[] label, byte[] data) {
        try {
            Mac m = Mac.getInstance("HmacSHA256");
            m.init(new SecretKeySpec(key, "HmacSHA256"));
            m.update(label);
            return m.doFinal(data);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    private static void checkKey(byte[] key) {
        if (key == null || key.length != KEY_BYTES) throw new IllegalArgumentException("a SIM key is " + KEY_BYTES + " bytes");
    }

    // ------------------------------------------------------------ parsing what a writer is given

    /** {@code text} as an IMSI; throws with a player-readable reason. */
    public static long parseImsi(String text) {
        String t = text == null ? "" : text.trim();
        if (!IMSI.matcher(t).matches()) throw new IllegalArgumentException("an IMSI is 6 to 15 digits, not '" + text + "'");
        return Long.parseLong(t);
    }

    /** {@code text} as a carrier id (lower-cased); throws with a player-readable reason. */
    public static String parseCarrier(String text) {
        String c = text == null ? "" : text.trim().toLowerCase(Locale.ROOT);
        if (!CARRIER_ID.matcher(c).matches())
            throw new IllegalArgumentException("a carrier id is 1-24 characters of a-z, 0-9, '-' and '_', not '" + text + "'");
        return c;
    }

    /** {@code hex} as a 16-byte key; throws with a player-readable reason. */
    public static byte[] parseKey(String hex) {
        String h = hex == null ? "" : hex.trim().toLowerCase(Locale.ROOT);
        if (h.length() != KEY_BYTES * 2 || !h.chars().allMatch(c -> (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')))
            throw new IllegalArgumentException("a SIM key is " + KEY_BYTES * 2 + " hex digits (" + KEY_BYTES + " bytes)");
        return HexFormat.of().parseHex(h);
    }

    /** {@code hex} as a challenge; throws with a player-readable reason. */
    public static byte[] parseChallenge(String hex) {
        String h = hex == null ? "" : hex.trim().toLowerCase(Locale.ROOT);
        if (h.length() != CHALLENGE_BYTES * 2 || !h.chars().allMatch(c -> (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')))
            throw new IllegalArgumentException("a challenge is " + CHALLENGE_BYTES * 2 + " hex digits (" + CHALLENGE_BYTES + " bytes)");
        return HexFormat.of().parseHex(h);
    }

    public static String formatImsi(long imsi) {
        return String.format(Locale.ROOT, "%015d", imsi);
    }
}
