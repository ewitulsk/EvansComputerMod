package com.example.evanscomputermod.api.wasm;

/**
 * One instantiated WASM module. Pinned to a single owning thread — the
 * thread that calls {@link #callExport(String, long...)} or any
 * {@link WasmExport#call(long...)} returned from {@link #export(String)}.
 *
 * <p>Cancellation: {@link #requestInterrupt()} may be called from any thread.
 * It causes the next instruction (or, on best-effort runtimes, the next host
 * call boundary) to throw {@link WasmTrap} with kind
 * {@link WasmTrap.Kind#INTERRUPTED}. Always call {@link #clearInterrupt()}
 * after handling the trap on the owning thread before reusing the instance.
 */
public interface WasmInstance extends AutoCloseable {

    /** True if the module exports a function with this name. */
    boolean hasExport(String name);

    /** One-shot export call. Throws if the export does not exist. */
    long[] callExport(String name, long... args) throws WasmTrap;

    /**
     * Resolve an exported function once for repeated calls. Returns
     * {@code null} if the export does not exist; never throws.
     */
    WasmExport export(String name);

    /** The instance's primary linear memory, or {@code null} if not exported. */
    WasmMemory memory();

    /** Request best-effort cancellation of any in-flight call on this instance. */
    void requestInterrupt();

    /**
     * Clear the cancellation request and any thread-level interrupt state on
     * the owning thread. Call after catching {@link WasmTrap.Kind#INTERRUPTED}
     * before reusing the instance.
     */
    void clearInterrupt();

    @Override
    void close();
}
