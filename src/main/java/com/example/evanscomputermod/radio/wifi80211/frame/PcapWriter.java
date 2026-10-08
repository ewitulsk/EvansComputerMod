package com.example.evanscomputermod.radio.wifi80211.frame;

import java.io.ByteArrayOutputStream;

/**
 * Minimal libpcap writer (little-endian, microsecond timestamps). Use
 * {@link #LINKTYPE_IEEE802_11_RADIO} with {@link Radiotap#wrap} records so
 * Wireshark shows rate, channel and signal.
 */
public final class PcapWriter {

    public static final int LINKTYPE_ETHERNET = 1;
    public static final int LINKTYPE_IEEE802_11 = 105;
    public static final int LINKTYPE_IEEE802_11_RADIO = 127;

    private final ByteArrayOutputStream out = new ByteArrayOutputStream();

    public PcapWriter(int linkType) {
        int32(0xA1B2C3D4);
        int16(2);
        int16(4);
        int32(0); // thiszone
        int32(0); // sigfigs
        int32(65535); // snaplen
        int32(linkType);
    }

    public PcapWriter add(long timestampMicros, byte[] packet) {
        int32((int) (timestampMicros / 1_000_000));
        int32((int) (timestampMicros % 1_000_000));
        int32(packet.length);
        int32(packet.length);
        out.writeBytes(packet);
        return this;
    }

    public byte[] toByteArray() {
        return out.toByteArray();
    }

    private void int16(int v) {
        out.write(v);
        out.write(v >>> 8);
    }

    private void int32(int v) {
        int16(v);
        int16(v >>> 16);
    }
}
