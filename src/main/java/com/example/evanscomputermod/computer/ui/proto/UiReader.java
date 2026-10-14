package com.example.evanscomputermod.computer.ui.proto;

import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CharsetDecoder;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;

/** Reads little-endian UI protocol fields from one message body, strictly. */
public final class UiReader {
    private final byte[] buf;
    private final int end;
    private int pos;

    public UiReader(byte[] buf, int off, int len) {
        this.buf = buf;
        this.pos = off;
        this.end = off + len;
    }

    public int remaining() {
        return end - pos;
    }

    private void need(int n, String path) throws UiProtoException {
        if (remaining() < n) throw UiProtoException.malformed("truncated: " + path);
    }

    public int u8() throws UiProtoException {
        need(1, "body");
        return buf[pos++] & 0xFF;
    }

    public int u16() throws UiProtoException {
        need(2, "body");
        int v = (buf[pos] & 0xFF) | (buf[pos + 1] & 0xFF) << 8;
        pos += 2;
        return v;
    }

    public int i16() throws UiProtoException {
        return (short) u16();
    }

    public int u32() throws UiProtoException {
        need(4, "body");
        int v = (buf[pos] & 0xFF) | (buf[pos + 1] & 0xFF) << 8 | (buf[pos + 2] & 0xFF) << 16 | (buf[pos + 3] & 0xFF) << 24;
        pos += 4;
        return v;
    }

    public int i32() throws UiProtoException {
        return u32();
    }

    public long u64() throws UiProtoException {
        long lo = u32() & 0xFFFFFFFFL;
        long hi = u32() & 0xFFFFFFFFL;
        return lo | hi << 32;
    }

    public boolean bool(String path) throws UiProtoException {
        int v = u8();
        if (v > 1) throw UiProtoException.malformed("bad bool: " + path);
        return v == 1;
    }

    private int lenPrefix(long max, String path) throws UiProtoException {
        long n = u32() & 0xFFFFFFFFL;
        if (n > max) throw UiProtoException.malformed("too long: " + path);
        if (n > remaining()) throw UiProtoException.malformed("truncated: " + path);
        return (int) n;
    }

    public String str(long max, String path) throws UiProtoException {
        int n = lenPrefix(max, path);
        CharsetDecoder d = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT);
        try {
            String s = d.decode(ByteBuffer.wrap(buf, pos, n)).toString();
            pos += n;
            return s;
        } catch (CharacterCodingException e) {
            throw UiProtoException.malformed("invalid UTF-8: " + path);
        }
    }

    public byte[] bytes(long max, String path) throws UiProtoException {
        int n = lenPrefix(max, path);
        byte[] out = new byte[n];
        System.arraycopy(buf, pos, out, 0, n);
        pos += n;
        return out;
    }

    /** An array count: at most {@code max}, and {@code count * minItem} must fit in what is left. */
    public int count(long max, int minItem, String path) throws UiProtoException {
        long n = u32() & 0xFFFFFFFFL;
        if (n > max) throw UiProtoException.malformed("too many items: " + path);
        if (n * minItem > remaining()) throw UiProtoException.malformed("truncated: " + path);
        return (int) n;
    }

    public void finish() throws UiProtoException {
        if (remaining() != 0) throw UiProtoException.malformed("trailing bytes after the last field");
    }

    /** Compare two strings by their UTF-8 bytes (the order sorted lists use on the wire). */
    public static int compareUtf8(String a, String b) {
        byte[] x = a.getBytes(StandardCharsets.UTF_8);
        byte[] y = b.getBytes(StandardCharsets.UTF_8);
        int n = Math.min(x.length, y.length);
        for (int i = 0; i < n; i++) {
            int c = Integer.compare(x[i] & 0xFF, y[i] & 0xFF);
            if (c != 0) return c;
        }
        return Integer.compare(x.length, y.length);
    }
}
