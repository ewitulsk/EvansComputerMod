package com.example.evanscomputermod.radio.controller;

import java.nio.ByteBuffer;
import java.util.UUID;

/**
 * The over-the-air input report a Wireless Controller sends on 2.4 GHz: a small
 * fixed frame (43 bytes) carrying the controller, the player holding it and the
 * full button/axis snapshot. Pure Java so it can be unit tested.
 *
 * <pre>
 *  0  2  magic 0xEC 0x43
 *  2  1  version (1)
 *  3 16  controller id
 * 19 16  player id
 * 35  4  buttons (big endian)
 * 39  4  lx, ly, rx, ry (signed bytes)
 * 43  2  lt, rt (unsigned bytes)
 * </pre>
 */
public record ControllerFrame(UUID controllerId, UUID playerId, int buttons, int lx, int ly, int rx, int ry, int lt, int rt) {

    public static final int LENGTH = 45;
    private static final byte M0 = (byte) 0xEC, M1 = 0x43, VERSION = 1;

    public byte[] encode() {
        ByteBuffer b = ByteBuffer.allocate(LENGTH);
        b.put(M0).put(M1).put(VERSION);
        b.putLong(controllerId.getMostSignificantBits()).putLong(controllerId.getLeastSignificantBits());
        b.putLong(playerId.getMostSignificantBits()).putLong(playerId.getLeastSignificantBits());
        b.putInt(buttons);
        b.put((byte) lx).put((byte) ly).put((byte) rx).put((byte) ry);
        b.put((byte) lt).put((byte) rt);
        return b.array();
    }

    /** Decode a report, or null if {@code data} isn't one. */
    public static ControllerFrame decode(byte[] data) {
        if (data == null || data.length != LENGTH || data[0] != M0 || data[1] != M1 || data[2] != VERSION) return null;
        ByteBuffer b = ByteBuffer.wrap(data, 3, LENGTH - 3);
        UUID c = new UUID(b.getLong(), b.getLong());
        UUID p = new UUID(b.getLong(), b.getLong());
        int buttons = b.getInt();
        int lx = b.get(), ly = b.get(), rx = b.get(), ry = b.get();
        int lt = b.get() & 255, rt = b.get() & 255;
        return new ControllerFrame(c, p, buttons, lx, ly, rx, ry, lt, rt);
    }

    /** Airtime of one report at the controller's 1 Mb/s GFSK-like rate (preamble + payload), µs. */
    public static long airtimeMicros() {
        return 80 + LENGTH * 8L;
    }
}
