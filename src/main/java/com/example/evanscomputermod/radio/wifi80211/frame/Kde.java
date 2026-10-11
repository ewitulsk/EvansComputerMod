package com.example.evanscomputermod.radio.wifi80211.frame;

import java.io.ByteArrayOutputStream;
import java.util.Arrays;

/**
 * Key Data Encapsulation (802.11-2016 12.7.2, Table 12-6): vendor-specific
 * elements with OUI 00-0F-AC inside the EAPOL-Key Key Data field, and the
 * padding rule for AES key wrap.
 */
public final class Kde {

    public static final int TYPE_GTK = 1;

    private Kde() {}

    /** A decoded GTK KDE. */
    public record Gtk(int keyId, boolean tx, byte[] key) {}

    /** GTK KDE: DD len 00-0F-AC 01 | KeyID(bits 0-1) Tx(bit 2) | reserved | GTK. */
    public static byte[] gtk(int keyId, boolean tx, byte[] gtk) {
        byte[] b = new byte[2 + 4 + 2 + gtk.length];
        b[0] = (byte) 0xDD;
        b[1] = (byte) (4 + 2 + gtk.length);
        b[2] = 0x00;
        b[3] = 0x0F;
        b[4] = (byte) 0xAC;
        b[5] = TYPE_GTK;
        b[6] = (byte) ((keyId & 3) | (tx ? 4 : 0));
        System.arraycopy(gtk, 0, b, 8, gtk.length);
        return b;
    }

    /**
     * Pads Key Data for AES key wrap: if shorter than 16 bytes or not a multiple
     * of 8, append 0xDD followed by zeros.
     */
    public static byte[] pad(byte[] keyData) {
        int len = keyData.length;
        if (len >= 16 && len % 8 == 0) return keyData.clone();
        int padded = Math.max(16, (len + 1 + 7) / 8 * 8);
        byte[] out = Arrays.copyOf(keyData, padded);
        out[len] = (byte) 0xDD;
        return out;
    }

    public static byte[] concat(byte[]... parts) {
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        for (byte[] p : parts) bos.writeBytes(p);
        return bos.toByteArray();
    }

    /** Finds the GTK KDE in (unwrapped) Key Data; returns {@code null} if absent or malformed. */
    public static Gtk findGtk(byte[] keyData) {
        int p = 0;
        while (p + 2 <= keyData.length) {
            int id = keyData[p] & 0xFF;
            int len = keyData[p + 1] & 0xFF;
            if (id == 0xDD && len == 0) break; // start of padding
            if (p + 2 + len > keyData.length) return null;
            if (id == 0xDD && len >= 6 && keyData[p + 2] == 0x00 && keyData[p + 3] == 0x0F
                    && (keyData[p + 4] & 0xFF) == 0xAC && keyData[p + 5] == TYPE_GTK) {
                int flags = keyData[p + 6] & 0xFF;
                return new Gtk(flags & 3, (flags & 4) != 0, Arrays.copyOfRange(keyData, p + 8, p + 2 + len));
            }
            p += 2 + len;
        }
        return null;
    }

    /** Returns the first element with {@code id} in Key Data (full bytes, ID and length included), or null. */
    public static byte[] findElement(byte[] keyData, int id) {
        int p = 0;
        while (p + 2 <= keyData.length) {
            int eid = keyData[p] & 0xFF;
            int len = keyData[p + 1] & 0xFF;
            if (eid == 0xDD && len == 0) break;
            if (p + 2 + len > keyData.length) return null;
            if (eid == id) return Arrays.copyOfRange(keyData, p, p + 2 + len);
            p += 2 + len;
        }
        return null;
    }
}
