package com.example.evanscomputermod.radio.wifi80211.frame;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

import java.util.Arrays;

/**
 * Ethernet II / 802.3 frame without FCS: destination, source, EtherType (or
 * 802.3 length when {@code <= 1500}), payload. VLAN tags stay in the payload.
 */
public record EthernetFrame(MacAddress dst, MacAddress src, int etherType, byte[] payload) {

    public static final int ETHERTYPE_IPV4 = 0x0800;
    public static final int ETHERTYPE_ARP = 0x0806;
    public static final int ETHERTYPE_IPV6 = 0x86DD;
    public static final int ETHERTYPE_EAPOL = 0x888E;

    public EthernetFrame {
        payload = payload.clone();
    }

    @Override
    public byte[] payload() {
        return payload.clone();
    }

    public byte[] encode() {
        byte[] b = new byte[14 + payload.length];
        dst.write(b, 0);
        src.write(b, 6);
        b[12] = (byte) (etherType >>> 8);
        b[13] = (byte) etherType;
        System.arraycopy(payload, 0, b, 14, payload.length);
        return b;
    }

    public static EthernetFrame parse(byte[] b) {
        if (b.length < 14) throw new IllegalArgumentException("Ethernet frame shorter than 14 bytes");
        int type = (b[12] & 0xFF) << 8 | (b[13] & 0xFF);
        int end = b.length;
        if (type <= 1500) end = Math.min(b.length, 14 + type); // 802.3 length: drop padding
        return new EthernetFrame(MacAddress.read(b, 0), MacAddress.read(b, 6), type, Arrays.copyOfRange(b, 14, end));
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof EthernetFrame e && e.dst.equals(dst) && e.src.equals(src) && e.etherType == etherType
                && Arrays.equals(e.payload, payload);
    }

    @Override
    public int hashCode() {
        return (dst.hashCode() * 31 + src.hashCode()) * 31 + Arrays.hashCode(payload);
    }
}
