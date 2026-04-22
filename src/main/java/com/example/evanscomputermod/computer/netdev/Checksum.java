package com.example.evanscomputermod.computer.netdev;

/**
 * RFC 1071 internet checksum helpers — matches the Rust stack in
 * {@code rust/crates/ecm-net/src/checksum.rs}. Any mismatch against the
 * pseudo-header rules below will cause the OS to silently drop packets.
 */
public final class Checksum {
    private Checksum() {}

    /** One's-complement 16-bit sum over a byte range. */
    public static int internet(byte[] data, int off, int len) {
        int sum = 0;
        int i = off;
        int end = off + len;
        while (i + 1 < end) {
            sum += ((data[i] & 0xff) << 8) | (data[i + 1] & 0xff);
            i += 2;
        }
        if (i < end) {
            sum += (data[i] & 0xff) << 8;
        }
        while ((sum >>> 16) != 0) {
            sum = (sum & 0xffff) + (sum >>> 16);
        }
        return (~sum) & 0xffff;
    }

    /**
     * Computes the TCP/UDP pseudo-header checksum. {@code segment} must be the full
     * L4 header+payload with its own checksum field pre-zeroed.
     */
    public static int pseudoHeader(int srcIp, int dstIp, int protocol,
                                   byte[] segment, int segOff, int segLen) {
        int sum = 0;
        sum += (srcIp >>> 16) & 0xffff;
        sum += srcIp & 0xffff;
        sum += (dstIp >>> 16) & 0xffff;
        sum += dstIp & 0xffff;
        sum += protocol & 0xff;
        sum += segLen & 0xffff;

        int i = segOff;
        int end = segOff + segLen;
        while (i + 1 < end) {
            sum += ((segment[i] & 0xff) << 8) | (segment[i + 1] & 0xff);
            i += 2;
        }
        if (i < end) {
            sum += (segment[i] & 0xff) << 8;
        }
        while ((sum >>> 16) != 0) {
            sum = (sum & 0xffff) + (sum >>> 16);
        }
        return (~sum) & 0xffff;
    }
}
