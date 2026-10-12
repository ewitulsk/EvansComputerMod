package com.example.evanscomputermod.radio.cellular;

import com.example.evanscomputermod.radio.framed.PhyProfile;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/**
 * The cellular air frames (a simplified LTE/NR control and user plane), byte
 * layout big-endian. Every frame starts with its type and the 32-bit cell key
 * of the tower it is from or for ({@link #cellKey(UUID)}).
 *
 * <pre>
 *   BEACON  (DL) type cell | tech channel txPower*10:i16 load | towerId:16 | carrierLen carrier
 *   ATTACH  (UL) type cell | imsi:8 mac:6 nonce:8 dlSinr*10:i16 flags | proof:8
 *   ACCEPT  (DL) type cell | imsi:8 rnti:2 slice | proof:8
 *   REJECT  (DL) type cell | imsi:8 cause
 *   DATA_UL (UL) type cell | rnti:2 dlSinr*10:i16 | Ethernet frame
 *   DATA_DL (DL) type cell | rnti:2 (0xFFFF = every device) | Ethernet frame
 *   REPORT  (UL) type cell | rnti:2 dlSinr*10:i16          (keep-alive + channel quality)
 *   DETACH  (UL) type cell | rnti:2
 *   RELEASE (DL) type cell | rnti:2 cause
 * </pre>
 * Pure.
 */
public final class CellFrame {
    private CellFrame() {}

    public static final byte BEACON = 1, ATTACH = 2, ACCEPT = 3, REJECT = 4, DATA_UL = 5, DATA_DL = 6, REPORT = 7, DETACH = 8,
            RELEASE = 9;
    public static final int BROADCAST_RNTI = 0xFFFF;
    public static final int FLAG_HANDOVER = 1;
    static final int HEADER = 5;

    /** Why a tower refused or released a device. */
    public enum Cause {
        NONE("ok"),
        UNKNOWN_SIM("this SIM isn't on the carrier's subscriber list"),
        BAD_KEY("SIM authentication failed"),
        WRONG_CARRIER("this tower belongs to another carrier"),
        QUOTA("data limit reached"),
        FULL("tower full"),
        REVOKED("SIM disabled by the carrier"),
        SHUTDOWN("tower switched off");

        public final String text;

        Cause(String text) {
            this.text = text;
        }

        static Cause of(int b) {
            Cause[] all = values();
            return b >= 0 && b < all.length ? all[b] : NONE;
        }
    }

    /** A decoded frame. */
    public sealed interface Frame permits Beacon, Attach, Accept, Reject, Data, Report, Detach, Release {
        int cellKey();
    }

    public record Beacon(int cellKey, PhyProfile tech, int channel, double txPowerDbm, int load, UUID towerId, String carrier) implements Frame {}

    public record Attach(int cellKey, long imsi, byte[] mac, long nonce, double dlSinrDb, int flags, byte[] proof) implements Frame {
        public boolean handover() {
            return (flags & FLAG_HANDOVER) != 0;
        }
    }

    public record Accept(int cellKey, long imsi, int rnti, int slice, byte[] proof) implements Frame {}

    public record Reject(int cellKey, long imsi, Cause cause) implements Frame {}

    /** User data: {@code uplink} frames carry the device's downlink SINR report. */
    public record Data(int cellKey, boolean uplink, int rnti, double dlSinrDb, byte[] ethernet) implements Frame {}

    public record Report(int cellKey, int rnti, double dlSinrDb) implements Frame {}

    public record Detach(int cellKey, int rnti) implements Frame {}

    public record Release(int cellKey, int rnti, Cause cause) implements Frame {}

    /** A tower's 32-bit cell key, from its id. */
    public static int cellKey(UUID towerId) {
        long h = towerId.getMostSignificantBits() ^ towerId.getLeastSignificantBits() * 0x9E3779B97F4A7C15L;
        h ^= h >>> 33;
        h *= 0xff51afd7ed558ccdL;
        h ^= h >>> 33;
        return (int) h;
    }

    // ------------------------------------------------------------ encode

    public static byte[] beacon(int cell, PhyProfile tech, int channel, double txPowerDbm, int load, UUID towerId, String carrier) {
        byte[] c = carrier.getBytes(StandardCharsets.UTF_8);
        ByteBuffer b = header(BEACON, cell, 1 + 1 + 2 + 1 + 16 + 1 + c.length);
        b.put((byte) tech.ordinal()).put((byte) channel).putShort(x10(txPowerDbm)).put((byte) Math.min(255, load))
                .putLong(towerId.getMostSignificantBits()).putLong(towerId.getLeastSignificantBits()).put((byte) c.length).put(c);
        return b.array();
    }

    public static byte[] attach(int cell, long imsi, byte[] mac, long nonce, double dlSinrDb, int flags, byte[] proof) {
        ByteBuffer b = header(ATTACH, cell, 8 + 6 + 8 + 2 + 1 + SimCard.PROOF_BYTES);
        b.putLong(imsi).put(mac, 0, 6).putLong(nonce).putShort(x10(dlSinrDb)).put((byte) flags).put(proof, 0, SimCard.PROOF_BYTES);
        return b.array();
    }

