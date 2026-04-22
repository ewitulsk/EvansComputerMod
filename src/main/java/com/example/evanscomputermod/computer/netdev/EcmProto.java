package com.example.evanscomputermod.computer.netdev;

/**
 * EvansComputerMod network-device protocol ("ECM1") — a tiny request/reply
 * format carried in UDP. Every payload starts with {@link #MAGIC} so clients
 * can sanity-check before parsing.
 *
 * <pre>
 *  0               1               2               3
 *  0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
 * +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
 * |                       Magic 'E' 'C' 'M' '1'                   |
 * +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
 * |   Type        |   Kind        |           Seq                 |
 * +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
 * |                         Payload...                            |
 * +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
 * </pre>
 */
public final class EcmProto {
    private EcmProto() {}

    /** "ECM1" */
    public static final byte[] MAGIC = { 'E', 'C', 'M', '1' };

    public static final int DEFAULT_PORT = 4401;
    public static final int HEADER_LEN = 8;

    // Message types
    public static final int TYPE_PING            = 1;
    public static final int TYPE_PONG            = 2;
    public static final int TYPE_DESCRIBE        = 3;
    public static final int TYPE_DESCRIBE_REPLY  = 4;
    public static final int TYPE_ERROR           = 5;

    public static final int TYPE_REDSTONE_SET    = 16;
    public static final int TYPE_REDSTONE_ACK    = 17;
    public static final int TYPE_REDSTONE_GET    = 18;
    public static final int TYPE_REDSTONE_REPORT = 19;

    // Device kinds
    public static final int KIND_UNKNOWN  = 0;
    public static final int KIND_REDSTONE = 1;

    public static boolean hasMagic(byte[] buf, int off, int len) {
        if (len < HEADER_LEN) return false;
        return buf[off] == MAGIC[0] && buf[off + 1] == MAGIC[1]
                && buf[off + 2] == MAGIC[2] && buf[off + 3] == MAGIC[3];
    }

    public static int getType(byte[] buf, int off) { return buf[off + 4] & 0xff; }
    public static int getKind(byte[] buf, int off) { return buf[off + 5] & 0xff; }
    public static int getSeq(byte[] buf, int off) {
        return ((buf[off + 6] & 0xff) << 8) | (buf[off + 7] & 0xff);
    }

    /** Writes {@link #HEADER_LEN} bytes and returns {@code off + HEADER_LEN}. */
    public static int writeHeader(byte[] buf, int off, int type, int kind, int seq) {
        buf[off]     = MAGIC[0];
        buf[off + 1] = MAGIC[1];
        buf[off + 2] = MAGIC[2];
        buf[off + 3] = MAGIC[3];
        buf[off + 4] = (byte) type;
        buf[off + 5] = (byte) kind;
        buf[off + 6] = (byte) ((seq >> 8) & 0xff);
        buf[off + 7] = (byte) (seq & 0xff);
        return off + HEADER_LEN;
    }
}
