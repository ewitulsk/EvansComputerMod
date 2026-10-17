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
     * functions returning zero values, except entropy imports
     * ({@link #mustNotStub}), which fail with {@link WasmTrap.Kind#LINK_ERROR}.
     */
    WasmInstance instantiate(WasmModuleHandle module, List<WasmHostFunc> imports) throws WasmTrap;

    /**
     * Imports that must never be auto-stubbed: a zero-returning stub for an
     * entropy source reports success and leaves the buffer untouched, so the
     * program silently runs on all-zero "random" bytes (keys, nonces). A
     * module importing one the host does not supply fails to instantiate.
     */
    static boolean mustNotStub(String module, String name) {
        return name.equals("random_get")
                || name.startsWith("__getrandom")
                || name.equals("getentropy")
                || name.startsWith("arc4random");
    }

    /**
     * What a stub for an import the host doesn't supply returns (as an i32 /
     * i64 result). A WASI preview1 function that is missing answers ENOSYS (52):
     * its result is an errno, and 0 would claim success with the out-parameters
     * never written (path_readlink "succeeding" with a garbage length sent vim
     * writing far past its buffer). Everything else returns 0.
     */
    static long stubResult(String module, String name) {
        return "wasi_snapshot_preview1".equals(module) || "wasi_unstable".equals(module) ? 52 : 0;
    }

    /** Provider name for diagnostics: {@code "chicory"} / {@code "wasmtime"}. */
    String providerName();

    /** Default no-op close — providers override only if they hold resources. */
    @Override
    default void close() {}
}
