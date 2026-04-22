package com.example.evanscomputermod.computer.netdev;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.NetworkHub;

import java.util.Arrays;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ThreadFactory;

/**
 * Abstract Java-side network device. Handles the Layer 2/3/4 plumbing
 * (Ethernet framing, ARP request/reply, ICMP echo, IPv4 + UDP parsing and
 * construction, checksums) so that each concrete device type only has to
 * implement {@link #onAppPacket}.
 *
 * <p>A device registers itself as a NIC on the server-wide {@link NetworkHub},
 * then runs purely in Java — no WASM runtime involved. Inbound frames are
 * drained on a shared worker thread so {@link NetworkHub#transmit} never
 * blocks waiting for a device to finish parsing.
 *
 * <p>The wire formats here must match the Rust stack in
 * {@code rust/crates/ecm-net/src/} exactly — any mismatched checksum
 * (IP header or UDP pseudo-header) will cause WASM computers to drop the
 * packet silently.
 */
public abstract class NetDevice {
    private static final int IRQ_NETWORK = 3;

    private static final int ETHERTYPE_IPV4 = 0x0800;
    private static final int ETHERTYPE_ARP  = 0x0806;

    private static final int ARP_HTYPE_ETHERNET = 1;
    private static final int ARP_PTYPE_IPV4     = 0x0800;
    private static final int ARP_REQUEST        = 1;
    private static final int ARP_REPLY          = 2;

    private static final int IPV4_PROTO_ICMP = 1;
    private static final int IPV4_PROTO_UDP  = 17;

    private static final int ICMP_ECHO_REQUEST = 8;
    private static final int ICMP_ECHO_REPLY   = 0;

    private static final int ETH_HEADER_LEN = 14;
    private static final int IPV4_HEADER_LEN = 20;
    private static final int UDP_HEADER_LEN = 8;
    private static final int ICMP_HEADER_LEN = 8;

