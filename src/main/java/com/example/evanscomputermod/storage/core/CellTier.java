package com.example.evanscomputermod.storage.core;

//? if <=1.21.1 {

import java.util.Locale;

/**
 * Storage Cell capacity tiers, with AE2's accounting: a cell of {@code k}
 * kilobytes has {@code k * 1024} bytes, each stored item type costs
 * {@code bytes / 128} bytes of overhead, every {@link #ITEMS_PER_BYTE} items
 * cost one byte, and a cell holds at most {@link #MAX_TYPES} types.
 */
public enum CellTier {
    K1(1),
    K4(4),
    K16(16),
    K64(64);

    public static final int MAX_TYPES = 63;
    public static final int ITEMS_PER_BYTE = 8;

    private final int kilobytes;

    CellTier(int kilobytes) {
        this.kilobytes = kilobytes;
    }

    public int kilobytes() {
        return kilobytes;
    }

    public long bytes() {
        return kilobytes * 1024L;
    }

    public long bytesPerType() {
        return bytes() / 128;
    }

    /** "1k", "4k", ... */
    public String label() {
        return kilobytes + "k";
    }

    public String id() {
        return name().toLowerCase(Locale.ROOT);
    }

    public static CellTier byId(String id) {
        for (CellTier t : values()) {
            if (t.id().equals(id) || t.label().equals(id)) return t;
        }
        return K1;
    }
}
//?}
