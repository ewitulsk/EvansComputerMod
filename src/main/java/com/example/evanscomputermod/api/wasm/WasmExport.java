package com.example.evanscomputermod.api.wasm;

/**
 * A pre-resolved exported WASM function. Returned by
 * {@link WasmInstance#export(String)}; calling repeatedly is cheaper than
 * looking up the export by name on every call.
 */
@FunctionalInterface
public interface WasmExport {
    long[] call(long... args) throws WasmTrap;
}
