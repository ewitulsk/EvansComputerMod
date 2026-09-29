package com.example.evanscomputermod.computer.peripheral;

import org.jetbrains.annotations.Nullable;

import java.io.ByteArrayOutputStream;
import java.lang.reflect.Array;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Tagged binary value encoding shared with {@code ecm_host_abi::peripheral}
 * (rust/crates/ecm-host-abi/src/peripheral.rs). All integers little-endian.
 *
 * <pre>
 * value  = tag:u8 payload
 *   0 NIL    -
 *   1 STR    len:u32 utf8[len]
 *   2 I32    i32
 *   3 I64    i64
 *   4 F64    f64
 *   5 BOOL   u8 (0 / 1)
 *   6 LIST   count:u32 value[count]
 *   7 MAP    count:u32 (key:value val:value)[count]
 *   8 BYTES  len:u32 u8[len]
 * result = status:u8 value      status 0 = ok, 1 = error (value is a STR message)
 * </pre>
 *
 * Java to wire: {@code null}, Boolean, Byte/Short/Integer (I32), Long (I64),
 * Float/Double (F64), CharSequence/Character/Enum (STR, enums lower-case),
 * {@code byte[]} (BYTES), Map (MAP), Collection, arrays and Optional (LIST /
 * inner value); anything else is sent as its {@code toString()}.
 * Wire to Java: NIL null, STR String, I32 Integer, I64 Long, F64 Double,
 * BOOL Boolean, LIST ArrayList, MAP LinkedHashMap, BYTES byte[].
 */
public final class PeripheralValues {

    public static final byte NIL = 0, STR = 1, I32 = 2, I64 = 3, F64 = 4, BOOL = 5, LIST = 6, MAP = 7, BYTES = 8;
    public static final byte STATUS_OK = 0, STATUS_ERROR = 1;

    private static final int MAX_DEPTH = 32;
    private static final int MAX_ELEMENTS = 1 << 20;

    private PeripheralValues() {
    }

    public static final class DecodeException extends Exception {
        DecodeException(String message) {
            super(message);
        }
    }

    // ------------------------------------------------------------ encoding

    public static byte[] ok(@Nullable Object value) {
        Writer w = new Writer();
        w.out.write(STATUS_OK);
        w.value(value, 0);
        return w.out.toByteArray();
    }

    public static byte[] error(String message) {
        Writer w = new Writer();
        w.out.write(STATUS_ERROR);
        w.value(message == null ? "error" : message, 0);
        return w.out.toByteArray();
    }

    public static byte[] encode(@Nullable Object value) {
        Writer w = new Writer();
        w.value(value, 0);
        return w.out.toByteArray();
    }

    private static final class Writer {
        final ByteArrayOutputStream out = new ByteArrayOutputStream(64);
        private final ByteBuffer scratch = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN);

        void u32(int v) {
            scratch.clear();
            scratch.putInt(v);
            out.write(scratch.array(), 0, 4);
        }

        void i64(long v) {
            scratch.clear();
            scratch.putLong(v);
            out.write(scratch.array(), 0, 8);
        }

        void bytes(byte tag, byte[] b) {
            out.write(tag);
            u32(b.length);
            out.write(b, 0, b.length);
        }

