package com.example.evanscomputermod.computer.ui.proto;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/** Appends little-endian UI protocol fields to a growable buffer. */
public final class UiWriter {
    private byte[] buf;
    private int len;

    public UiWriter() {
        this(256);
    }

    public UiWriter(int capacity) {
        buf = new byte[Math.max(16, capacity)];
    }

    private void ensure(int extra) {
        if (len + extra > buf.length) buf = Arrays.copyOf(buf, Math.max(buf.length * 2, len + extra));
    }

    public int length() {
        return len;
    }

    public byte[] toByteArray() {
        return Arrays.copyOf(buf, len);
    }

    public void reset() {
        len = 0;
    }

    public void u8(int v) {
        ensure(1);
        buf[len++] = (byte) v;
    }

    public void u16(int v) {
        ensure(2);
        buf[len++] = (byte) v;
        buf[len++] = (byte) (v >>> 8);
    }

    public void i16(int v) {
        u16(v);
    }

    public void u32(int v) {
        ensure(4);
        buf[len++] = (byte) v;
        buf[len++] = (byte) (v >>> 8);
        buf[len++] = (byte) (v >>> 16);
        buf[len++] = (byte) (v >>> 24);
    }

    public void i32(int v) {
        u32(v);
    }

    public void u64(long v) {
        u32((int) v);
        u32((int) (v >>> 32));
    }

    public void bool(boolean v) {
        u8(v ? 1 : 0);
    }

    public void str(String s) {
        bytes(s.getBytes(StandardCharsets.UTF_8));
    }

    public void bytes(byte[] b) {
        u32(b.length);
        ensure(b.length);
        System.arraycopy(b, 0, buf, len, b.length);
        len += b.length;
    }

    /** Overwrite 4 bytes at {@code pos} (used to patch a frame length). */
    void patchU32(int pos, int v) {
        buf[pos] = (byte) v;
        buf[pos + 1] = (byte) (v >>> 8);
        buf[pos + 2] = (byte) (v >>> 16);
        buf[pos + 3] = (byte) (v >>> 24);
    }
}
