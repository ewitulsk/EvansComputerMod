package com.example.evanscomputermod.bluetooth;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Arrays;

/**
 * One Bluetooth frame on the radio medium (docs/BLUETOOTH.md "Frames"). Pure Java
 * so it is unit tested.
 *
 * <pre>
 *  0  2  magic 0xEC 0xB7
 *  2  1  version (1)
 *  3  1  type ({@link Type})
 *  4  6  sender address
 * 10  6  receiver address (all zero: anyone)
 * 16  .. body
 * </pre>
 *
 * Bodies:
 * <ul>
 *   <li>{@code ADV}: flags (bit 0 pairable, bit 1 connectable to its paired phone), device class,
 *       battery %, case battery % (255 = none), the paired phone's address (6 bytes, zero = none),
 *       name length + UTF-8 name.</li>
 *   <li>{@code PAIR_REQ}: the 16-byte link key, name length + the phone's name.
 *       {@code PAIR_RSP}: accepted (1/0).</li>
 *   <li>{@code CONNECT_REQ}: data channel (0-36), 8-byte proof ({@link #proof}).
 *       {@code CONNECT_RSP}: accepted (1/0).</li>
 *   <li>{@code AUDIO} (phone to headset) / {@code MIC} (headset to phone): sequence (u16), rate (u16),
 *       samples (u16), IMA ADPCM block.</li>
 *   <li>{@code CTRL}: microphone wanted (1/0), battery %, worn (1/0). Both ends send it as the link's
 *       keep-alive.</li>
 *   <li>{@code DISCONNECT}: nothing.</li>
 * </ul>
 */
public record BtFrame(Type type, long from, long to, byte[] body) {

    public enum Type { ADV, PAIR_REQ, PAIR_RSP, CONNECT_REQ, CONNECT_RSP, AUDIO, MIC, CTRL, DISCONNECT }

    public static final int HEADER = 16;
    public static final int FLAG_PAIRABLE = 1, FLAG_CONNECTABLE = 2;
    /** Device class of headphones. */
    public static final int CLASS_HEADSET = 1;
    private static final byte M0 = (byte) 0xEC, M1 = (byte) 0xB7, VERSION = 1;

    public byte[] encode() {
        ByteBuffer b = ByteBuffer.allocate(HEADER + body.length);
        b.put(M0).put(M1).put(VERSION).put((byte) type.ordinal());
        putAddr(b, from);
        putAddr(b, to);
        b.put(body);
        return b.array();
    }

    /** Decode a frame, or null if {@code data} isn't one. */
    public static BtFrame decode(byte[] data) {
        if (data == null || data.length < HEADER || data[0] != M0 || data[1] != M1 || data[2] != VERSION) return null;
        int t = data[3] & 255;
        if (t >= Type.values().length) return null;
        ByteBuffer b = ByteBuffer.wrap(data);
        b.position(4);
        long from = getAddr(b), to = getAddr(b);
        return new BtFrame(Type.values()[t], from, to, Arrays.copyOfRange(data, HEADER, data.length));
    }

    /** True if this frame is for {@code addr} (or for anyone). */
    public boolean isFor(long addr) {
        return to == 0 || to == addr;
    }

    // ------------------------------------------------------------------ bodies

    public record Adv(int flags, int deviceClass, int battery, int caseBattery, long pairedTo, String name) {
        public boolean pairable() { return (flags & FLAG_PAIRABLE) != 0; }
        public boolean connectable() { return (flags & FLAG_CONNECTABLE) != 0; }
    }

    public static BtFrame adv(long from, Adv a) {
        byte[] name = utf8(a.name(), 32);
        ByteBuffer b = ByteBuffer.allocate(11 + name.length);
        b.put((byte) a.flags()).put((byte) a.deviceClass()).put((byte) a.battery()).put((byte) a.caseBattery());
        putAddr(b, a.pairedTo());
        b.put((byte) name.length).put(name);
        return new BtFrame(Type.ADV, from, 0, b.array());
    }

    public Adv adv() {
        if (type != Type.ADV || body.length < 11) return null;
        ByteBuffer b = ByteBuffer.wrap(body);
        int flags = b.get() & 255, cls = b.get() & 255, bat = b.get() & 255, cb = b.get() & 255;
        long paired = getAddr(b);
        int n = b.get() & 255;
        if (b.remaining() < n) return null;
        byte[] name = new byte[n];
        b.get(name);
        return new Adv(flags, cls, bat, cb, paired, new String(name, StandardCharsets.UTF_8));
    }

