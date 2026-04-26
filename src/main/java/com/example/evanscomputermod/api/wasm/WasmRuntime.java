package com.example.evanscomputermod.api.wasm;

import java.util.List;

/**
 * Factory for parsing modules and creating instances. One {@code WasmRuntime}
 * is shared across all computers in a server; it is not thread-pinned. The
 * {@link WasmInstance}s it creates are.
 */
public interface WasmRuntime extends AutoCloseable {

    /** Parse a WASM binary; safe to call from any thread. */
    WasmModuleHandle compile(byte[] wasmBytes) throws WasmTrap;

    /**
     * Build a new instance from a parsed module and a list of host-function
     * imports. Imports are matched by ({@code moduleName}, {@code fieldName})
     * against the module's declared imports; non-function imports and any
     * function imports the caller did not supply are stubbed with no-op
     * functions returning zero values.
     */
    WasmInstance instantiate(WasmModuleHandle module, List<WasmHostFunc> imports) throws WasmTrap;

    /** Provider name for diagnostics: {@code "chicory"} / {@code "wasmtime"}. */
    String providerName();

    /** Default no-op close — providers override only if they hold resources. */
    @Override
    default void close() {}
}
