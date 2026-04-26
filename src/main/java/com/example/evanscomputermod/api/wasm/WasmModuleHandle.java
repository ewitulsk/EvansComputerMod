package com.example.evanscomputermod.api.wasm;

import java.util.List;

/**
 * Provider-opaque parsed module handle. Reusable across multiple
 * {@link WasmRuntime#instantiate(WasmModuleHandle, List)} calls.
 */
public interface WasmModuleHandle {

    /**
     * The module's import descriptors in declaration order. Used by
     * call-sites that build per-instance host-function tables (e.g. to
     * generate stub imports for functions the host doesn't supply).
     */
    List<ImportDescriptor> imports();

    /** Names of exported functions on this module. */
    List<String> exportedFunctions();

    /**
     * Description of one declared import. Function imports populate
     * {@code params} and {@code results}; non-function imports leave them
     * empty.
     */
    final class ImportDescriptor {
        public enum Kind { FUNCTION, MEMORY, GLOBAL, TABLE, OTHER }

        public final String moduleName;
        public final String fieldName;
        public final Kind kind;
        public final List<WasmValType> params;
        public final List<WasmValType> results;

        public ImportDescriptor(String moduleName, String fieldName, Kind kind,
                                List<WasmValType> params, List<WasmValType> results) {
            this.moduleName = moduleName;
            this.fieldName = fieldName;
            this.kind = kind;
            this.params = List.copyOf(params);
            this.results = List.copyOf(results);
        }
    }
}
