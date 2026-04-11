package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;

import java.util.Arrays;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

/**
 * Server-wide ethernet hub that routes Layer 2 frames between computers.
 *
 * Each computer registers a NIC (identified by MAC address) with the hub.
 * Frames are routed based on destination MAC:
 * - Broadcast (ff:ff:ff:ff:ff:ff) → all NICs except sender
 * - Known unicast → matching NIC
 * - Unknown unicast → TAP bridge (if attached, for internet access)
 */
public class NetworkHub {
    private static NetworkHub INSTANCE;

    private final Map<MacAddress, NicMailbox> nics = new ConcurrentHashMap<>();

    // TAP bridge (optional, for real internet access)
    private TapBridge tapBridge;

    private static final int MAX_QUEUE_SIZE = 64;
    private static final int IRQ_NETWORK = 3;
    private static final byte[] BROADCAST_MAC = {(byte)0xff, (byte)0xff, (byte)0xff, (byte)0xff, (byte)0xff, (byte)0xff};

    /**
     * Per-NIC receive mailbox.
     */
    static class NicMailbox {
        final byte[] mac;
        final ConcurrentLinkedQueue<byte[]> rxQueue = new ConcurrentLinkedQueue<>();
        final LinkedBlockingQueue<byte[]> blockingQueue = new LinkedBlockingQueue<>(MAX_QUEUE_SIZE);
        volatile boolean promiscuous = false;
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

        void enqueue(byte[] frame) {
            if (rxQueue.size() >= MAX_QUEUE_SIZE) {
                rxQueue.poll(); // drop oldest
            }
            rxQueue.offer(frame.clone());
            blockingQueue.offer(frame.clone());
            // Mirror to pcap queue if capture is enabled
            if (pcapEnabled) {
                if (pcapQueue.size() >= MAX_QUEUE_SIZE) {
                    pcapQueue.poll(); // drop oldest
                }
                pcapQueue.offer(frame.clone());
            }
            // Fire IRQ_NETWORK
            interruptPusher.accept(IRQ_NETWORK, "{\"frame_len\":" + frame.length + "}");
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
     * Transmit a frame from a computer NIC.
     * Only delivers to NICs on the same cable network.
     * Only forwards to TAP if source has internet access via gateway.
     */
    public void transmit(byte[] srcMac, byte[] frame) {
        if (frame.length < 14) return;

        // Mirror outgoing frame to sender's pcap queue (tcpdump sees own TX)
        NicMailbox srcMailbox = nics.get(new MacAddress(srcMac));
        if (srcMailbox != null && srcMailbox.pcapEnabled) {
            if (srcMailbox.pcapQueue.size() >= MAX_QUEUE_SIZE) {
                srcMailbox.pcapQueue.poll();
            }
            srcMailbox.pcapQueue.offer(frame.clone());
        }

        byte[] dstMac = Arrays.copyOfRange(frame, 0, 6);
        boolean isBroadcast = Arrays.equals(dstMac, BROADCAST_MAC);
        CableNetworkManager cableMgr = CableNetworkManager.getInstance();

        if (isBroadcast) {
            for (var entry : nics.entrySet()) {
                if (!Arrays.equals(entry.getKey().bytes, srcMac)) {
                    // Only deliver if on the same cable network
                    if (cableMgr != null && !cableMgr.areOnSameNetwork(srcMac, entry.getKey().bytes)) {
                        continue;
                    }
                    entry.getValue().enqueue(frame);
                }
            }
            // Only forward to TAP if source has internet access
            if (tapBridge != null && (cableMgr == null || cableMgr.hasInternetAccess(srcMac))) {
                tapBridge.sendFrame(frame);
            }
        } else {
            MacAddress dstKey = new MacAddress(dstMac);
            NicMailbox target = nics.get(dstKey);
            boolean deliveredToNic = false;
            if (target != null) {
                // Only deliver if on the same cable network
                if (cableMgr == null || cableMgr.areOnSameNetwork(srcMac, dstMac)) {
                    target.enqueue(frame);
                    deliveredToNic = true;
                }
            }

            // Promiscuous NICs (only on same network)
            for (var entry : nics.entrySet()) {
                if (!Arrays.equals(entry.getKey().bytes, srcMac) &&
                    !Arrays.equals(entry.getKey().bytes, dstMac) &&
                    entry.getValue().promiscuous) {
                    if (cableMgr != null && !cableMgr.areOnSameNetwork(srcMac, entry.getKey().bytes)) {
                        continue;
                    }
                    entry.getValue().enqueue(frame);
                }
            }

            // Forward to TAP if no NIC matched and source has internet access
            if (!deliveredToNic && tapBridge != null && (cableMgr == null || cableMgr.hasInternetAccess(srcMac))) {
                tapBridge.sendFrame(frame);
            }
        }
    }

    /**
     * Inject a frame from the TAP device into the hub.
     * Only delivers to NICs that have internet access via the gateway.
     */
    public void injectFromTap(byte[] frame) {
        if (frame.length < 14) return;

        byte[] dstMac = Arrays.copyOfRange(frame, 0, 6);
        boolean isBroadcast = Arrays.equals(dstMac, BROADCAST_MAC);
        CableNetworkManager cableMgr = CableNetworkManager.getInstance();

        if (isBroadcast) {
            for (var entry : nics.entrySet()) {
                if (cableMgr != null && !cableMgr.hasInternetAccess(entry.getKey().bytes)) {
                    continue;
                }
                entry.getValue().enqueue(frame);
            }
        } else {
            MacAddress dstKey = new MacAddress(dstMac);
            NicMailbox target = nics.get(dstKey);
            if (target != null) {
                if (cableMgr == null || cableMgr.hasInternetAccess(dstMac)) {
                    target.enqueue(frame);
                }
            }
            // Promiscuous (only if they have internet access)
            for (var entry : nics.entrySet()) {
                if (!Arrays.equals(entry.getKey().bytes, dstMac) && entry.getValue().promiscuous) {
                    if (cableMgr != null && !cableMgr.hasInternetAccess(entry.getKey().bytes)) {
                        continue;
                    }
                    entry.getValue().enqueue(frame);
                }
            }
        }
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
     * Blocking receive with timeout for a NIC.
     */
    public byte[] receiveBlocking(byte[] mac, int timeoutMs) {
        NicMailbox mailbox = nics.get(new MacAddress(mac));
        if (mailbox == null) return null;

        // First check non-blocking queue
        byte[] frame = mailbox.rxQueue.poll();
        if (frame != null) return frame;

        // Block on the blocking queue
        try {
            return mailbox.blockingQueue.poll(timeoutMs, TimeUnit.MILLISECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return null;
        }
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
