package com.example.evanscomputermod.radio.wifi80211.frame;

/**
 * Frame Control field constants. The field is handled as a 16-bit value read
 * little-endian from the air, so the first octet holds version/type/subtype
 * and the second the flags.
 */
public final class FrameControl {

    private FrameControl() {}

    public static final int TYPE_MGMT = 0;
    public static final int TYPE_CTRL = 1;
    public static final int TYPE_DATA = 2;

    // Management subtypes
    public static final int ASSOC_REQ = 0;
    public static final int ASSOC_RESP = 1;
    public static final int REASSOC_REQ = 2;
    public static final int REASSOC_RESP = 3;
    public static final int PROBE_REQ = 4;
    public static final int PROBE_RESP = 5;
    public static final int BEACON = 8;
    public static final int ATIM = 9;
    public static final int DISASSOC = 10;
    public static final int AUTH = 11;
    public static final int DEAUTH = 12;
    public static final int ACTION = 13;

    // Control subtypes
    public static final int BLOCK_ACK_REQ = 8;
    public static final int BLOCK_ACK = 9;
    public static final int PS_POLL = 10;
    public static final int RTS = 11;
    public static final int CTS = 12;
    public static final int ACK = 13;
    public static final int CF_END = 14;

    // Data subtypes
    public static final int DATA = 0;
    public static final int NULL = 4;
    public static final int QOS_DATA = 8;
    public static final int QOS_NULL = 12;

    // Flag bits (in the 16-bit little-endian view)
    public static final int TO_DS = 0x0100;
    public static final int FROM_DS = 0x0200;
    public static final int MORE_FRAG = 0x0400;
    public static final int RETRY = 0x0800;
    public static final int PWR_MGT = 0x1000;
    public static final int MORE_DATA = 0x2000;
    public static final int PROTECTED = 0x4000;
    public static final int ORDER = 0x8000;

    public static int of(int type, int subtype) {
        return ((type & 3) << 2) | ((subtype & 15) << 4);
    }

    public static int of(int type, int subtype, int flags) {
        return of(type, subtype) | flags;
    }

    public static int version(int fc) { return fc & 3; }
    public static int type(int fc) { return (fc >>> 2) & 3; }
    public static int subtype(int fc) { return (fc >>> 4) & 15; }
    public static boolean toDs(int fc) { return (fc & TO_DS) != 0; }
    public static boolean fromDs(int fc) { return (fc & FROM_DS) != 0; }
    public static boolean isProtected(int fc) { return (fc & PROTECTED) != 0; }

    /** QoS data subtypes have bit 3 of the subtype set. */
    public static boolean isQosData(int fc) {
        return type(fc) == TYPE_DATA && (subtype(fc) & 8) != 0;
    }

    /** Data subtypes with bit 2 set (Null, QoS Null, CF-*) carry no frame body. */
    public static boolean isNullData(int fc) {
        return type(fc) == TYPE_DATA && (subtype(fc) & 4) != 0;
    }
}
