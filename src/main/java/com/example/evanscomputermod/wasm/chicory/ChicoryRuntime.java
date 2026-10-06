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
 * Chicory-backed implementation of {@link WasmRuntime}. Modules can be parsed
 * concurrently and instances run on any thread (one thread each at a time).
 *
 * <p><b>Compilation.</b> Parsed modules are cached by content hash, so a
 * program is parsed once per server rather than on every launch. Each cached
 * module is also compiled to JVM bytecode with Chicory's compiler (its
 * functions become Java methods the JIT can optimise, many times faster than
 * the interpreter). Small modules compile before their first run; large ones
 * (e.g. the Python interpreter) compile in the background and run interpreted
 * until the compiled code is ready. If the compiler is unavailable or fails on
 * a module, that module stays interpreted.
 */
final class ChicoryRuntime implements WasmRuntime {

    /** Modules up to this size are compiled before their first instance. */
    private static final int SYNC_COMPILE_MAX_BYTES = 2 * 1024 * 1024;

    private static final java.util.concurrent.ExecutorService BACKGROUND_COMPILER =
            java.util.concurrent.Executors.newSingleThreadExecutor(r -> {
                Thread t = new Thread(r, "ECM-Chicory-Compiler");
                t.setDaemon(true);
                t.setPriority(Thread.MIN_PRIORITY);
                return t;
            });

    /** Parsed + compiled modules by SHA-256 of their bytes. */
    private final Map<String, ChicoryModuleHandle> cache = new java.util.concurrent.ConcurrentHashMap<>();

    private final boolean compilerEnabled;
    private volatile boolean compilerBroken;

    ChicoryRuntime() {
        this(true);
    }

    ChicoryRuntime(boolean compilerEnabled) {
        this.compilerEnabled = compilerEnabled;
    }

    @Override
    public String providerName() {
        return "chicory";
    }

    @Override
    public WasmModuleHandle compile(byte[] wasmBytes) throws WasmTrap {
        String key = sha256(wasmBytes);
        ChicoryModuleHandle cached = cache.get(key);
        if (cached != null) return cached;
        ChicoryModuleHandle handle;
        try {
            handle = new ChicoryModuleHandle(Parser.parse(wasmBytes));
        } catch (ChicoryException e) {
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, "WASM parse failed: " + e.getMessage(), e);
        }
        ChicoryModuleHandle raced = cache.putIfAbsent(key, handle);
        if (raced != null) return raced;
        if (compilerEnabled && !compilerBroken) {
            if (wasmBytes.length <= SYNC_COMPILE_MAX_BYTES) {
                compileToBytecode(handle, wasmBytes.length);
            } else {
                BACKGROUND_COMPILER.execute(() -> compileToBytecode(handle, wasmBytes.length));
            }
        }
        return handle;
    }

    /** Compile a module's functions to JVM bytecode; on failure it stays interpreted. */
    private void compileToBytecode(ChicoryModuleHandle handle, int size) {
        long t0 = System.nanoTime();
        try {
            handle.machineFactory = com.dylibso.chicory.compiler.MachineFactoryCompiler
                    .builder(handle.module)
                    // Functions too big for one JVM method run in the interpreter.
                    .withInterpreterFallback(com.dylibso.chicory.compiler.InterpreterFallback.SILENT)
                    .compile();
            com.example.evanscomputermod.EvansComputerMod.LOGGER.info(
                    "Chicory: compiled a {} KiB module to bytecode in {} ms",
                    size / 1024, (System.nanoTime() - t0) / 1_000_000);
        } catch (LinkageError e) {
            // The compiler (or ASM) isn't on the classpath: don't try again.
            compilerBroken = true;
            com.example.evanscomputermod.EvansComputerMod.LOGGER.warn(
                    "Chicory compiler unavailable, programs will be interpreted: {}", e.toString());
        } catch (Throwable e) {
            com.example.evanscomputermod.EvansComputerMod.LOGGER.warn(
                    "Chicory: compiling a {} KiB module failed, it will be interpreted", size / 1024, e);
        }
    }

    private static String sha256(byte[] bytes) {
        try {
            return java.util.HexFormat.of().formatHex(
                    java.security.MessageDigest.getInstance("SHA-256").digest(bytes));
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
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
            Instance.Builder builder = Instance.builder(wm)
                    .withImportValues(iv)
                    .withStart(false);        // we drive entry points manually
            var factory = handle.machineFactory;
            if (factory != null) builder.withMachineFactory(factory);
            Instance inst = builder.build();
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
        /** Compiled code for this module, once ready; null = interpret. */
        volatile java.util.function.Function<Instance, com.dylibso.chicory.runtime.Machine> machineFactory;
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
