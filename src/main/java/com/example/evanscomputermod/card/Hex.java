package com.example.evanscomputermod.card;

import java.util.HexFormat;

/** Hex helpers for card keys, UIDs, nonces and faces. Lower-case, no separators. */
public final class Hex {
    private Hex() {}

    public static String of(byte[] b) {
        return HexFormat.of().formatHex(b);
    }

    /** Parse exactly {@code bytes} bytes (2*bytes hex digits, either case); {@code what} names the value in errors. */
    public static byte[] parse(String hex, int bytes, String what) {
        if (hex == null || hex.length() != bytes * 2 || !hex.chars().allMatch(c -> Character.digit(c, 16) >= 0))
            throw new IllegalArgumentException(what + " is " + bytes + " bytes (" + bytes * 2 + " hex digits)");
        return HexFormat.of().parseHex(hex);
    }

    /** Parse any even-length hex string. */
    public static byte[] parseAny(String hex, String what) {
        if (hex == null || hex.length() % 2 != 0 || !hex.chars().allMatch(c -> Character.digit(c, 16) >= 0))
            throw new IllegalArgumentException(what + " is not hex");
        return HexFormat.of().parseHex(hex);
    }

    public static String ofLong(long v) {
        return String.format("%016x", v);
    }

    public static byte[] longBytes(long v) {
        byte[] b = new byte[8];
        for (int i = 7; i >= 0; i--) {
            b[i] = (byte) v;
            v >>>= 8;
        }
        return b;
    }

    public static long toLong(byte[] b) {
        long v = 0;
        for (int i = 0; i < 8; i++) v = v << 8 | (b[i] & 0xff);
        return v;
    }
}
