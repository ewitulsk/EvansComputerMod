package com.example.evanscomputermod.wasmtime;

import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmRuntimeProvider;

/**
 * Sidecar provider. Higher priority than the bundled Chicory provider, so
 * if the wasmtime native libs successfully load on this platform, the main
 * mod chooses this implementation.
 */
public final class WasmtimeRuntimeProvider implements WasmRuntimeProvider {

    @Override public String name() { return "wasmtime"; }
    @Override public int priority() { return 100; }
    @Override public int apiVersion() { return WasmRuntimeProvider.CURRENT_API_VERSION; }

    @Override
    public boolean isAvailable() {
        // Probe for the wasmtime-java Engine class. If the native library
        // can't be loaded (e.g. unsupported OS/arch), this throws and we
        // gracefully fall back to Chicory.
        try {
            Class<?> engineClass = Class.forName("io.github.kawamuray.wasmtime.Engine");
            engineClass.getDeclaredConstructor().newInstance();
            return true;
        } catch (Throwable t) {
            return false;
        }
    }

    @Override
    public WasmRuntime create() {
        return new WasmtimeRuntime();
    }
}
