package com.example.evanscomputermod.bluetooth;

import com.example.evanscomputermod.radio.api.Channel;

import java.util.UUID;

/**
 * Bluetooth addresses (48 bits, shown {@code AC:DE:48:00:11:22}) and the radio
 * channels Bluetooth uses: one advertising channel where devices announce
 * themselves and pair, and 37 data channels a link moves to once connected
 * (docs/BLUETOOTH.md "Radio"). Pure Java.
 */
public final class BtAddress {

    /** Width of a Bluetooth channel. */
    public static final double BANDWIDTH_HZ = 2e6;
    /** The advertising channel (2402 MHz, Bluetooth LE's channel 37). */
    public static final Channel ADV = new Channel(2.402e9, BANDWIDTH_HZ);
    public static final int DATA_CHANNELS = 37;

    private BtAddress() {}

    /** A stable address for a device id: Apple's OUI-like prefix bits cleared, never zero. */
    public static long of(UUID id) {
        long a = (id.getMostSignificantBits() ^ id.getLeastSignificantBits() * 31) & 0xFFFF_FFFF_FFFFL;
        a &= ~0x0100_0000_0000L; // unicast
        return a == 0 ? 1 : a;
    }

    public static String format(long a) {
        StringBuilder sb = new StringBuilder(17);
        for (int i = 5; i >= 0; i--) {
            if (sb.length() > 0) sb.append(':');
            sb.append(String.format("%02X", (a >>> (8 * i)) & 255));
        }
        return sb.toString();
    }

    /** Parse {@code AC:DE:48:00:11:22} (any case), or -1. */
    public static long parse(String s) {
        if (s == null) return -1;
        String[] p = s.trim().split(":");
        if (p.length != 6) return -1;
        long a = 0;
        for (String x : p) {
            if (x.length() != 2) return -1;
            try {
                a = (a << 8) | Integer.parseInt(x, 16);
            } catch (NumberFormatException e) {
                return -1;
            }
        }
        return a;
    }

    /** Data channel {@code n} (0-36): 2404-2478 MHz in 2 MHz steps, skipping 2426 (an advertising frequency). */
    public static Channel data(int n) {
        int k = Math.floorMod(n, DATA_CHANNELS);
        double mhz = 2404 + 2 * k;
        if (mhz >= 2426) mhz += 2;
        return new Channel(mhz * 1e6, BANDWIDTH_HZ);
    }

    /** The data channel a phone picks for a headset (spreads links across the band). */
    public static int channelFor(long phone, long headset) {
        return (int) Math.floorMod(phone * 31 + headset, (long) DATA_CHANNELS);
    }
}