    public static BtFrame pairRequest(long from, long to, byte[] key, String phoneName) {
        byte[] name = utf8(phoneName, 32);
        ByteBuffer b = ByteBuffer.allocate(17 + name.length);
        b.put(key, 0, 16).put((byte) name.length).put(name);
        return new BtFrame(Type.PAIR_REQ, from, to, b.array());
    }

    /** The link key of a PAIR_REQ, or null. */
    public byte[] pairKey() {
        return type == Type.PAIR_REQ && body.length >= 17 ? Arrays.copyOf(body, 16) : null;
    }

    /** The phone's name in a PAIR_REQ. */
    public String pairName() {
        if (type != Type.PAIR_REQ || body.length < 17) return "";
        int n = Math.min(body[16] & 255, body.length - 17);
        return new String(body, 17, n, StandardCharsets.UTF_8);
    }

    public static BtFrame answer(Type type, long from, long to, boolean ok) {
        return new BtFrame(type, from, to, new byte[] {(byte) (ok ? 1 : 0)});
    }

    public boolean ok() {
        return body.length > 0 && body[0] == 1;
    }

    public static BtFrame connectRequest(long from, long to, int channel, byte[] key) {
        ByteBuffer b = ByteBuffer.allocate(9);
        b.put((byte) channel).put(proof(key, from, to));
        return new BtFrame(Type.CONNECT_REQ, from, to, b.array());
    }

    public int connectChannel() {
        return body.length >= 1 ? body[0] & 255 : -1;
    }

    /** A CONNECT_REQ from the phone that holds {@code key}? */
    public boolean proves(byte[] key) {
        return type == Type.CONNECT_REQ && body.length >= 9 && key != null
                && MessageDigest.isEqual(Arrays.copyOfRange(body, 1, 9), proof(key, from, to));
    }

    /** The first 8 bytes of SHA-256(key, phone address, headset address): only the paired phone knows the key. */
    public static byte[] proof(byte[] key, long phone, long headset) {
        try {
            MessageDigest d = MessageDigest.getInstance("SHA-256");
            d.update(key);
            ByteBuffer b = ByteBuffer.allocate(12);
            putAddr(b, phone);
            putAddr(b, headset);
            d.update(b.array());
            return Arrays.copyOf(d.digest(), 8);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    public record Audio(int seq, int rate, int samples, byte[] adpcm) {}

    public static BtFrame audio(Type type, long from, long to, Audio a) {
        ByteBuffer b = ByteBuffer.allocate(6 + a.adpcm().length);
        b.putShort((short) a.seq()).putShort((short) a.rate()).putShort((short) a.samples()).put(a.adpcm());
        return new BtFrame(type, from, to, b.array());
    }

    public Audio audio() {
        if ((type != Type.AUDIO && type != Type.MIC) || body.length < 6) return null;
        ByteBuffer b = ByteBuffer.wrap(body);
        int seq = b.getShort() & 0xffff, rate = b.getShort() & 0xffff, samples = b.getShort() & 0xffff;
        return new Audio(seq, rate, samples, Arrays.copyOfRange(body, 6, body.length));
    }

    public record Ctrl(boolean micWanted, int battery, boolean worn) {}

    public static BtFrame ctrl(long from, long to, Ctrl c) {
        return new BtFrame(Type.CTRL, from, to, new byte[] {(byte) (c.micWanted() ? 1 : 0), (byte) c.battery(), (byte) (c.worn() ? 1 : 0)});
    }

    public Ctrl ctrl() {
        if (type != Type.CTRL || body.length < 3) return null;
        return new Ctrl(body[0] == 1, body[1] & 255, body[2] == 1);
    }

    // ------------------------------------------------------------------ addresses

    private static void putAddr(ByteBuffer b, long a) {
        for (int i = 5; i >= 0; i--) b.put((byte) (a >>> (8 * i)));
    }

    private static long getAddr(ByteBuffer b) {
        long a = 0;
        for (int i = 0; i < 6; i++) a = (a << 8) | (b.get() & 255);
        return a;
    }

    private static byte[] utf8(String s, int max) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        return b.length <= max ? b : Arrays.copyOf(b, max);
    }

    /** Airtime of a frame at the 2 Mb/s rate (preamble + access code + header + payload), µs. */
    public static long airtimeMicros(int bytes) {
        return 126 + bytes * 4L;
    }
}
