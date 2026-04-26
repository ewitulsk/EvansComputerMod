package com.example.evanscomputermod.api.wasm;

/**
 * The four numeric WASM value types this mod uses across the host boundary.
 * Reference types and v128 are not exposed here — none of the kernel or WASI
 * imports need them.
 */
public enum WasmValType {
    I32,
    I64,
    F32,
    F64
}
