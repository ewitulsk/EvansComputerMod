package com.example.evanscomputermod.card.applet;

/**
 * The Sector Lattice {@code Mix} permutation (docs/cards/NFC_MIX.md): 8 rounds of
 * add-rotate-xor over four little-endian 32-bit words, keyed by four key words. A toy,
 * on purpose: it shares nothing with any real contactless card cipher. Every language
 * that speaks to NFC cards (the Rust crate, Python, Lua) implements exactly this and
 * tests against the same vectors.
 */
public final class NfcMix {
    private NfcMix() {}

    public static byte[] mix(byte[] key, byte[] x) {
        if (key == null || key.length != 16 || x == null || x.length != 16) throw new IllegalArgumentException("Mix takes 16-byte key and input");
        int[] k = {le(key, 0), le(key, 4), le(key, 8), le(key, 12)};
        int v0 = le(x, 0), v1 = le(x, 4), v2 = le(x, 8), v3 = le(x, 12);
        for (int r = 0; r < 8; r++) {
            v0 += k[r & 3];
            v1 ^= Integer.rotateLeft(v0, 7);
            v2 += v1;
            v3 ^= Integer.rotateLeft(v2, 13);
            v0 += v3;
            v1 = Integer.rotateLeft(v1, 17) + k[(r + 1) & 3];
            v2 ^= Integer.rotateLeft(v0, 5);
            v3 = (v3 ^ r) + k[(r + 2) & 3];
        }
        byte[] out = new byte[16];
        put(out, 0, v0);
        put(out, 4, v1);
        put(out, 8, v2);
        put(out, 12, v3);
        return out;
    }

    /** {@code R1 || R2} (8 + 8 bytes). */
    public static byte[] concat(byte[] a, byte[] b) {
        byte[] out = new byte[a.length + b.length];
        System.arraycopy(a, 0, out, 0, a.length);
        System.arraycopy(b, 0, out, a.length, b.length);
        return out;
    }

    private static int le(byte[] b, int i) {
        return (b[i] & 0xff) | (b[i + 1] & 0xff) << 8 | (b[i + 2] & 0xff) << 16 | (b[i + 3] & 0xff) << 24;
    }

    private static void put(byte[] b, int i, int v) {
        b[i] = (byte) v;
        b[i + 1] = (byte) (v >>> 8);
        b[i + 2] = (byte) (v >>> 16);
        b[i + 3] = (byte) (v >>> 24);
    }
}
