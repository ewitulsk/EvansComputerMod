package com.example.evanscomputermod.radio.framed;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * The binary records of a framed radio's device file ({@code /dev/sdr<N>.frames},
 * {@code /dev/frames.<attachment>}), little-endian. Every record starts with
 * {@code u16 len} (the whole record, header included) and {@code u8 type}.
 * Documented in docs/radio/SDR_PROGRAMS.md "Framed PHY mode" and mirrored by
 * {@code ecm_radio::framed} (Rust) and {@code radio.Framed} (Python).
 *
 * <pre>
 * written by the program
 *   TX      0x01  u16 len, u8 type, u8 flags (bit0 HARQ: ask for an ACK and retransmit),
 *                 u32 dst (0xFFFFFFFF = broadcast), u64 slot (0 = the next slot),
 *                 u8 cqi (0 = automatic), u8 subchannel (0 = whole channel, 1..4 = a quarter),
 *                 u16 tag (echoed in the TX status); payload (header 20 bytes)
 *   BEACON  0x02  u16 len, u8 type, u8 index (0..3), u32 period_ms (0 = stop),
 *                 u8 cqi (0 = the robust CQI 1), u8 subchannel, u16 reserved; payload (header 12 bytes)
 * read by the program
 *   RX      0x81  u16 len, u8 type, u8 frame flags, u32 src, u32 dst, u16 seq,
 *                 u8 cqi (the CQI this frame's SINR supports, what to report back), u8 cqi_sent,
 *                 i32 rssi_dbm_x10, i32 sinr_db_x10, u64 timestamp_us (end of the frame on the
 *                 airtime clock, propagation delay included), u64 slot (the slot it started in),
 *                 u8 subchannel, u8 cqi_report (the sender's report of how it hears us, 0 = none),
 *                 u16 delay_us (propagation delay); payload (header 44 bytes)
 *   STATUS  0x83  u16 len, u8 type, u8 result (0 sent, 1 acknowledged, 2 failed: no ACK after the
 *                 retransmissions, 3 rejected), u32 dst, u16 tag, u8 attempts, u8 cqi (last attempt),
 *                 u64 slot (last attempt), u32 airtime_us (24 bytes)
 * frame flags (also on the air): 0x01 HARQ (ACK requested), 0x02 ACK, 0x04 beacon, 0x08 retransmission
 * </pre>
 */
public final class FramedRecords {
    private FramedRecords() {}

    public static final int TX = 0x01, BEACON = 0x02, RX = 0x81, STATUS = 0x83;
    public static final int TX_HEADER = 20, BEACON_HEADER = 12, RX_HEADER = 44, STATUS_LEN = 24;
    public static final int FLAG_HARQ = 0x01, FLAG_ACK = 0x02, FLAG_BEACON = 0x04, FLAG_RETX = 0x08;
    public static final int RESULT_SENT = 0, RESULT_ACKED = 1, RESULT_FAILED = 2, RESULT_REJECTED = 3;
    /** Largest record a program reads (an RX record with a full payload). */
    public static final int MAX_RECORD = RX_HEADER + FramedRadio.MAX_PAYLOAD;

    /** A TX request parsed from the device file. */
    public record TxRequest(int flags, int dst, long slot, int cqi, int subchannel, int tag, byte[] payload) {}

    /** A BEACON request parsed from the device file. */
    public record BeaconRequest(int index, long periodMs, int cqi, int subchannel, byte[] payload) {}

    static ByteBuffer le(byte[] b) {
        return ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN);
    }

    public static byte[] rx(int flags, int src, int dst, int seq, int cqi, int cqiSent, double rssiDbm, double sinrDb,
                            long timestampUs, long slot, int subchannel, int cqiReport, long delayUs, byte[] payload) {
        byte[] out = new byte[RX_HEADER + payload.length];
        ByteBuffer b = le(out);
        b.putShort((short) out.length).put((byte) RX).put((byte) flags).putInt(src).putInt(dst).putShort((short) seq)
                .put((byte) cqi).put((byte) cqiSent)
                .putInt((int) Math.round(rssiDbm * 10)).putInt((int) Math.round(Math.max(-3000, Math.min(3000, sinrDb)) * 10))
                .putLong(timestampUs).putLong(slot).put((byte) subchannel).put((byte) cqiReport)
                .putShort((short) Math.min(65535, Math.max(0, delayUs)));
        b.put(payload);
        return out;
    }

    public static byte[] status(int result, int dst, int tag, int attempts, int cqi, long slot, long airtimeUs) {
        byte[] out = new byte[STATUS_LEN];
        le(out).putShort((short) STATUS_LEN).put((byte) STATUS).put((byte) result).putInt(dst).putShort((short) tag)
                .put((byte) attempts).put((byte) cqi).putLong(slot).putInt((int) Math.min(Integer.MAX_VALUE, airtimeUs));
        return out;
    }

    /** Encode a TX record (tests, tools). */
    public static byte[] tx(int flags, int dst, long slot, int cqi, int subchannel, int tag, byte[] payload) {
        byte[] out = new byte[TX_HEADER + payload.length];
        le(out).putShort((short) out.length).put((byte) TX).put((byte) flags).putInt(dst).putLong(slot)
                .put((byte) cqi).put((byte) subchannel).putShort((short) tag).put(payload);
        return out;
    }

    /** Encode a BEACON record (tests, tools). */
    public static byte[] beacon(int index, long periodMs, int cqi, int subchannel, byte[] payload) {
        byte[] out = new byte[BEACON_HEADER + payload.length];
        le(out).putShort((short) out.length).put((byte) BEACON).put((byte) index).putInt((int) periodMs)
                .put((byte) cqi).put((byte) subchannel).putShort((short) 0).put(payload);
        return out;
    }

    /** Length of the record at {@code off}, or -1 if fewer than 3 bytes remain. */
    public static int recordLength(byte[] data, int off, int end) {
        if (end - off < 3) return -1;
        return (data[off] & 0xff) | (data[off + 1] & 0xff) << 8;
    }

    public static TxRequest parseTx(byte[] d, int off, int len) {
        if (len < TX_HEADER) throw new IllegalArgumentException("TX record shorter than its 20-byte header");
        ByteBuffer b = ByteBuffer.wrap(d, off, len).order(ByteOrder.LITTLE_ENDIAN);
        b.position(off + 3);
        int flags = b.get() & 0xff;
        int dst = b.getInt();
        long slot = b.getLong();
        int cqi = b.get() & 0xff;
        int sub = b.get() & 0xff;
        int tag = b.getShort() & 0xffff;
        byte[] payload = new byte[len - TX_HEADER];
        b.get(payload);
        return new TxRequest(flags, dst, slot, cqi, sub, tag, payload);
    }

    public static BeaconRequest parseBeacon(byte[] d, int off, int len) {
        if (len < BEACON_HEADER) throw new IllegalArgumentException("BEACON record shorter than its 12-byte header");
        ByteBuffer b = ByteBuffer.wrap(d, off, len).order(ByteOrder.LITTLE_ENDIAN);
        b.position(off + 3);
        int index = b.get() & 0xff;
        long period = b.getInt() & 0xffffffffL;
        int cqi = b.get() & 0xff;
        int sub = b.get() & 0xff;
        b.getShort();
        byte[] payload = new byte[len - BEACON_HEADER];
        b.get(payload);
        return new BeaconRequest(index, period, cqi, sub, payload);
    }
}
