package com.example.evanscomputermod.wasm.chicory;

import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmModuleHandle;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.api.wasm.WasmValType;

import com.dylibso.chicory.runtime.HostFunction;
import com.dylibso.chicory.runtime.ImportValues;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.wasm.ChicoryException;
import com.dylibso.chicory.wasm.Parser;
import com.dylibso.chicory.wasm.UnlinkableException;
import com.dylibso.chicory.wasm.WasmModule;
import com.dylibso.chicory.wasm.types.Export;
import com.dylibso.chicory.wasm.types.ExternalType;
import com.dylibso.chicory.wasm.types.FunctionImport;
import com.dylibso.chicory.wasm.types.FunctionType;
import com.dylibso.chicory.wasm.types.Import;
import com.dylibso.chicory.wasm.types.ValType;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Chicory-backed implementation of {@link WasmRuntime}. Stateless apart from
 * its provider name; modules can be parsed concurrently and instances run on
 * any thread (one thread each at a time).
 */
final class ChicoryRuntime implements WasmRuntime {

    @Override
    public String providerName() {
        return "chicory";
    }

    @Override
    public WasmModuleHandle compile(byte[] wasmBytes) throws WasmTrap {
        try {
            WasmModule module = Parser.parse(wasmBytes);
            return new ChicoryModuleHandle(module);
        } catch (ChicoryException e) {
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, "WASM parse failed: " + e.getMessage(), e);
        }
    }

    @Override
    public WasmInstance instantiate(WasmModuleHandle module, List<WasmHostFunc> imports) throws WasmTrap {
        ChicoryModuleHandle handle = (ChicoryModuleHandle) module;
        WasmModule wm = handle.module;

        // Index supplied imports by (moduleName, fieldName) — first match wins.
        Map<String, WasmHostFunc> supplied = new HashMap<>();
        for (WasmHostFunc f : imports) {
            supplied.putIfAbsent(f.moduleName() + "\0" + f.fieldName(), f);
        }

        // Walk the module's import list in order. For each function import:
        // either match against `supplied` (rebuilding to Chicory's signature so
        // type-check passes regardless of caller-declared types) or insert a
        // no-op stub. Non-function imports are not handled here — Chicory's
        // builder will error on them, which is what we want.
        ChicoryInstance shell = new ChicoryInstance();
        List<HostFunction> hostFuncs = new ArrayList<>();
        int importCount = wm.importSection().importCount();
        for (int i = 0; i < importCount; i++) {
            Import imp = wm.importSection().getImport(i);
            if (imp.importType() != ExternalType.FUNCTION) continue;

            FunctionImport fimp = (FunctionImport) imp;
            FunctionType fty = wm.typeSection().getType(fimp.typeIndex());

            String key = imp.module() + "\0" + imp.name();
            WasmHostFunc handler = supplied.get(key);

            if (handler != null) {
                hostFuncs.add(new HostFunction(
                        imp.module(),
                        imp.name(),
                        fty,
                        new HostHandlerAdapter(shell, handler.handler())));
            } else {
                // Auto-stub: log once, return zeros.
                hostFuncs.add(new HostFunction(
                        imp.module(),
                        imp.name(),
                        fty,
                        new StubHandler(fty)));
            }
        }

        ImportValues iv = ImportValues.builder()
                .addFunction(hostFuncs.toArray(new HostFunction[0]))
                .build();

        try {
            Instance inst = Instance.builder(wm)
                    .withImportValues(iv)
                    .withStart(false)         // we drive entry points manually
                    .build();
            shell.attach(inst);
            return shell;
        } catch (UnlinkableException e) {
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, e.getMessage(), e);
        } catch (ChicoryException e) {
            throw new WasmTrap(WasmTrap.Kind.EXEC_ERROR, e.getMessage(), e);
        }
    }

    // --- helpers ---

    private static final class ChicoryModuleHandle implements WasmModuleHandle {
        final WasmModule module;
        ChicoryModuleHandle(WasmModule module) { this.module = module; }

        @Override
        public List<ImportDescriptor> imports() {
            int n = module.importSection().importCount();
            List<ImportDescriptor> out = new ArrayList<>(n);
            for (int i = 0; i < n; i++) {
                Import imp = module.importSection().getImport(i);
                ImportDescriptor.Kind k;
                List<WasmValType> p = List.of();
                List<WasmValType> r = List.of();
                switch (imp.importType()) {
                    case FUNCTION: {
                        k = ImportDescriptor.Kind.FUNCTION;
                        FunctionImport fi = (FunctionImport) imp;
                        FunctionType ft = module.typeSection().getType(fi.typeIndex());
                        p = mapTypes(ft.params());
                        r = mapTypes(ft.returns());
                        break;
                    }
                    case MEMORY: k = ImportDescriptor.Kind.MEMORY; break;
                    case GLOBAL: k = ImportDescriptor.Kind.GLOBAL; break;
                    case TABLE:  k = ImportDescriptor.Kind.TABLE; break;
                    default:     k = ImportDescriptor.Kind.OTHER; break;
                }
                out.add(new ImportDescriptor(imp.module(), imp.name(), k, p, r));
            }
            return out;
        }

        @Override
        public List<String> exportedFunctions() {
            List<String> out = new ArrayList<>();
            int n = module.exportSection().exportCount();
            for (int i = 0; i < n; i++) {
                Export e = module.exportSection().getExport(i);
                if (e.exportType() == ExternalType.FUNCTION) out.add(e.name());
            }
            return out;
        }
    }

    private static List<WasmValType> mapTypes(List<ValType> types) {
        List<WasmValType> out = new ArrayList<>(types.size());
        for (ValType t : types) out.add(toApi(t));
        return out;
    }

    static WasmValType toApi(ValType t) {
        if (t.equals(ValType.I32)) return WasmValType.I32;
        if (t.equals(ValType.I64)) return WasmValType.I64;
        if (t.equals(ValType.F32)) return WasmValType.F32;
        if (t.equals(ValType.F64)) return WasmValType.F64;
        // Reference types and v128 map to I32 just for descriptor reporting; we
        // never construct these as host imports ourselves.
        return WasmValType.I32;
    }

    /** Bridges the SPI handler signature to Chicory's {@code WasmFunctionHandle}. */
    private static final class HostHandlerAdapter implements com.dylibso.chicory.runtime.WasmFunctionHandle {
        private final ChicoryInstance shell;
        private final WasmHostFunc.Handler handler;

        HostHandlerAdapter(ChicoryInstance shell, WasmHostFunc.Handler handler) {
            this.shell = shell;
            this.handler = handler;
        }

        @Override
        public long[] apply(Instance instance, long... args) {
            try {
                long[] result = handler.invoke(shell, args);
                return result == null ? new long[0] : result;
            } catch (WasmTrap e) {
                throw new WasmTrapBridge(e);
            }
        }
    }

    /** Default zero-returning stub for function imports the host doesn't supply. */
    private static final class StubHandler implements com.dylibso.chicory.runtime.WasmFunctionHandle {
        private final int returnArity;
        StubHandler(FunctionType ft) {
            this.returnArity = ft.returns().size();
        }
        @Override
        public long[] apply(Instance instance, long... args) {
            return returnArity == 0 ? new long[0] : new long[returnArity];
        }
    }

    /**
     * Carries a {@link WasmTrap} out of a Chicory host call so
     * {@link ChicoryInstance#callExport(String, long...)} can unwrap it
     * cleanly. Subclassing {@link ChicoryException} lets it propagate
     * unchanged through Chicory's own try/catch sites.
     */
    static final class WasmTrapBridge extends ChicoryException {
        final WasmTrap trap;
        WasmTrapBridge(WasmTrap trap) {
            super(trap.getMessage(), trap);
            this.trap = trap;
        }
    }
}
