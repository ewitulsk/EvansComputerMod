package com.example.evanscomputermod.radio.wifi80211.frame;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;

import java.util.List;

/**
 * Management frame bodies (802.11-2016 9.3.3), status codes (9.4.1.9) and
 * reason codes (9.4.1.7). All multi-octet fixed fields are little-endian.
 */
public final class Mgmt {

    private Mgmt() {}

    // Capability Information bits
    public static final int CAP_ESS = 0x0001;
    public static final int CAP_IBSS = 0x0002;
    public static final int CAP_PRIVACY = 0x0010;
    public static final int CAP_SHORT_PREAMBLE = 0x0020;
    public static final int CAP_SHORT_SLOT = 0x0400;

    // Authentication algorithms
    public static final int AUTH_OPEN = 0;
    public static final int AUTH_SHARED_KEY = 1;

    // Status codes
    public static final int STATUS_SUCCESS = 0;
    public static final int STATUS_UNSPECIFIED = 1;
    public static final int STATUS_UNSUPPORTED_AUTH_ALG = 13;
    public static final int STATUS_AUTH_SEQ_UNEXPECTED = 14;
    public static final int STATUS_AP_FULL = 17;
    public static final int STATUS_INVALID_IE = 40;
    public static final int STATUS_INVALID_GROUP_CIPHER = 41;
    public static final int STATUS_INVALID_PAIRWISE_CIPHER = 42;
    public static final int STATUS_INVALID_AKMP = 43;
    public static final int STATUS_UNSUPPORTED_RSN_VERSION = 44;

    // Reason codes
    public static final int REASON_UNSPECIFIED = 1;
    public static final int REASON_PREV_AUTH_NOT_VALID = 2;
    public static final int REASON_DEAUTH_LEAVING = 3;
    public static final int REASON_INACTIVITY = 4;
    public static final int REASON_AP_BUSY = 5;
    public static final int REASON_CLASS2_FROM_NONAUTH = 6;
    public static final int REASON_CLASS3_FROM_NONASSOC = 7;
    public static final int REASON_DISASSOC_LEAVING = 8;
    public static final int REASON_NOT_AUTHENTICATED = 9;
    public static final int REASON_INVALID_IE = 13;
    public static final int REASON_MIC_FAILURE = 14;
    public static final int REASON_4WAY_TIMEOUT = 15;
    public static final int REASON_GROUP_KEY_TIMEOUT = 16;
    public static final int REASON_IE_DIFFERENT = 17;
    public static final int REASON_INVALID_GROUP_CIPHER = 18;
    public static final int REASON_INVALID_PAIRWISE_CIPHER = 19;
    public static final int REASON_INVALID_AKMP = 20;

    /** Beacon and Probe Response body: Timestamp, Beacon Interval, Capability, elements. */
    public record Beacon(long timestamp, int beaconInterval, int capability, List<Ie> ies) {
        public Beacon {
            ies = List.copyOf(ies);
        }

        public byte[] encode() {
            byte[] e = Ie.encodeAll(ies);
            byte[] b = new byte[12 + e.length];
            for (int i = 0; i < 8; i++) b[i] = (byte) (timestamp >>> (8 * i));
            MacHeader.le16(b, 8, beaconInterval);
            MacHeader.le16(b, 10, capability);
            System.arraycopy(e, 0, b, 12, e.length);
            return b;
        }

        public static Beacon parse(byte[] b) {
            if (b.length < 12) throw new IllegalArgumentException("beacon body too short");
            long ts = 0;
            for (int i = 7; i >= 0; i--) ts = (ts << 8) | (b[i] & 0xFF);
            return new Beacon(ts, MacHeader.u16(b, 8), MacHeader.u16(b, 10), Ie.parseAll(b, 12, b.length - 12));
        }

        public byte[] ssid() {
            Ie ie = Ie.find(ies, Ie.SSID);
            return ie == null ? null : ie.data();
        }
    }

    /** Authentication body: algorithm, transaction sequence, status, optional elements. */
    public record Auth(int algorithm, int sequence, int status, List<Ie> ies) {
        public Auth {
            ies = List.copyOf(ies);
        }

        public byte[] encode() {
            byte[] e = Ie.encodeAll(ies);
            byte[] b = new byte[6 + e.length];
            MacHeader.le16(b, 0, algorithm);
            MacHeader.le16(b, 2, sequence);
            MacHeader.le16(b, 4, status);
            System.arraycopy(e, 0, b, 6, e.length);
            return b;
        }

        public static Auth parse(byte[] b) {
            if (b.length < 6) throw new IllegalArgumentException("auth body too short");
            return new Auth(MacHeader.u16(b, 0), MacHeader.u16(b, 2), MacHeader.u16(b, 4), Ie.parseAll(b, 6, b.length - 6));
        }
    }

    /**
     * (Re)Association Request body: Capability, Listen Interval, Current AP address
     * (reassociation only; {@code null} otherwise), elements.
     */
    public record AssocRequest(int capability, int listenInterval, MacAddress currentAp, List<Ie> ies) {
        public AssocRequest {
            ies = List.copyOf(ies);
        }

        public byte[] encode() {
            byte[] e = Ie.encodeAll(ies);
            int fixed = currentAp == null ? 4 : 10;
            byte[] b = new byte[fixed + e.length];
            MacHeader.le16(b, 0, capability);
            MacHeader.le16(b, 2, listenInterval);
            if (currentAp != null) currentAp.write(b, 4);
            System.arraycopy(e, 0, b, fixed, e.length);
            return b;
        }

        public static AssocRequest parse(byte[] b, boolean reassoc) {
            int fixed = reassoc ? 10 : 4;
            if (b.length < fixed) throw new IllegalArgumentException("assoc request too short");
            MacAddress cur = reassoc ? MacAddress.read(b, 4) : null;
            return new AssocRequest(MacHeader.u16(b, 0), MacHeader.u16(b, 2), cur, Ie.parseAll(b, fixed, b.length - fixed));
        }
    }

    /** (Re)Association Response body: Capability, Status, AID (sent with bits 14-15 set), elements. */
    public record AssocResponse(int capability, int status, int aid, List<Ie> ies) {
        public AssocResponse {
            ies = List.copyOf(ies);
        }

        public byte[] encode() {
            byte[] e = Ie.encodeAll(ies);
            byte[] b = new byte[6 + e.length];
            MacHeader.le16(b, 0, capability);
            MacHeader.le16(b, 2, status);
            MacHeader.le16(b, 4, aid == 0 ? 0 : (aid | 0xC000));
            System.arraycopy(e, 0, b, 6, e.length);
            return b;
        }

        public static AssocResponse parse(byte[] b) {
            if (b.length < 6) throw new IllegalArgumentException("assoc response too short");
            return new AssocResponse(MacHeader.u16(b, 0), MacHeader.u16(b, 2), MacHeader.u16(b, 4) & 0x3FFF,
                    Ie.parseAll(b, 6, b.length - 6));
        }
    }

    /** Deauthentication / Disassociation body: the 2-byte reason code. */
    public static byte[] reason(int code) {
        byte[] b = new byte[2];
        MacHeader.le16(b, 0, code);
        return b;
    }

    public static int parseReason(byte[] body) {
        if (body.length < 2) throw new IllegalArgumentException("reason body too short");
        return MacHeader.u16(body, 0);
    }
}
