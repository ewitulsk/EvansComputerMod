package com.example.evanscomputermod.api.wasm;

import java.util.List;
import java.util.Objects;

/**
 * Host function descriptor: a single ({@code module}, {@code name}) entry
 * point supplied to the WASM module at instantiate time.
 *
 * <p>Encoding for {@code long[]} params and results:
 * <ul>
 *   <li>i32: low 32 bits of the {@code long}, sign-extended on read with {@code (int) value}</li>
 *   <li>i64: the {@code long} verbatim</li>
 *   <li>f32: bit pattern via {@link Float#intBitsToFloat(int)} on the low 32 bits</li>
 *   <li>f64: bit pattern via {@link Double#longBitsToDouble(long)}</li>
 * </ul>
 *
 * <p>Handlers may return {@code null} for void functions, or a {@code long[]}
 * matching the declared {@code results} arity. Throwing {@link WasmTrap}
 * propagates through the runtime back to the caller of {@code callExport}.
 */
public final class WasmHostFunc {

    @FunctionalInterface
    public interface Handler {
        /**
         * @param instance the WASM instance making the call (memory access, etc)
         * @param args     params encoded into {@code long[]} per the rules above
         * @return result array sized to declared returns, or {@code null} for void
         */
        long[] invoke(WasmInstance instance, long[] args) throws WasmTrap;
    }

    private final String moduleName;
    private final String fieldName;
    private final List<WasmValType> params;
    private final List<WasmValType> results;
    private final Handler handler;

    public WasmHostFunc(String moduleName,
                        String fieldName,
                        List<WasmValType> params,
                        List<WasmValType> results,
                        Handler handler) {
        this.moduleName = Objects.requireNonNull(moduleName);
        this.fieldName = Objects.requireNonNull(fieldName);
        this.params = List.copyOf(params);
        this.results = List.copyOf(results);
        this.handler = Objects.requireNonNull(handler);
    }

    public String moduleName() { return moduleName; }
    public String fieldName() { return fieldName; }
    public List<WasmValType> params() { return params; }
    public List<WasmValType> results() { return results; }
    public Handler handler() { return handler; }

    // --- Convenience helpers for handlers --------------------------------

    /** Empty result for void host functions. Equivalent to returning {@code null}. */
    public static final long[] VOID = null;

    /** Wrap a single i32 return. */
    public static long[] retI32(int v) {
        return new long[] { (long) v };
    }

    /** Wrap a single i64 return. */
    public static long[] retI64(long v) {
        return new long[] { v };
    }

    /** Wrap a single f32 return. */
    public static long[] retF32(float v) {
        return new long[] { Integer.toUnsignedLong(Float.floatToRawIntBits(v)) };
    }

    /** Wrap a single f64 return. */
    public static long[] retF64(double v) {
        return new long[] { Double.doubleToRawLongBits(v) };
    }

    /** Read an f32 argument. */
    public static float argF32(long bits) {
        return Float.intBitsToFloat((int) bits);
    }

    /** Read an f64 argument. */
    public static double argF64(long bits) {
        return Double.longBitsToDouble(bits);
    }
}
