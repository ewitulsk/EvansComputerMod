package com.example.evanscomputermod.radio.wifi80211;

import java.util.Locale;

/**
 * A 48-bit IEEE MAC address. The value holds the six octets big-endian (first
 * octet on the air is the most significant), so numeric order equals the
 * byte-wise order that 802.11i uses for Min/Max in the PTK derivation.
 */
public record MacAddress(long value) implements Comparable<MacAddress> {

    public static final MacAddress BROADCAST = new MacAddress(0xFFFF_FFFF_FFFFL);
    public static final MacAddress ZERO = new MacAddress(0L);

    public MacAddress {
        if ((value & ~0xFFFF_FFFF_FFFFL) != 0) {
            throw new IllegalArgumentException("MAC address wider than 48 bits");
        }
    }

    public static MacAddress of(byte[] b) {
        if (b.length != 6) throw new IllegalArgumentException("MAC address needs 6 bytes");
        return read(b, 0);
    }

    public static MacAddress read(byte[] b, int off) {
        long v = 0;
        for (int i = 0; i < 6; i++) v = (v << 8) | (b[off + i] & 0xFF);
        return new MacAddress(v);
    }

    /** Parses {@code aa:bb:cc:dd:ee:ff} (also accepts '-' separators). */
    public static MacAddress parse(String s) {
        String[] parts = s.trim().split("[:-]");
        if (parts.length != 6) throw new IllegalArgumentException("bad MAC address: " + s);
        long v = 0;
        for (String p : parts) {
            if (p.length() != 2) throw new IllegalArgumentException("bad MAC address: " + s);
            v = (v << 8) | Integer.parseInt(p, 16);
        }
        return new MacAddress(v);
    }

    public void write(byte[] b, int off) {
        for (int i = 0; i < 6; i++) b[off + i] = (byte) (value >>> (40 - 8 * i));
    }

    public byte[] bytes() {
        byte[] b = new byte[6];
        write(b, 0);
        return b;
    }

    /** Individual/group bit (least significant bit of the first octet). */
    public boolean isGroup() {
        return ((value >>> 40) & 1) != 0;
    }

    public boolean isBroadcast() {
        return value == BROADCAST.value;
    }

    @Override
    public int compareTo(MacAddress o) {
        return Long.compare(value, o.value);
    }

    @Override
    public String toString() {
        StringBuilder sb = new StringBuilder(17);
        for (int i = 0; i < 6; i++) {
            if (i > 0) sb.append(':');
            sb.append(String.format(Locale.ROOT, "%02x", (value >>> (40 - 8 * i)) & 0xFF));
        }
        return sb.toString();
    }
}
