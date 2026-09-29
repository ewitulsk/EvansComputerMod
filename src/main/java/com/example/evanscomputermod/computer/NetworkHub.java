package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;

import java.util.Arrays;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * Server-wide model of the Ethernet segments between computers.
 *
 * <p>Each connected cable mesh (see {@link CableNetworkManager}) is one
 * shared segment, like a hub: a frame transmitted by a NIC is offered to
 * every other NIC on the same segment, and each NIC's own filter decides
 * whether to accept it -- its own MAC, broadcast, any multicast, or
 * everything when promiscuous. A NIC whose link is administratively
 * disabled neither sends nor receives. Unicast frames for a MAC that is not
 * on the segment go to the TAP bridge if the segment reaches the Internet
 * Gateway.
 */
public class NetworkHub {
    private static NetworkHub INSTANCE;

    private final Map<MacAddress, NicMailbox> nics = new ConcurrentHashMap<>();

    // TAP bridge (optional, for real internet access)
    private TapBridge tapBridge;

    /** Per-NIC receive queue depth (drop-oldest). */
    private static final int MAX_QUEUE_SIZE = 256;
    private static final int IRQ_NETWORK = 3;
    private static final byte[] BROADCAST_MAC = {(byte)0xff, (byte)0xff, (byte)0xff, (byte)0xff, (byte)0xff, (byte)0xff};
    // Pre-built constant payload for IRQ_NETWORK — the kernel's IRQ handler
    // (rust/operating-system/rust/src/lib.rs:319 — `|_irq, _data| stack.poll_rx()`)
    // ignores the body, so building "{\"frame_len\":...}" per frame was pure
    // young-gen pressure. Keep it as "{}" in case any future handler expects
    // valid JSON.
    private static final String IRQ_PAYLOAD = "{}";

    /**
     * Per-NIC receive mailbox.
     */
    static class NicMailbox {
        final byte[] mac;
        final ConcurrentLinkedQueue<byte[]> rxQueue = new ConcurrentLinkedQueue<>();
        volatile boolean promiscuous = false;
        /** Administrative link state (ifconfig up/down). Disabled = no TX, no RX. */
        volatile boolean linkEnabled = true;
        // Packet capture (pcap) mirror queue — receives copies of all frames
        // without consuming from the main rxQueue.
        final ConcurrentLinkedQueue<byte[]> pcapQueue = new ConcurrentLinkedQueue<>();
        volatile boolean pcapEnabled = false;
        // Reference to computer's interrupt queue for IRQ delivery
        final java.util.function.BiConsumer<Integer, String> interruptPusher;

        NicMailbox(byte[] mac, java.util.function.BiConsumer<Integer, String> interruptPusher) {
            this.mac = mac.clone();
            this.interruptPusher = interruptPusher;
        }

        /** The NIC's receive filter. */
        boolean accepts(byte[] frame) {
            if (promiscuous) return true;
            if ((frame[0] & 0x01) != 0) return true; // broadcast or multicast
            for (int i = 0; i < 6; i++) {
                if (frame[i] != mac[i]) return false;
            }
            return true;
        }

        void enqueue(byte[] frame) {
            if (rxQueue.size() >= MAX_QUEUE_SIZE) {
                rxQueue.poll(); // drop oldest
            }
            rxQueue.offer(frame.clone());
            // Mirror to pcap queue if capture is enabled
            if (pcapEnabled) {
                if (pcapQueue.size() >= MAX_QUEUE_SIZE) {
                    pcapQueue.poll(); // drop oldest
                }
                pcapQueue.offer(frame.clone());
            }
            // Fire IRQ_NETWORK with a constant payload — the kernel handler
            // doesn't read it, and per-frame String concat was a hot
            // allocation source under network noise.
            interruptPusher.accept(IRQ_NETWORK, IRQ_PAYLOAD);
        }
    }

    /**
     * Wrapper for MAC address that can be used as a HashMap key.
     */
    static class MacAddress {
        final byte[] bytes;
        final int hash;

