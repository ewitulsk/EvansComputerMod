package com.example.evanscomputermod.radio.wifi80211.frame;

import java.util.Arrays;

/**
 * An 802.11 MPDU without FCS: MAC header plus frame body. For protected
 * frames the body starts with the CCMP header and ends with the MIC.
 */
public record Frame80211(MacHeader header, byte[] body) {

    public Frame80211 {
        body = body == null ? new byte[0] : body;
    }

    public static Frame80211 parse(byte[] mpdu) {
        MacHeader h = MacHeader.parse(mpdu);
        return new Frame80211(h, Arrays.copyOfRange(mpdu, h.length(), mpdu.length));
    }

    public byte[] encode() {
        int hl = header.length();
        byte[] out = new byte[hl + body.length];
        header.encodeInto(out, 0);
        System.arraycopy(body, 0, out, hl, body.length);
        return out;
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof Frame80211 f && header.equals(f.header) && Arrays.equals(body, f.body);
    }

    @Override
    public int hashCode() {
        return header.hashCode() * 31 + Arrays.hashCode(body);
    }

    @Override
    public String toString() {
        return "Frame80211[" + header + ", body=" + body.length + "B]";
    }
}
