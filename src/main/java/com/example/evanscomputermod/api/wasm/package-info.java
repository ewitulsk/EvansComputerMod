/**
 * Runtime-agnostic SPI for hosting WASM execution inside the mod.
 *
 * <p>The main mod ships a Chicory-backed implementation. A separate sidecar
 * mod can provide a Wasmtime-backed implementation; whichever provider has
 * the highest priority and is available wins at server start.
 *
 * <p>Stability: treat the public types in this package as stable across
 * mod versions. Bump {@code WasmRuntimeProvider.CURRENT_API_VERSION} on any
 * breaking change so older sidecars are rejected at load time.
 */
package com.example.evanscomputermod.api.wasm;
