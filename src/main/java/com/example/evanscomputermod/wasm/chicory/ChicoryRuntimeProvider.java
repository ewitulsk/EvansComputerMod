package com.example.evanscomputermod.wasm.chicory;

import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmRuntimeProvider;

/**
 * Default provider bundled in the main mod. Pure-Java; always available.
 */
public final class ChicoryRuntimeProvider implements WasmRuntimeProvider {

    @Override public String name() { return "chicory"; }
    @Override public int priority() { return 0; }
    @Override public int apiVersion() { return WasmRuntimeProvider.CURRENT_API_VERSION; }

    @Override
    public boolean isAvailable() {
        try {
            Class.forName("com.dylibso.chicory.runtime.Instance");
            return true;
        } catch (Throwable t) {
            return false;
        }
    }

    @Override
    public WasmRuntime create() {
        return new ChicoryRuntime();
    }
}
