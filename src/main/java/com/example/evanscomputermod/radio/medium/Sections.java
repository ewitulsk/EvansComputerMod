package com.example.evanscomputermod.radio.medium;

/**
 * Packed 16³ chunk-section keys (the same layout as vanilla {@code SectionPos.asLong})
 * plus whole-column keys, used for link-cache dependencies. Pure.
 */
public final class Sections {

    /** Section y used for "the surface of this chunk column" (heightmap middle paths). */
    public static final int COLUMN_Y = -(1 << 19);

    private Sections() {}

    public static long key(int sx, int sy, int sz) {
        return ((long) (sx & 0x3FFFFF) << 42) | ((long) (sy & 0xFFFFF)) | ((long) (sz & 0x3FFFFF) << 20);
    }

    public static long ofBlock(int x, int y, int z) {
        return key(x >> 4, y >> 4, z >> 4);
    }

    public static long column(int cx, int cz) {
        return key(cx, COLUMN_Y, cz);
    }

    public static long columnOfBlock(int x, int z) {
        return column(x >> 4, z >> 4);
    }

    public static int x(long key) {
        return (int) (key >> 42);
    }

    public static int y(long key) {
        return (int) (key << 44 >> 44);
    }

    public static int z(long key) {
        return (int) (key << 22 >> 42);
    }
}
