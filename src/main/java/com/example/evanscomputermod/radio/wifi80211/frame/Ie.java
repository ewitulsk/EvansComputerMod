package com.example.evanscomputermod.radio.wifi80211.frame;

import com.example.evanscomputermod.radio.wifi80211.Channels;

import java.io.ByteArrayOutputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** One information element (ID, length, data) plus helpers for element lists. */
public record Ie(int id, byte[] data) {

    public static final int SSID = 0;
    public static final int SUPPORTED_RATES = 1;
    public static final int DS_PARAMS = 3;
    public static final int TIM = 5;
    public static final int ERP = 42;
    public static final int RSN = 48;
    public static final int EXT_SUPPORTED_RATES = 50;
    public static final int VENDOR = 221;

    public Ie {
        if (id < 0 || id > 255) throw new IllegalArgumentException("element ID out of range");
        if (data.length > 255) throw new IllegalArgumentException("element longer than 255 bytes");
        data = data.clone();
    }

    @Override
    public byte[] data() {
        return data.clone();
    }

    /** The element as it appears on the air (ID, length, data). */
    public byte[] encode() {
        byte[] b = new byte[2 + data.length];
        b[0] = (byte) id;
        b[1] = (byte) data.length;
        System.arraycopy(data, 0, b, 2, data.length);
        return b;
    }

    public static Ie ssid(byte[] ssid) {
        return new Ie(SSID, ssid);
    }

    public static Ie dsParams(int channel) {
        return new Ie(DS_PARAMS, new byte[]{(byte) channel});
    }

    /** Minimal TIM: DTIM count 0, DTIM period 1, no buffered traffic. */
    public static Ie tim() {
        return new Ie(TIM, new byte[]{0, 1, 0, 0});
    }

    /** Supported Rates (+ Extended Supported Rates on 2.4 GHz) for a channel. */
    public static List<Ie> rates(int channel) {
        List<Ie> out = new ArrayList<>(2);
        out.add(new Ie(SUPPORTED_RATES, Channels.supportedRates(channel)));
        byte[] ext = Channels.extendedRates(channel);
        if (ext.length > 0) out.add(new Ie(EXT_SUPPORTED_RATES, ext));
        return out;
    }

    public static byte[] encodeAll(List<Ie> ies) {
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        for (Ie ie : ies) bos.writeBytes(ie.encode());
        return bos.toByteArray();
    }

    /** Parses elements from {@code b[off..off+len)}; throws if an element overruns the buffer. */
    public static List<Ie> parseAll(byte[] b, int off, int len) {
        List<Ie> out = new ArrayList<>();
        int p = off;
        int end = off + len;
        while (p < end) {
            if (p + 2 > end) throw new IllegalArgumentException("truncated element header");
            int id = b[p] & 0xFF;
            int l = b[p + 1] & 0xFF;
            if (p + 2 + l > end) throw new IllegalArgumentException("element " + id + " overruns frame");
            out.add(new Ie(id, Arrays.copyOfRange(b, p + 2, p + 2 + l)));
            p += 2 + l;
        }
        return out;
    }

    public static Ie find(List<Ie> ies, int id) {
        for (Ie ie : ies) if (ie.id == id) return ie;
        return null;
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof Ie ie && ie.id == id && Arrays.equals(ie.data, data);
    }

    @Override
    public int hashCode() {
        return id * 31 + Arrays.hashCode(data);
    }

    @Override
    public String toString() {
        return "Ie[" + id + ", " + data.length + "B]";
    }
}
