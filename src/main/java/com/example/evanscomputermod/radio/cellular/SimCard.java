package com.example.evanscomputermod.radio.cellular;

import java.util.HexFormat;
import java.util.Locale;
import java.util.regex.Pattern;

/**
 * The server-side view of a written SIM card: the carrier it belongs to, its subscriber
 * number (IMSI) and its 128-bit secret key K. Built from the card's two components
 * ({@link SimCardItem#sim}) on the server only (clients never have K); the card answers
 * challenges with it ({@code card/SimAuth}, through the {@code sim} peripheral). Pure.
 */
public record SimCard(String carrier, long imsi, String keyHex, String msisdn) {
    /** Carrier ids: 1-24 characters of a-z, 0-9, '-' and '_'. */
    public static final Pattern CARRIER_ID = Pattern.compile("[a-z0-9_-]{1,24}");
    public static final int KEY_BYTES = 16;

    /** A card without a number. */
    public SimCard(String carrier, long imsi, String keyHex) {
        this(carrier, imsi, keyHex, "");
    }

    public SimCard {
        if (msisdn == null) msisdn = "";
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
}
