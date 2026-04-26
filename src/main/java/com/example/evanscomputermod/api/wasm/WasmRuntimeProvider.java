package com.example.evanscomputermod.api.wasm;

/**
 * ServiceLoader-discoverable factory for a {@link WasmRuntime}. The main mod
 * registers a Chicory provider; an optional sidecar mod can register a
 * Wasmtime provider with higher priority. The highest-priority available
 * provider wins at server start.
 */
public interface WasmRuntimeProvider {

    /**
     * Bumped on any breaking change to the SPI. Sidecars must declare the
     * same value here that they were compiled against; the registry rejects
     * providers whose {@link #apiVersion()} differs from
     * {@link #CURRENT_API_VERSION}.
     */
    int CURRENT_API_VERSION = 1;

    /** Short identifier for diagnostics: {@code "chicory"} / {@code "wasmtime"}. */
    String name();

    /** Higher wins. Suggested values: chicory=0, wasmtime=100. */
    int priority();

    /** Must equal {@link #CURRENT_API_VERSION} for the provider to be loaded. */
    int apiVersion();

    /**
     * False if the provider's required classes/natives could not be loaded.
     * Implementations should swallow exceptions here and return {@code false}
     * rather than letting them escape — this is the safe-fallback path.
     */
    boolean isAvailable();

    /** Create the runtime. Called at most once per server lifecycle. */
    WasmRuntime create();
}