        MacAddress(byte[] mac) {
            this.bytes = mac.clone();
            this.hash = Arrays.hashCode(bytes);
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof MacAddress)) return false;
            return Arrays.equals(bytes, ((MacAddress) o).bytes);
        }

        @Override
        public int hashCode() {
            return hash;
        }
    }

    // ===== Lifecycle =====

    public static void init() {
        INSTANCE = new NetworkHub();
        EvansComputerMod.LOGGER.info("NetworkHub initialized");
    }

    public static void shutdown() {
        if (INSTANCE != null) {
            if (INSTANCE.tapBridge != null) {
                INSTANCE.tapBridge.close();
                INSTANCE.tapBridge = null;
            }
            INSTANCE.nics.clear();
            INSTANCE = null;
            EvansComputerMod.LOGGER.info("NetworkHub shut down");
        }
    }

    public static NetworkHub getInstance() {
        return INSTANCE;
    }

    // ===== NIC Management =====

    /**
     * Register a NIC. The interruptPusher is called with (irq, payload) when a frame arrives.
     */
    public void registerNic(byte[] mac, java.util.function.BiConsumer<Integer, String> interruptPusher) {
        nics.put(new MacAddress(mac), new NicMailbox(mac, interruptPusher));
        EvansComputerMod.LOGGER.debug("NetworkHub: registered NIC {}", formatMac(mac));
    }

    public void unregisterNic(byte[] mac) {
        nics.remove(new MacAddress(mac));
        EvansComputerMod.LOGGER.debug("NetworkHub: unregistered NIC {}", formatMac(mac));
    }

    // ===== Frame Routing =====

    /**
     * Transmit a frame from a computer NIC onto its segment.
     */
    public void transmit(byte[] srcMac, byte[] frame) {
        if (frame.length < 14) return;
        NicMailbox src = nics.get(new MacAddress(srcMac));
        if (src == null || !src.linkEnabled) return;

        // tcpdump on the sender sees its own TX.
        if (src.pcapEnabled) {
            if (src.pcapQueue.size() >= MAX_QUEUE_SIZE) {
                src.pcapQueue.poll();
            }
            src.pcapQueue.offer(frame.clone());
        }

        CableNetworkManager cableMgr = CableNetworkManager.getInstance();
        if (cableMgr == null) return;
        Integer segment = cableMgr.networkOf(srcMac);
        if (segment == null) return; // no cable on this face

        boolean groupAddr = (frame[0] & 0x01) != 0;
        boolean dstOnSegment = false;
        for (CableNetworkManager.MacAddress member : cableMgr.membersOf(segment)) {
            if (Arrays.equals(member.bytes, srcMac)) continue;
            if (!groupAddr && !dstOnSegment && matchesDst(member.bytes, frame)) {
                dstOnSegment = true;
            }
            NicMailbox nic = nics.get(new MacAddress(member.bytes));
            if (nic != null && nic.linkEnabled && nic.accepts(frame)) {
                nic.enqueue(frame);
            }
        }

        if (tapBridge != null && cableMgr.hasInternetAccess(srcMac) && (groupAddr || !dstOnSegment)) {
            tapBridge.sendFrame(frame);
        }
    }

    private static boolean matchesDst(byte[] mac, byte[] frame) {
        for (int i = 0; i < 6; i++) {
            if (mac[i] != frame[i]) return false;
        }
        return true;
    }

    /**
     * Inject a frame from the TAP device: it appears on every segment that
     * reaches the Internet Gateway.
     */
    public void injectFromTap(byte[] frame) {
        if (frame.length < 14) return;
        CableNetworkManager cableMgr = CableNetworkManager.getInstance();
        if (cableMgr == null) return;
        for (var entry : nics.entrySet()) {
            NicMailbox nic = entry.getValue();
            if (nic.linkEnabled && cableMgr.hasInternetAccess(entry.getKey().bytes) && nic.accepts(frame)) {
                nic.enqueue(frame);
            }
        }
    }

    /** Administrative link state (the kernel's ifconfig up/down). */
    public void setLinkEnabled(byte[] mac, boolean enabled) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        if (mailbox != null) {
            mailbox.linkEnabled = enabled;
            if (!enabled) mailbox.rxQueue.clear();
        }
    }

    /** Carrier: a cable connects this NIC to a segment and its link is enabled. */
    public boolean hasCarrier(byte[] mac) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        CableNetworkManager cableMgr = CableNetworkManager.getInstance();
        return mailbox != null && mailbox.linkEnabled && cableMgr != null && cableMgr.networkOf(mac) != null;
    }

    /**
     * Non-blocking receive for a NIC.
     */
    public byte[] receive(byte[] mac) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        if (mailbox == null) return null;
        return mailbox.rxQueue.poll();
    }

    /**
     * Set promiscuous mode for a NIC.
     */
    public void setPromiscuous(byte[] mac, boolean enabled) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        if (mailbox != null) {
            mailbox.promiscuous = enabled;
        }
    }

    /**
     * Enable or disable pcap (packet capture) on a NIC.
     * When enabled, copies of all received frames are placed in a
     * separate pcap queue that does NOT consume from the main rx queue.
     */
    public void setPcapEnabled(byte[] mac, boolean enabled) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        if (mailbox != null) {
            mailbox.pcapEnabled = enabled;
            if (!enabled) {
                mailbox.pcapQueue.clear();
            }
        }
    }

    /**
     * Non-blocking receive from the pcap mirror queue.
     * Returns a frame copy without affecting the main rx queue.
     */
    public byte[] pcapReceive(byte[] mac) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        if (mailbox == null) return null;
        return mailbox.pcapQueue.poll();
    }

    // ===== TAP Bridge =====

    public void setTapBridge(TapBridge bridge) {
        this.tapBridge = bridge;
    }

    public TapBridge getTapBridge() {
        return tapBridge;
    }

    // ===== Utilities =====

    /**
     * Derive a MAC address from a computer UUID.
     * Uses locally administered bit (bit 1 of first octet).
     */
    public static byte[] deriveMac(java.util.UUID computerId) {
        return deriveMac(computerId, 0);
    }

    public static byte[] deriveMac(java.util.UUID computerId, int ifaceIndex) {
        long msb = computerId.getMostSignificantBits();
        return new byte[] {
            0x02,
            (byte)(ifaceIndex),
            (byte)(msb >> 32),
            (byte)(msb >> 24),
            (byte)(msb >> 16),
            (byte)(msb >> 8)
        };
    }

    static String formatMac(byte[] mac) {
        return String.format("%02x:%02x:%02x:%02x:%02x:%02x",
            mac[0] & 0xff, mac[1] & 0xff, mac[2] & 0xff,
            mac[3] & 0xff, mac[4] & 0xff, mac[5] & 0xff);
    }
}
