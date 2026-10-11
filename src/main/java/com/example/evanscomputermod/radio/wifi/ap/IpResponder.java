package com.example.evanscomputermod.radio.wifi.ap;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

/**
 * The smallest IPv4 host that can be pinged: answers ARP requests for its
 * address and ICMP echo requests sent to it. Used by the virtual Wi-Fi
 * station so a wired computer can {@code ping} it through an Access Point.
 * Pure; frames are Ethernet II without FCS.
 */
public final class IpResponder {

    private static final int ETH_ARP = 0x0806, ETH_IPV4 = 0x0800;

    private IpResponder() {}

    /** Packs a dotted quad into an int. */
    public static int ip(String dotted) {
        String[] p = dotted.split("\\.");
        if (p.length != 4) throw new IllegalArgumentException("bad IPv4 address " + dotted);
        int v = 0;
        for (String s : p) {
            int o = Integer.parseInt(s);
            if (o < 0 || o > 255) throw new IllegalArgumentException("bad IPv4 address " + dotted);
            v = (v << 8) | o;
        }
        return v;
    }

    /** The reply to {@code eth} if it is an ARP request or echo request for {@code ip}, else null. */
    public static byte[] respond(byte[] eth, MacAddress mac, int ip) {
        if (eth == null || eth.length < 14) return null;
        int type = u16(eth, 12);
        if (type == ETH_ARP) return arp(eth, mac, ip);
        if (type == ETH_IPV4) return icmp(eth, mac, ip);
        return null;
    }

    private static byte[] arp(byte[] eth, MacAddress mac, int ip) {
        if (eth.length < 14 + 28) return null;
        int o = 14;
        if (u16(eth, o) != 1 || u16(eth, o + 2) != ETH_IPV4 || eth[o + 4] != 6 || eth[o + 5] != 4) return null;
        if (u16(eth, o + 6) != 1) return null;               // request
        if (i32(eth, o + 24) != ip) return null;             // target protocol address
        byte[] r = new byte[14 + 28];
        System.arraycopy(eth, o + 8, r, 0, 6);               // to the asker's hardware address
        mac.write(r, 6);
        put16(r, 12, ETH_ARP);
        put16(r, o, 1);
        put16(r, o + 2, ETH_IPV4);
        r[o + 4] = 6;
        r[o + 5] = 4;
        put16(r, o + 6, 2);                                  // reply
        mac.write(r, o + 8);
        put32(r, o + 14, ip);
        System.arraycopy(eth, o + 8, r, o + 18, 10);         // target = asker's MAC + IP
        return r;
    }

    private static byte[] icmp(byte[] eth, MacAddress mac, int ip) {
        int o = 14;
        if (eth.length < o + 20) return null;
        int vihl = eth[o] & 0xFF;
        if (vihl >> 4 != 4) return null;
        int ihl = (vihl & 0x0F) * 4;
        int total = u16(eth, o + 2);
        if (ihl < 20 || total < ihl + 8 || eth.length < o + total) return null;
        if ((eth[o + 9] & 0xFF) != 1 || i32(eth, o + 16) != ip) return null;
        if ((u16(eth, o + 6) & 0x3FFF) != 0) return null;    // fragments: not handled
        int ic = o + ihl;
        if ((eth[ic] & 0xFF) != 8) return null;              // echo request
        byte[] r = new byte[o + total];
        System.arraycopy(eth, 6, r, 0, 6);
        mac.write(r, 6);
        put16(r, 12, ETH_IPV4);
        System.arraycopy(eth, o, r, o, total);
        System.arraycopy(eth, o + 16, r, o + 12, 4);         // src <- old dst
        System.arraycopy(eth, o + 12, r, o + 16, 4);         // dst <- old src
        r[o + 8] = 64;                                       // TTL
        put16(r, o + 10, 0);
        put16(r, o + 10, checksum(r, o, ihl));
        r[ic] = 0;                                           // echo reply
        put16(r, ic + 2, 0);
        put16(r, ic + 2, checksum(r, ic, total - ihl));
        return r;
    }

    /** RFC 1071 Internet checksum. */
    public static int checksum(byte[] b, int off, int len) {
        long sum = 0;
        for (int i = 0; i + 1 < len; i += 2) sum += u16(b, off + i);
        if ((len & 1) != 0) sum += (b[off + len - 1] & 0xFF) << 8;
        while ((sum >> 16) != 0) sum = (sum & 0xFFFF) + (sum >> 16);
        return (int) (~sum & 0xFFFF);
    }

    private static int u16(byte[] b, int o) {
        return ((b[o] & 0xFF) << 8) | (b[o + 1] & 0xFF);
    }

    private static int i32(byte[] b, int o) {
        return (u16(b, o) << 16) | u16(b, o + 2);
    }

    private static void put16(byte[] b, int o, int v) {
        b[o] = (byte) (v >>> 8);
        b[o + 1] = (byte) v;
    }

    private static void put32(byte[] b, int o, int v) {
        put16(b, o, v >>> 16);
        put16(b, o + 2, v);
    }
}
