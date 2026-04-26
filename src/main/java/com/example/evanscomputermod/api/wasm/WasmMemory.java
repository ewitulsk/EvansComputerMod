package com.example.evanscomputermod.api.wasm;

import java.nio.charset.StandardCharsets;

/**
 * Linear memory of a WASM instance. All multi-byte reads/writes are
 * little-endian (WASM's only endianness).
 *
 * <p>Implementations are not required to be thread-safe; treat each instance
 * as pinned to a single owning thread, the same as {@link WasmInstance}.
 */
public interface WasmMemory {

    /** Current size in bytes. */
    int size();

    /** Current size in 64 KiB pages. */
    int pages();

    /**
     * Try to grow the memory by {@code deltaPages} pages. Returns the previous
     * page count on success, or {@code -1} on failure.
     */
    int grow(int deltaPages);

    // --- byte ---

    byte readByte(int addr);
    void writeByte(int addr, byte v);

    // --- short (i16) ---

    short readShort(int addr);
    void writeShort(int addr, short v);

    // --- int (i32) ---

    int readInt(int addr);
    void writeInt(int addr, int v);

    // --- long (i64) ---

    long readLong(int addr);
    void writeLong(int addr, long v);

    // --- float / double ---

    float readFloat(int addr);
    void writeFloat(int addr, float v);

    double readDouble(int addr);
    void writeDouble(int addr, double v);

    // --- bytes / strings ---

    /** Read {@code len} bytes starting at {@code addr}. */
    byte[] readBytes(int addr, int len);

    /** Write the entire byte array. */
    default void writeBytes(int addr, byte[] src) {
        writeBytes(addr, src, 0, src.length);
    }

    /** Write {@code len} bytes from {@code src} starting at {@code srcOff}. */
    void writeBytes(int addr, byte[] src, int srcOff, int len);

    /** Read a UTF-8 string of exactly {@code len} bytes. */
    default String readString(int addr, int len) {
        return new String(readBytes(addr, len), StandardCharsets.UTF_8);
    }

    /** Write a UTF-8 string. */
    default void writeString(int addr, String s) {
        writeBytes(addr, s.getBytes(StandardCharsets.UTF_8));
    }

    /**
     * Read a NUL-terminated UTF-8 string starting at {@code addr}, scanning
     * up to {@code maxLen} bytes. If no NUL is found, returns the whole range.
     */
    default String readCString(int addr, int maxLen) {
        int end = addr;
        int limit = addr + maxLen;
        while (end < limit && readByte(end) != 0) {
            end++;
        }
        return readString(addr, end - addr);
    }
}
