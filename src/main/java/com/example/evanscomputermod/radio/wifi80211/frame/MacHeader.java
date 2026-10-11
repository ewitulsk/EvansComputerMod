package com.example.evanscomputermod.radio.wifi80211.frame;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

/**
 * 802.11 MAC header (802.11-2016 9.2.3). Absent fields are {@code null} (addresses)
 * or {@code -1} (sequence control, QoS control, HT control).
 *
 * <ul>
 *   <li>Management: FC, Duration, A1, A2, A3, Sequence Control (24 bytes; +4 HT Control if Order).</li>
 *   <li>Data: as management, + A4 if ToDS and FromDS, + QoS Control for QoS subtypes,
 *       + HT Control for QoS frames with Order set.</li>
 *   <li>Control: ACK/CTS carry only A1 (10 bytes); RTS, PS-Poll, BlockAck(Req), CF-End carry A1 and A2 (16 bytes).</li>
 * </ul>
 */
public record MacHeader(int frameControl, int duration, MacAddress addr1, MacAddress addr2, MacAddress addr3,
                        int sequenceControl, MacAddress addr4, int qosControl, int htControl) {

    public static MacHeader mgmt(int subtype, MacAddress a1, MacAddress a2, MacAddress a3, int seq) {
        return new MacHeader(FrameControl.of(FrameControl.TYPE_MGMT, subtype), 0, a1, a2, a3, seqCtl(seq, 0), null, -1, -1);
    }

    public static MacHeader data(int flags, MacAddress a1, MacAddress a2, MacAddress a3, int seq) {
        return new MacHeader(FrameControl.of(FrameControl.TYPE_DATA, FrameControl.DATA, flags), 0, a1, a2, a3,
                seqCtl(seq, 0), null, -1, -1);
    }

    public static MacHeader qosData(int flags, MacAddress a1, MacAddress a2, MacAddress a3, int seq, int tid) {
        return new MacHeader(FrameControl.of(FrameControl.TYPE_DATA, FrameControl.QOS_DATA, flags), 0, a1, a2, a3,
                seqCtl(seq, 0), null, tid & 0x0F, -1);
    }

    public static int seqCtl(int seq, int frag) {
        return ((seq & 0xFFF) << 4) | (frag & 0xF);
    }

    public int type() { return FrameControl.type(frameControl); }
    public int subtype() { return FrameControl.subtype(frameControl); }
    public boolean toDs() { return FrameControl.toDs(frameControl); }
    public boolean fromDs() { return FrameControl.fromDs(frameControl); }
    public boolean isProtected() { return FrameControl.isProtected(frameControl); }
    public boolean isQos() { return qosControl >= 0; }
    public int sequenceNumber() { return sequenceControl < 0 ? -1 : (sequenceControl >>> 4) & 0xFFF; }
    public int tid() { return qosControl < 0 ? -1 : qosControl & 0x0F; }

    public MacHeader withFrameControl(int fc) {
        return new MacHeader(fc, duration, addr1, addr2, addr3, sequenceControl, addr4, qosControl, htControl);
    }

    /** Encoded length of this header. */
    public int length() {
        int t = type();
        if (t == FrameControl.TYPE_CTRL) {
            return addr2 == null ? 10 : 16;
        }
        int len = 24;
        if (addr4 != null) len += 6;
        if (qosControl >= 0) len += 2;
        if (htControl >= 0) len += 4;
        return len;
    }

    public byte[] encode() {
        byte[] b = new byte[length()];
        encodeInto(b, 0);
        return b;
    }

    public int encodeInto(byte[] b, int off) {
        int p = off;
        p = le16(b, p, frameControl);
        p = le16(b, p, duration);
        addr1.write(b, p);
        p += 6;
        if (type() == FrameControl.TYPE_CTRL) {
            if (addr2 != null) {
                addr2.write(b, p);
                p += 6;
            }
            return p - off;
        }
        addr2.write(b, p);
        p += 6;
        addr3.write(b, p);
        p += 6;
        p = le16(b, p, sequenceControl);
        if (addr4 != null) {
            addr4.write(b, p);
            p += 6;
        }
        if (qosControl >= 0) p = le16(b, p, qosControl);
        if (htControl >= 0) {
            for (int i = 0; i < 4; i++) b[p++] = (byte) (htControl >>> (8 * i));
        }
        return p - off;
    }

    /** Parses the header at the start of {@code b}; throws {@link IllegalArgumentException} if truncated or reserved. */
    public static MacHeader parse(byte[] b) {
        return parse(b, 0, b.length);
    }

    public static MacHeader parse(byte[] b, int off, int len) {
        if (len < 10) throw new IllegalArgumentException("frame shorter than a MAC header");
        int fc = u16(b, off);
        if (FrameControl.version(fc) != 0) throw new IllegalArgumentException("unsupported protocol version");
        int duration = u16(b, off + 2);
        MacAddress a1 = MacAddress.read(b, off + 4);
        int type = FrameControl.type(fc);
        if (type == 3) throw new IllegalArgumentException("reserved frame type");
        if (type == FrameControl.TYPE_CTRL) {
            int st = FrameControl.subtype(fc);
            if (st == FrameControl.ACK || st == FrameControl.CTS) {
                return new MacHeader(fc, duration, a1, null, null, -1, null, -1, -1);
            }
            if (len < 16) throw new IllegalArgumentException("truncated control frame");
            return new MacHeader(fc, duration, a1, MacAddress.read(b, off + 10), null, -1, null, -1, -1);
        }
        if (len < 24) throw new IllegalArgumentException("truncated MAC header");
        MacAddress a2 = MacAddress.read(b, off + 10);
        MacAddress a3 = MacAddress.read(b, off + 16);
        int sc = u16(b, off + 22);
        int p = 24;
        MacAddress a4 = null;
        int qos = -1;
        int ht = -1;
        boolean order = (fc & FrameControl.ORDER) != 0;
        if (type == FrameControl.TYPE_DATA) {
            if (FrameControl.toDs(fc) && FrameControl.fromDs(fc)) {
                if (len < p + 6) throw new IllegalArgumentException("truncated A4");
                a4 = MacAddress.read(b, off + p);
                p += 6;
            }
            if (FrameControl.isQosData(fc)) {
                if (len < p + 2) throw new IllegalArgumentException("truncated QoS control");
                qos = u16(b, off + p);
                p += 2;
                if (order) {
                    if (len < p + 4) throw new IllegalArgumentException("truncated HT control");
                    ht = (int) u32(b, off + p);
                    p += 4;
                }
            }
        } else if (order) {
            if (len < p + 4) throw new IllegalArgumentException("truncated HT control");
            ht = (int) u32(b, off + p);
        }
        return new MacHeader(fc, duration, a1, a2, a3, sc, a4, qos, ht);
    }

    static int u16(byte[] b, int off) {
        return (b[off] & 0xFF) | (b[off + 1] & 0xFF) << 8;
    }

    static long u32(byte[] b, int off) {
        return (b[off] & 0xFFL) | (b[off + 1] & 0xFFL) << 8 | (b[off + 2] & 0xFFL) << 16 | (b[off + 3] & 0xFFL) << 24;
    }

    static int le16(byte[] b, int off, int v) {
        b[off] = (byte) v;
        b[off + 1] = (byte) (v >>> 8);
        return off + 2;
    }
}