        void value(@Nullable Object v, int depth) {
            if (depth > MAX_DEPTH) {
                throw new IllegalArgumentException("value nested too deeply");
            }
            if (v == null) {
                out.write(NIL);
            } else if (v instanceof Boolean b) {
                out.write(BOOL);
                out.write(b ? 1 : 0);
            } else if (v instanceof Integer || v instanceof Short || v instanceof Byte) {
                out.write(I32);
                u32(((Number) v).intValue());
            } else if (v instanceof Long l) {
                out.write(I64);
                i64(l);
            } else if (v instanceof Float || v instanceof Double) {
                out.write(F64);
                i64(Double.doubleToRawLongBits(((Number) v).doubleValue()));
            } else if (v instanceof Number n) {
                out.write(F64);
                i64(Double.doubleToRawLongBits(n.doubleValue()));
            } else if (v instanceof CharSequence || v instanceof Character) {
                bytes(STR, v.toString().getBytes(StandardCharsets.UTF_8));
            } else if (v instanceof Enum<?> e) {
                bytes(STR, e.name().toLowerCase(java.util.Locale.ROOT).getBytes(StandardCharsets.UTF_8));
            } else if (v instanceof byte[] b) {
                bytes(BYTES, b);
            } else if (v instanceof Map<?, ?> m) {
                out.write(MAP);
                u32(m.size());
                for (Map.Entry<?, ?> e : m.entrySet()) {
                    value(e.getKey(), depth + 1);
                    value(e.getValue(), depth + 1);
                }
            } else if (v instanceof Collection<?> c) {
                out.write(LIST);
                u32(c.size());
                for (Object o : c) value(o, depth + 1);
            } else if (v instanceof Optional<?> o) {
                value(o.orElse(null), depth);
            } else if (v.getClass().isArray()) {
                int n = Array.getLength(v);
                out.write(LIST);
                u32(n);
                for (int i = 0; i < n; i++) value(Array.get(v, i), depth + 1);
            } else {
                bytes(STR, v.toString().getBytes(StandardCharsets.UTF_8));
            }
        }
    }

    // ------------------------------------------------------------ decoding

    /** Decode one value that fills {@code data} exactly. */
    @Nullable
    public static Object decode(byte[] data) throws DecodeException {
        ByteBuffer buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        Object v = read(buf, 0);
        if (buf.hasRemaining()) {
            throw new DecodeException("trailing bytes after value");
        }
        return v;
    }

    /** Decode an argument list (a LIST value, or empty input for no arguments). */
    public static Object[] decodeArgs(byte[] data) throws DecodeException {
        if (data.length == 0) return new Object[0];
        Object v = decode(data);
        if (!(v instanceof List<?> list)) {
            throw new DecodeException("arguments must be a list");
        }
        return list.toArray();
    }

    @Nullable
    private static Object read(ByteBuffer buf, int depth) throws DecodeException {
        if (depth > MAX_DEPTH) throw new DecodeException("value nested too deeply");
        need(buf, 1);
        byte tag = buf.get();
        switch (tag) {
            case NIL:
                return null;
            case STR:
                return new String(readBlob(buf), StandardCharsets.UTF_8);
            case I32:
                need(buf, 4);
                return buf.getInt();
            case I64:
                need(buf, 8);
                return buf.getLong();
            case F64:
                need(buf, 8);
                return buf.getDouble();
            case BOOL:
                need(buf, 1);
                return buf.get() != 0;
            case LIST: {
                int n = count(buf);
                List<Object> list = new ArrayList<>(Math.min(n, 1024));
                for (int i = 0; i < n; i++) list.add(read(buf, depth + 1));
                return list;
            }
            case MAP: {
                int n = count(buf);
                Map<Object, Object> map = new LinkedHashMap<>();
                for (int i = 0; i < n; i++) {
                    Object k = read(buf, depth + 1);
                    map.put(k, read(buf, depth + 1));
                }
                return map;
            }
            case BYTES:
                return readBlob(buf);
            default:
                throw new DecodeException("unknown value tag " + tag);
        }
    }

    private static int count(ByteBuffer buf) throws DecodeException {
        need(buf, 4);
        int n = buf.getInt();
        if (n < 0 || n > MAX_ELEMENTS || n > buf.remaining()) {
            throw new DecodeException("bad element count " + Integer.toUnsignedString(n));
        }
        return n;
    }

    private static byte[] readBlob(ByteBuffer buf) throws DecodeException {
        need(buf, 4);
        int n = buf.getInt();
        if (n < 0 || n > buf.remaining()) {
            throw new DecodeException("bad length " + Integer.toUnsignedString(n));
        }
        byte[] b = new byte[n];
        buf.get(b);
        return b;
    }

    private static void need(ByteBuffer buf, int n) throws DecodeException {
        if (buf.remaining() < n) throw new DecodeException("truncated value");
    }
}
