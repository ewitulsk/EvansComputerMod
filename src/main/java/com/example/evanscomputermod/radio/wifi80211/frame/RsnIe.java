package com.example.evanscomputermod.radio.wifi80211.frame;

import java.util.ArrayList;
import java.util.List;

/**
 * RSN element (802.11-2016 9.4.2.25). Cipher and AKM suites are 32-bit
 * values: OUI in the top three octets, suite type in the low octet.
 */
public record RsnIe(int version, int groupCipher, List<Integer> pairwiseCiphers, List<Integer> akms, int capabilities) {

    public static final int SUITE_CCMP = 0x000FAC04;
    public static final int AKM_8021X = 0x000FAC01;
    public static final int AKM_PSK = 0x000FAC02;

    public RsnIe {
        pairwiseCiphers = List.copyOf(pairwiseCiphers);
        akms = List.copyOf(akms);
    }

    /** RSN element advertising WPA2-PSK with CCMP for group and pairwise keys. */
    public static RsnIe wpa2PskCcmp() {
        return new RsnIe(1, SUITE_CCMP, List.of(SUITE_CCMP), List.of(AKM_PSK), 0);
    }

    public boolean isWpa2PskCcmp() {
        return version == 1 && groupCipher == SUITE_CCMP && pairwiseCiphers.contains(SUITE_CCMP) && akms.contains(AKM_PSK);
    }

    public Ie toIe() {
        int n = 2 + 4 + 2 + 4 * pairwiseCiphers.size() + 2 + 4 * akms.size() + 2;
        byte[] d = new byte[n];
        int p = 0;
        d[p++] = (byte) version;
        d[p++] = (byte) (version >>> 8);
        p = suite(d, p, groupCipher);
        d[p++] = (byte) pairwiseCiphers.size();
        d[p++] = (byte) (pairwiseCiphers.size() >>> 8);
        for (int s : pairwiseCiphers) p = suite(d, p, s);
        d[p++] = (byte) akms.size();
        d[p++] = (byte) (akms.size() >>> 8);
        for (int s : akms) p = suite(d, p, s);
        d[p++] = (byte) capabilities;
        d[p] = (byte) (capabilities >>> 8);
        return new Ie(Ie.RSN, d);
    }

    /** The full element bytes (ID, length, data), as compared during the 4-way handshake. */
    public byte[] encode() {
        return toIe().encode();
    }

    /** Parses the data part of an RSN element; optional trailing fields default as the standard says. */
    public static RsnIe parse(byte[] d) {
        if (d.length < 2) throw new IllegalArgumentException("RSN element too short");
        int p = 0;
        int version = (d[0] & 0xFF) | (d[1] & 0xFF) << 8;
        p = 2;
        int group = SUITE_CCMP;
        List<Integer> pairwise = new ArrayList<>(List.of(SUITE_CCMP));
        List<Integer> akms = new ArrayList<>(List.of(AKM_8021X));
        int caps = 0;
        if (p + 4 <= d.length) {
            group = readSuite(d, p);
            p += 4;
        }
        if (p + 2 <= d.length) {
            int count = (d[p] & 0xFF) | (d[p + 1] & 0xFF) << 8;
            p += 2;
            if (p + 4 * count > d.length) throw new IllegalArgumentException("RSN pairwise list overruns");
            pairwise.clear();
            for (int i = 0; i < count; i++, p += 4) pairwise.add(readSuite(d, p));
        }
        if (p + 2 <= d.length) {
            int count = (d[p] & 0xFF) | (d[p + 1] & 0xFF) << 8;
            p += 2;
            if (p + 4 * count > d.length) throw new IllegalArgumentException("RSN AKM list overruns");
            akms.clear();
            for (int i = 0; i < count; i++, p += 4) akms.add(readSuite(d, p));
        }
        if (p + 2 <= d.length) caps = (d[p] & 0xFF) | (d[p + 1] & 0xFF) << 8;
        return new RsnIe(version, group, pairwise, akms, caps);
    }

    private static int suite(byte[] d, int p, int s) {
        d[p] = (byte) (s >>> 24);
        d[p + 1] = (byte) (s >>> 16);
        d[p + 2] = (byte) (s >>> 8);
        d[p + 3] = (byte) s;
        return p + 4;
    }

    private static int readSuite(byte[] d, int p) {
        return (d[p] & 0xFF) << 24 | (d[p + 1] & 0xFF) << 16 | (d[p + 2] & 0xFF) << 8 | (d[p + 3] & 0xFF);
    }
}
