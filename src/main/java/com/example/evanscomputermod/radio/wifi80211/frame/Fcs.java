package com.example.evanscomputermod.radio.wifi80211.frame;

import java.util.Arrays;
import java.util.zip.CRC32;

/** 802.11 frame check sequence: CRC-32 (IEEE) over header and body, sent little-endian. */
public final class Fcs {

    private Fcs() {}

    public static int compute(byte[] b, int off, int len) {
        CRC32 crc = new CRC32();
        crc.update(b, off, len);
        return (int) crc.getValue();
    }

    /** Returns {@code mpdu} with the 4-byte FCS appended. */
    public static byte[] append(byte[] mpdu) {
        int fcs = compute(mpdu, 0, mpdu.length);
        byte[] out = Arrays.copyOf(mpdu, mpdu.length + 4);
        for (int i = 0; i < 4; i++) out[mpdu.length + i] = (byte) (fcs >>> (8 * i));
        return out;
    }

    public static boolean verify(byte[] withFcs) {
        if (withFcs.length < 4) return false;
        int n = withFcs.length - 4;
        int fcs = compute(withFcs, 0, n);
        for (int i = 0; i < 4; i++) {
            if (withFcs[n + i] != (byte) (fcs >>> (8 * i))) return false;
        }
        return true;
    }

    /** Verifies and strips the FCS; returns {@code null} if it is wrong. */
    public static byte[] strip(byte[] withFcs) {
        return verify(withFcs) ? Arrays.copyOf(withFcs, withFcs.length - 4) : null;
    }
}
