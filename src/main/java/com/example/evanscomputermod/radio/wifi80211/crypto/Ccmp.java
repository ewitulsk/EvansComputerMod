package com.example.evanscomputermod.radio.wifi80211.crypto;

import com.example.evanscomputermod.radio.wifi80211.frame.FrameControl;
import com.example.evanscomputermod.radio.wifi80211.frame.MacHeader;

import java.io.ByteArrayOutputStream;
import java.util.Arrays;

/**
 * CCMP-128 (802.11-2016 12.5.3): AES-CCM with M = 8, L = 2 over an MPDU.
 *
 * <ul>
 *   <li>CCMP header (8 bytes): PN0 PN1 rsvd (KeyID&lt;&lt;6 | ExtIV) PN2 PN3 PN4 PN5.</li>
 *   <li>Nonce (13 bytes): flags (priority = TID, bit 4 = management) || A2 || PN5..PN0.</li>
 *   <li>AAD: FC with subtype bits 4-6 (data), Retry, PwrMgt, MoreData masked and Protected set
 *       (Order masked for QoS data); A1 A2 A3; SC with the sequence number masked; A4 if
 *       present; QoS Control masked to the TID if present.</li>
 * </ul>
 * MPDUs here never include the FCS.
 */
public final class Ccmp {

    public static final int HEADER_LEN = 8;
    public static final int MIC_LEN = 8;
    public static final long PN_MAX = 0xFFFF_FFFF_FFFFL;
    private static final int EXT_IV = 0x20;

    private Ccmp() {}

    /** A decrypted MPDU (Protected bit cleared, CCMP header and MIC removed) with its PN and key ID. */
    public record Decrypted(byte[] mpdu, long pn, int keyId) {}

    /** Encrypts a plaintext MPDU (header + body) and returns the protected MPDU. */
    public static byte[] encrypt(byte[] tk, byte[] mpdu, long pn, int keyId) {
        if (pn < 0 || pn > PN_MAX) throw new IllegalArgumentException("PN out of range");
        MacHeader h = MacHeader.parse(mpdu);
        int hl = h.length();
        MacHeader ph = h.withFrameControl(h.frameControl() | FrameControl.PROTECTED);
        byte[] plain = Arrays.copyOfRange(mpdu, hl, mpdu.length);
        byte[] sealed = AesCcm.encrypt(tk, nonce(ph, pn), aad(ph), plain, MIC_LEN);
        byte[] out = new byte[hl + HEADER_LEN + sealed.length];
        ph.encodeInto(out, 0);
        writeHeader(out, hl, pn, keyId);
        System.arraycopy(sealed, 0, out, hl + HEADER_LEN, sealed.length);
        return out;
    }

    /** Decrypts a protected MPDU; returns {@code null} if malformed or the MIC fails. */
    public static Decrypted decrypt(byte[] tk, byte[] mpdu) {
        MacHeader h;
        try {
            h = MacHeader.parse(mpdu);
        } catch (IllegalArgumentException e) {
            return null;
        }
        int hl = h.length();
        if (!h.isProtected() || mpdu.length < hl + HEADER_LEN + MIC_LEN) return null;
        if ((mpdu[hl + 3] & EXT_IV) == 0) return null;
        long pn = readPn(mpdu, hl);
        int keyId = (mpdu[hl + 3] >>> 6) & 3;
        byte[] sealed = Arrays.copyOfRange(mpdu, hl + HEADER_LEN, mpdu.length);
        byte[] plain = AesCcm.decrypt(tk, nonce(h, pn), aad(h), sealed, MIC_LEN);
        if (plain == null) return null;
        MacHeader uh = h.withFrameControl(h.frameControl() & ~FrameControl.PROTECTED);
        byte[] out = new byte[hl + plain.length];
        uh.encodeInto(out, 0);
        System.arraycopy(plain, 0, out, hl, plain.length);
        return new Decrypted(out, pn, keyId);
    }

    /** Key ID of a protected MPDU, or -1 if it has no valid CCMP header. */
    public static int peekKeyId(byte[] mpdu) {
        try {
            int hl = MacHeader.parse(mpdu).length();
            if (mpdu.length < hl + HEADER_LEN || (mpdu[hl + 3] & EXT_IV) == 0) return -1;
            return (mpdu[hl + 3] >>> 6) & 3;
        } catch (IllegalArgumentException e) {
            return -1;
        }
    }

    public static void writeHeader(byte[] b, int off, long pn, int keyId) {
        b[off] = (byte) pn;
        b[off + 1] = (byte) (pn >>> 8);
        b[off + 2] = 0;
        b[off + 3] = (byte) (((keyId & 3) << 6) | EXT_IV);
        b[off + 4] = (byte) (pn >>> 16);
        b[off + 5] = (byte) (pn >>> 24);
        b[off + 6] = (byte) (pn >>> 32);
        b[off + 7] = (byte) (pn >>> 40);
    }

    public static long readPn(byte[] b, int off) {
        return (b[off] & 0xFFL) | (b[off + 1] & 0xFFL) << 8 | (b[off + 4] & 0xFFL) << 16
                | (b[off + 5] & 0xFFL) << 24 | (b[off + 6] & 0xFFL) << 32 | (b[off + 7] & 0xFFL) << 40;
    }

    /** CCM nonce for a header and PN. */
    public static byte[] nonce(MacHeader h, long pn) {
        byte[] n = new byte[13];
        int flags = h.isQos() ? h.tid() : 0;
        if (h.type() == FrameControl.TYPE_MGMT) flags |= 0x10;
        n[0] = (byte) flags;
        h.addr2().write(n, 1);
        for (int i = 0; i < 6; i++) n[7 + i] = (byte) (pn >>> (40 - 8 * i));
        return n;
    }

    /** Additional authentication data for a header. */
    public static byte[] aad(MacHeader h) {
        int fc = h.frameControl();
        if (h.type() == FrameControl.TYPE_DATA) fc &= ~0x0070;
        fc &= ~(FrameControl.RETRY | FrameControl.PWR_MGT | FrameControl.MORE_DATA);
        fc |= FrameControl.PROTECTED;
        if (h.isQos()) fc &= ~FrameControl.ORDER;
        ByteArrayOutputStream a = new ByteArrayOutputStream(30);
        a.write(fc);
        a.write(fc >>> 8);
        a.writeBytes(h.addr1().bytes());
        a.writeBytes(h.addr2().bytes());
        a.writeBytes(h.addr3().bytes());
        int sc = h.sequenceControl() & 0x000F;
        a.write(sc);
        a.write(sc >>> 8);
        if (h.addr4() != null) a.writeBytes(h.addr4().bytes());
        if (h.isQos()) {
            a.write(h.qosControl() & 0x0F);
            a.write(0);
        }
        return a.toByteArray();
    }
}