    public static byte[] accept(int cell, long imsi, int rnti, int slice, byte[] proof) {
        ByteBuffer b = header(ACCEPT, cell, 8 + 2 + 1 + SimCard.PROOF_BYTES);
        b.putLong(imsi).putShort((short) rnti).put((byte) slice).put(proof, 0, SimCard.PROOF_BYTES);
        return b.array();
    }

    public static byte[] reject(int cell, long imsi, Cause cause) {
        ByteBuffer b = header(REJECT, cell, 8 + 1);
        b.putLong(imsi).put((byte) cause.ordinal());
        return b.array();
    }

    public static byte[] dataUplink(int cell, int rnti, double dlSinrDb, byte[] eth) {
        ByteBuffer b = header(DATA_UL, cell, 2 + 2 + eth.length);
        b.putShort((short) rnti).putShort(x10(dlSinrDb)).put(eth);
        return b.array();
    }

    public static byte[] dataDownlink(int cell, int rnti, byte[] eth) {
        ByteBuffer b = header(DATA_DL, cell, 2 + eth.length);
        b.putShort((short) rnti).put(eth);
        return b.array();
    }

    public static byte[] report(int cell, int rnti, double dlSinrDb) {
        ByteBuffer b = header(REPORT, cell, 4);
        b.putShort((short) rnti).putShort(x10(dlSinrDb));
        return b.array();
    }

    public static byte[] detach(int cell, int rnti) {
        ByteBuffer b = header(DETACH, cell, 2);
        b.putShort((short) rnti);
        return b.array();
    }

    public static byte[] release(int cell, int rnti, Cause cause) {
        ByteBuffer b = header(RELEASE, cell, 3);
        b.putShort((short) rnti).put((byte) cause.ordinal());
        return b.array();
    }

    // ------------------------------------------------------------ decode

    /** Parse a frame; null when it is malformed or not a cellular frame. */
    public static Frame decode(byte[] f) {
        if (f == null || f.length < HEADER) return null;
        try {
            ByteBuffer b = ByteBuffer.wrap(f);
            byte type = b.get();
            int cell = b.getInt();
            switch (type) {
                case BEACON -> {
                    int techIdx = b.get() & 0xff;
                    PhyProfile[] techs = PhyProfile.values();
                    if (techIdx >= techs.length) return null;
                    int ch = b.get() & 0xff;
                    double p = b.getShort() / 10.0;
                    int load = b.get() & 0xff;
                    UUID id = new UUID(b.getLong(), b.getLong());
                    int n = b.get() & 0xff;
                    if (n > b.remaining()) return null;
                    byte[] c = new byte[n];
                    b.get(c);
                    return new Beacon(cell, techs[techIdx], ch, p, load, id, new String(c, StandardCharsets.UTF_8));
                }
                case ATTACH -> {
                    long imsi = b.getLong();
                    byte[] mac = new byte[6];
                    b.get(mac);
                    long nonce = b.getLong();
                    double s = sinr(b.getShort());
                    int flags = b.get() & 0xff;
                    byte[] proof = new byte[SimCard.PROOF_BYTES];
                    b.get(proof);
                    return new Attach(cell, imsi, mac, nonce, s, flags, proof);
                }
                case ACCEPT -> {
                    long imsi = b.getLong();
                    int rnti = b.getShort() & 0xffff;
                    int slice = b.get() & 0xff;
                    byte[] proof = new byte[SimCard.PROOF_BYTES];
                    b.get(proof);
                    return new Accept(cell, imsi, rnti, slice, proof);
                }
                case REJECT -> {
                    return new Reject(cell, b.getLong(), Cause.of(b.get() & 0xff));
                }
                case DATA_UL -> {
                    int rnti = b.getShort() & 0xffff;
                    double s = sinr(b.getShort());
                    byte[] eth = new byte[b.remaining()];
                    b.get(eth);
                    return new Data(cell, true, rnti, s, eth);
                }
                case DATA_DL -> {
                    int rnti = b.getShort() & 0xffff;
                    byte[] eth = new byte[b.remaining()];
                    b.get(eth);
                    return new Data(cell, false, rnti, Double.NaN, eth);
                }
                case REPORT -> {
                    int rnti = b.getShort() & 0xffff;
                    return new Report(cell, rnti, sinr(b.getShort()));
                }
                case DETACH -> {
                    return new Detach(cell, b.getShort() & 0xffff);
                }
                case RELEASE -> {
                    return new Release(cell, b.getShort() & 0xffff, Cause.of(b.get() & 0xff));
                }
                default -> {
                    return null;
                }
            }
        } catch (java.nio.BufferUnderflowException e) {
            return null;
        }
    }

    private static ByteBuffer header(byte type, int cell, int body) {
        ByteBuffer b = ByteBuffer.allocate(HEADER + body);
        b.put(type).putInt(cell);
        return b;
    }

    private static short x10(double v) {
        if (Double.isNaN(v)) return Short.MIN_VALUE;
        return (short) Math.max(Short.MIN_VALUE + 1, Math.min(Short.MAX_VALUE, Math.round(v * 10)));
    }

    /** A SINR report decoded from {@link #x10}: NaN for "unknown". */
    private static double sinr(short v) {
        return v == Short.MIN_VALUE ? Double.NaN : v / 10.0;
    }
}