    private static final long ARP_TTL_MS = 5 * 60 * 1000L;
    private static final byte[] BROADCAST_MAC = {
            (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff
    };

    /** Shared daemon executor for all devices. Keeps packet work off the sender's TX thread. */
    private static final ExecutorService WORKER = Executors.newSingleThreadExecutor(new ThreadFactory() {
        @Override
        public Thread newThread(Runnable r) {
            Thread t = new Thread(r, "ecm-netdev-worker");
            t.setDaemon(true);
            return t;
        }
    });

    private final byte[] mac;              // 6 bytes
    private volatile int ip;               // big-endian-as-int (e.g. 10.0.0.100 = 0x0A000064)
    private volatile int prefixLen;        // 0-32, 0 = unconfigured
    private volatile int appPort = EcmProto.DEFAULT_PORT;
    private volatile int ipId;

    /** IP -> (MAC, expiryMs). Synchronised on the map itself. */
    private final Map<Integer, ArpEntry> arpCache = new HashMap<>();

    private volatile boolean attached;
    private volatile int nextSeq = 1;

    private static final class ArpEntry {
        final byte[] mac;
        final long expiryMs;
        ArpEntry(byte[] mac, long expiryMs) { this.mac = mac; this.expiryMs = expiryMs; }
    }

    protected NetDevice(byte[] mac) {
        if (mac == null || mac.length != 6) {
            throw new IllegalArgumentException("MAC must be 6 bytes");
        }
        this.mac = mac.clone();
    }

    // ===== Configuration =====

    public final byte[] getMac() { return mac.clone(); }
    public final int getIp() { return ip; }
    public final int getPrefixLen() { return prefixLen; }
    public final int getAppPort() { return appPort; }
    public final boolean isConfigured() { return prefixLen > 0; }

    public final void setIp(int ip, int prefixLen) {
        this.ip = ip;
        this.prefixLen = (prefixLen < 0 || prefixLen > 32) ? 0 : prefixLen;
    }

    public final void setAppPort(int port) {
        if (port < 1 || port > 0xFFFF) throw new IllegalArgumentException("bad port " + port);
        this.appPort = port;
    }

    /** Device kind used in {@link EcmProto} DESCRIBE replies. */
    protected abstract int getDeviceKind();

    /** Called on the shared worker thread when a UDP packet to {@link #appPort} arrives. */
    protected abstract void onAppPacket(int srcIp, int srcPort, byte[] payload, int off, int len);

    /** Optional hook for subclasses that want to extend the DESCRIBE reply payload. */
    protected byte[] describePayload() { return new byte[0]; }

    /** Allocates the next protocol sequence number for outbound replies. */
    public final int nextSeq() {
        int s = nextSeq;
        nextSeq = (s + 1) & 0xffff;
        return s == 0 ? nextSeq() : s;
    }

    // ===== Lifecycle =====

    public final void attach() {
        if (attached) return;
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return;
        hub.registerNic(mac, (irq, payload) -> WORKER.submit(this::drainAndProcess));
        attached = true;
    }

    public final void detach() {
        if (!attached) return;
        NetworkHub hub = NetworkHub.getInstance();
        if (hub != null) hub.unregisterNic(mac);
        attached = false;
    }

    public final boolean isAttached() { return attached; }

    // ===== Inbound =====

    private void drainAndProcess() {
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return;
        byte[] frame;
        while ((frame = hub.receive(mac)) != null) {
            try {
                processFrame(frame);
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.debug("NetDevice {} frame error: {}",
                        NetworkHub.deriveMac(java.util.UUID.nameUUIDFromBytes(mac)), t.toString());
            }
        }
    }

    private void processFrame(byte[] frame) {
        if (frame.length < ETH_HEADER_LEN) return;

        // Destination MAC must match us or broadcast
        boolean forUs = matches(frame, 0, mac) || matches(frame, 0, BROADCAST_MAC);
        if (!forUs) return;

        int etype = u16(frame, 12);
        int payloadOff = ETH_HEADER_LEN;

        // 802.1Q handling: device does not participate in VLANs for now —
        // reject tagged frames to stay aligned with the Rust stack's
        // "None = untagged-only" behaviour.
        if (etype == 0x8100) {
            return;
        }

        byte[] srcMac = new byte[6];
        System.arraycopy(frame, 6, srcMac, 0, 6);

        switch (etype) {
            case ETHERTYPE_ARP -> handleArp(frame, payloadOff, frame.length - payloadOff, srcMac);
            case ETHERTYPE_IPV4 -> handleIpv4(frame, payloadOff, frame.length - payloadOff, srcMac);
            default -> { /* ignore */ }
        }
    }

    // --- ARP ---

    private void handleArp(byte[] buf, int off, int len, byte[] srcMac) {
        if (len < 28) return;
        int htype = u16(buf, off);
        int ptype = u16(buf, off + 2);
        int hlen = buf[off + 4] & 0xff;
        int plen = buf[off + 5] & 0xff;
        int oper = u16(buf, off + 6);
        if (htype != ARP_HTYPE_ETHERNET || ptype != ARP_PTYPE_IPV4 || hlen != 6 || plen != 4) return;

        byte[] senderMac = Arrays.copyOfRange(buf, off + 8, off + 14);
        int senderIp = u32(buf, off + 14);
        int targetIp = u32(buf, off + 24);

        learnArp(senderIp, senderMac);

        if (oper == ARP_REQUEST && isConfigured() && targetIp == ip) {
            byte[] reply = buildArpReply(senderMac, senderIp);
            NetworkHub hub = NetworkHub.getInstance();
            if (hub != null) hub.transmit(mac, reply);
        }
    }

    private byte[] buildArpReply(byte[] targetMac, int targetIp) {
        byte[] frame = new byte[ETH_HEADER_LEN + 28];
        // Ethernet header
        System.arraycopy(targetMac, 0, frame, 0, 6);
        System.arraycopy(mac, 0, frame, 6, 6);
        put16(frame, 12, ETHERTYPE_ARP);
        // ARP
        int off = ETH_HEADER_LEN;
        put16(frame, off, ARP_HTYPE_ETHERNET);
        put16(frame, off + 2, ARP_PTYPE_IPV4);
        frame[off + 4] = 6;
        frame[off + 5] = 4;
        put16(frame, off + 6, ARP_REPLY);
        System.arraycopy(mac, 0, frame, off + 8, 6);
        put32(frame, off + 14, ip);
        System.arraycopy(targetMac, 0, frame, off + 18, 6);
        put32(frame, off + 24, targetIp);
        return frame;
    }

    // --- IPv4 ---

    private void handleIpv4(byte[] buf, int off, int len, byte[] srcMac) {
        if (len < IPV4_HEADER_LEN) return;
        int versionIhl = buf[off] & 0xff;
        int version = versionIhl >> 4;
        int ihl = versionIhl & 0x0f;
        if (version != 4 || ihl < 5) return;
        int headerLen = ihl * 4;
        if (len < headerLen) return;

        int totalLen = u16(buf, off + 2);
        if (totalLen < headerLen || totalLen > len) return;

        // Verify header checksum
        int rxSum = u16(buf, off + 10);
        put16(buf, off + 10, 0);
        int computed = Checksum.internet(buf, off, headerLen);
        put16(buf, off + 10, rxSum);
        if (computed != rxSum) return;

        int proto = buf[off + 9] & 0xff;
        int srcIp = u32(buf, off + 12);
        int dstIp = u32(buf, off + 16);

        if (!isConfigured() || (dstIp != ip && dstIp != 0xFFFFFFFF)) return;

        // The sender's MAC is authoritative — cache it under its IP so we can reply directly.
        learnArp(srcIp, srcMac);

        int payloadOff = off + headerLen;
        int payloadLen = totalLen - headerLen;

        switch (proto) {
            case IPV4_PROTO_ICMP -> handleIcmp(buf, payloadOff, payloadLen, srcIp);
            case IPV4_PROTO_UDP -> handleUdp(buf, payloadOff, payloadLen, srcIp);
            default -> { /* ignore */ }
        }
    }

    // --- ICMP ---

    private void handleIcmp(byte[] buf, int off, int len, int srcIp) {
        if (len < ICMP_HEADER_LEN) return;
        int type = buf[off] & 0xff;
        int code = buf[off + 1] & 0xff;
        if (code != 0) return;

        int rxSum = u16(buf, off + 2);
        put16(buf, off + 2, 0);
        int computed = Checksum.internet(buf, off, len);
        put16(buf, off + 2, rxSum);
        if (computed != rxSum) return;

        if (type == ICMP_ECHO_REQUEST) {
            int id = u16(buf, off + 4);
            int seq = u16(buf, off + 6);
            byte[] icmpPayload = Arrays.copyOfRange(buf, off + ICMP_HEADER_LEN, off + len);
            byte[] replyIcmp = buildIcmpEchoReply(id, seq, icmpPayload);
            sendIpv4(srcIp, IPV4_PROTO_ICMP, replyIcmp, 0, replyIcmp.length);
        }
    }

    private byte[] buildIcmpEchoReply(int id, int seq, byte[] payload) {
        byte[] icmp = new byte[ICMP_HEADER_LEN + payload.length];
        icmp[0] = ICMP_ECHO_REPLY;
        icmp[1] = 0;
        // checksum placeholder
        put16(icmp, 4, id);
        put16(icmp, 6, seq);
        System.arraycopy(payload, 0, icmp, ICMP_HEADER_LEN, payload.length);
        int cksum = Checksum.internet(icmp, 0, icmp.length);
        put16(icmp, 2, cksum);
        return icmp;
    }

    // --- UDP ---

    private void handleUdp(byte[] buf, int off, int len, int srcIp) {
        if (len < UDP_HEADER_LEN) return;
        int srcPort = u16(buf, off);
        int dstPort = u16(buf, off + 2);
        int udpLen = u16(buf, off + 4);
        if (udpLen < UDP_HEADER_LEN || udpLen > len) return;

        // UDP checksum is optional in IPv4; 0 means "not computed". We accept
        // either 0 (skip) or a valid pseudo-header checksum.
        int rxSum = u16(buf, off + 6);
        if (rxSum != 0) {
            put16(buf, off + 6, 0);
            int computed = Checksum.pseudoHeader(srcIp, ip, IPV4_PROTO_UDP, buf, off, udpLen);
            put16(buf, off + 6, rxSum);
            // The Rust stack always produces a non-zero checksum, but 0 is a
            // legal "no checksum" value we permit too. A valid 0 computes to
            // 0xffff on the wire, so rxSum=0 is unambiguous.
            if (computed != rxSum) return;
        }

        if (dstPort != appPort) return;

        int payloadOff = off + UDP_HEADER_LEN;
        int payloadLen = udpLen - UDP_HEADER_LEN;
        onAppPacket(srcIp, srcPort, buf, payloadOff, payloadLen);
    }

    // ===== Outbound =====

    /**
     * Send a UDP datagram. Returns true on success. Requires the destination MAC
     * to already be in the ARP cache (populated automatically when the peer first
     * contacts us); returns false otherwise. Use {@link #sendUdpReply} when the
     * payload is a reply — that path always has the peer's MAC cached.
     */
    public final boolean sendUdp(int dstIp, int dstPort, byte[] payload, int off, int len) {
        if (!isConfigured()) return false;
        int udpLen = UDP_HEADER_LEN + len;
        byte[] udp = new byte[udpLen];
        put16(udp, 0, appPort);
        put16(udp, 2, dstPort);
        put16(udp, 4, udpLen);
        // checksum zero, fill after
        System.arraycopy(payload, off, udp, UDP_HEADER_LEN, len);
        int cksum = Checksum.pseudoHeader(ip, dstIp, IPV4_PROTO_UDP, udp, 0, udpLen);
        // RFC 768: a computed checksum of 0 is transmitted as 0xffff.
        if (cksum == 0) cksum = 0xffff;
        put16(udp, 6, cksum);

        return sendIpv4(dstIp, IPV4_PROTO_UDP, udp, 0, udpLen);
    }

    /** Convenience: reply to the peer a message just came from. */
    public final boolean sendUdpReply(int dstIp, int dstPort, byte[] payload) {
        return sendUdp(dstIp, dstPort, payload, 0, payload.length);
    }

    /** Low-level IPv4 send; returns false if we don't know the destination's MAC. */
    public final boolean sendIpv4(int dstIp, int protocol, byte[] payload, int off, int len) {
        if (!isConfigured()) return false;

        byte[] dstMac = resolveNextHopMac(dstIp);
        if (dstMac == null) return false;

        int frameLen = ETH_HEADER_LEN + IPV4_HEADER_LEN + len;
        byte[] frame = new byte[frameLen];
        // Ethernet
        System.arraycopy(dstMac, 0, frame, 0, 6);
        System.arraycopy(mac, 0, frame, 6, 6);
        put16(frame, 12, ETHERTYPE_IPV4);

        // IPv4
        int ipOff = ETH_HEADER_LEN;
        frame[ipOff] = (byte) 0x45;     // v4, IHL=5
        frame[ipOff + 1] = 0;           // DSCP/ECN
        put16(frame, ipOff + 2, IPV4_HEADER_LEN + len);
        put16(frame, ipOff + 4, nextIpId());
        put16(frame, ipOff + 6, 0);     // flags/frag
        frame[ipOff + 8] = 64;          // TTL
        frame[ipOff + 9] = (byte) protocol;
        put16(frame, ipOff + 10, 0);    // checksum placeholder
        put32(frame, ipOff + 12, ip);
        put32(frame, ipOff + 16, dstIp);
        int ipSum = Checksum.internet(frame, ipOff, IPV4_HEADER_LEN);
        put16(frame, ipOff + 10, ipSum);

        // Payload
        System.arraycopy(payload, off, frame, ipOff + IPV4_HEADER_LEN, len);

        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return false;
        hub.transmit(mac, frame);
        return true;
    }

    private synchronized int nextIpId() {
        ipId = (ipId + 1) & 0xffff;
        return ipId == 0 ? nextIpId() : ipId;
    }

    // --- ARP cache ---

    private byte[] resolveNextHopMac(int dstIp) {
        if (dstIp == 0xFFFFFFFF) return BROADCAST_MAC.clone();
        // On-link check: only the subnet we were configured for is reachable
        // directly; off-subnet would need a gateway, which this MVP doesn't
        // model. For redstone use, same-subnet is enough.
        if (prefixLen == 0) return null;
        int mask = prefixLen == 0 ? 0 : (prefixLen == 32 ? -1 : ~((1 << (32 - prefixLen)) - 1));
        if ((dstIp & mask) != (ip & mask)) return null;

        long now = System.currentTimeMillis();
        synchronized (arpCache) {
            ArpEntry e = arpCache.get(dstIp);
            if (e != null && e.expiryMs > now) return e.mac.clone();
        }
        return null;
    }

    private void learnArp(int ipAddr, byte[] mac) {
        if (mac == null || mac.length != 6) return;
        long expiry = System.currentTimeMillis() + ARP_TTL_MS;
        synchronized (arpCache) {
            arpCache.put(ipAddr, new ArpEntry(mac.clone(), expiry));
        }
    }

    // ===== Byte utilities =====

    private static boolean matches(byte[] buf, int off, byte[] target) {
        for (int i = 0; i < target.length; i++) {
            if (buf[off + i] != target[i]) return false;
        }
        return true;
    }

    private static int u16(byte[] buf, int off) {
        return ((buf[off] & 0xff) << 8) | (buf[off + 1] & 0xff);
    }

    private static int u32(byte[] buf, int off) {
        return ((buf[off] & 0xff) << 24) | ((buf[off + 1] & 0xff) << 16)
                | ((buf[off + 2] & 0xff) << 8) | (buf[off + 3] & 0xff);
    }

    private static void put16(byte[] buf, int off, int v) {
        buf[off] = (byte) ((v >> 8) & 0xff);
        buf[off + 1] = (byte) (v & 0xff);
    }

    private static void put32(byte[] buf, int off, int v) {
        buf[off] = (byte) ((v >> 24) & 0xff);
        buf[off + 1] = (byte) ((v >> 16) & 0xff);
        buf[off + 2] = (byte) ((v >> 8) & 0xff);
        buf[off + 3] = (byte) (v & 0xff);
    }

    // ===== Utilities =====

    /**
     * Parse a CIDR string like "10.0.0.5/24" into {ip, prefixLen}. Throws
     * IllegalArgumentException on any parse failure.
     */
    public static int[] parseCidr(String cidr) {
        int slash = cidr.indexOf('/');
        if (slash <= 0) throw new IllegalArgumentException("expected A.B.C.D/N");
        String ipStr = cidr.substring(0, slash);
        int prefix;
        try {
            prefix = Integer.parseInt(cidr.substring(slash + 1));
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("bad prefix: " + cidr);
        }
        if (prefix < 0 || prefix > 32) throw new IllegalArgumentException("prefix out of range");
        return new int[] { parseIp(ipStr), prefix };
    }

    public static int parseIp(String ipStr) {
        String[] parts = ipStr.split("\\.");
        if (parts.length != 4) throw new IllegalArgumentException("bad ip: " + ipStr);
        int out = 0;
        for (String p : parts) {
            int b;
            try { b = Integer.parseInt(p); } catch (NumberFormatException e) {
                throw new IllegalArgumentException("bad ip: " + ipStr);
            }
            if (b < 0 || b > 255) throw new IllegalArgumentException("bad ip: " + ipStr);
            out = (out << 8) | b;
        }
        return out;
    }

    public static String formatIp(int ip) {
        return ((ip >>> 24) & 0xff) + "." + ((ip >>> 16) & 0xff) + "."
                + ((ip >>> 8) & 0xff) + "." + (ip & 0xff);
    }
}
