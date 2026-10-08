package com.example.evanscomputermod.radio.wifi80211.crypto;

import java.util.Arrays;

/**
 * Pairwise transient key for CCMP, split into its three 128-bit parts.
 *
 * @param kck key confirmation key (EAPOL-Key MIC)
 * @param kek key encryption key (AES key wrap of the Key Data field)
 * @param tk  temporal key (CCMP)
 */
public record Ptk(byte[] kck, byte[] kek, byte[] tk) {

    public Ptk {
        if (kck.length != 16 || kek.length != 16 || tk.length != 16) throw new IllegalArgumentException("PTK parts are 16 bytes");
        kck = kck.clone();
        kek = kek.clone();
        tk = tk.clone();
    }

    @Override public byte[] kck() { return kck.clone(); }
    @Override public byte[] kek() { return kek.clone(); }
    @Override public byte[] tk() { return tk.clone(); }

    @Override
    public boolean equals(Object o) {
        return o instanceof Ptk p && Arrays.equals(kck, p.kck) && Arrays.equals(kek, p.kek) && Arrays.equals(tk, p.tk);
    }

    @Override
    public int hashCode() {
        return Arrays.hashCode(tk);
    }

    @Override
    public String toString() {
        return "Ptk[redacted]";
    }
}
