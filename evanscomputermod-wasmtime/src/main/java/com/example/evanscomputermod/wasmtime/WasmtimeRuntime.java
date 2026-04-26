package com.example.evanscomputermod.wasmtime;

import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmModuleHandle;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.api.wasm.WasmValType;

import io.github.kawamuray.wasmtime.Config;
import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.FuncType;
import io.github.kawamuray.wasmtime.ImportType;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Memory;
import io.github.kawamuray.wasmtime.Module;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.WasmtimeException;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Wasmtime-backed implementation of {@link WasmRuntime}. One shared
 * {@link Engine} (with epoch interruption enabled) services every instance.
 * Each instance gets its own {@link Store}; the store is single-threaded
 * but the engine is safe to share across threads.
 */
final class WasmtimeRuntime implements WasmRuntime {

    private final Engine engine;

    WasmtimeRuntime() {
        Config config = new Config();
        config.epochInterruption(true);
        this.engine = new Engine(config);
    }

    @Override
    public String providerName() {
        return "wasmtime";
    }

    @Override
    public WasmModuleHandle compile(byte[] wasmBytes) throws WasmTrap {
        try {
            Module module = Module.fromBinary(engine, wasmBytes);
            return new WasmtimeModuleHandle(module);
        } catch (WasmtimeException e) {
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, "WASM compile failed: " + e.getMessage(), e);
        }
    }

    @Override
    public WasmInstance instantiate(WasmModuleHandle module, List<WasmHostFunc> imports) throws WasmTrap {
        WasmtimeModuleHandle handle = (WasmtimeModuleHandle) module;
        Store<Void> store = new Store<>(null, engine);
        store.setEpochDeadline(1);

        // Build a name -> WasmHostFunc map (first match wins).
        Map<String, WasmHostFunc> supplied = new HashMap<>();
        for (WasmHostFunc f : imports) {
            supplied.putIfAbsent(f.moduleName() + "\0" + f.fieldName(), f);
        }

        List<Func> ownedFuncs = new ArrayList<>();
        List<Extern> orderedExterns = new ArrayList<>();

        try {
            for (ImportType imp : handle.module.imports()) {
                String key = imp.module() + "\0" + imp.name();
                WasmHostFunc handler = supplied.get(key);
                Func f;
                if (imp.type() == ImportType.Type.FUNC) {
                    FuncType ft = imp.func();
                    if (handler != null) {
                        f = new Func(store, ft, new HostHandlerAdapter(handler.handler(),
                                ft.getResults(), imp.module(), imp.name()));
                    } else {
                        f = new Func(store, ft, new StubHandler(ft.getResults()));
                    }
                    ownedFuncs.add(f);
                    orderedExterns.add(Extern.fromFunc(f));
                } else {
                    // Non-function imports (memory/global/table) — not supported here.
                    throw new WasmTrap(WasmTrap.Kind.LINK_ERROR,
                            "Unsupported import kind for " + imp.module() + "::" + imp.name() + ": " + imp.type());
                }
            }

            Instance inst = new Instance(store, handle.module, orderedExterns);
            Memory mem = inst.getMemory(store, "memory").orElse(null);
            return new WasmtimeInstance(engine, store, inst, mem, ownedFuncs);
        } catch (WasmTrap e) {
            // Clean up before propagating.
            for (Func f : ownedFuncs) {
                try { f.close(); } catch (Throwable ignored) {}
            }
            try { store.close(); } catch (Throwable ignored) {}
            throw e;
        } catch (WasmtimeException e) {
            for (Func f : ownedFuncs) {
                try { f.close(); } catch (Throwable ignored) {}
            }
            try { store.close(); } catch (Throwable ignored) {}
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, e.getMessage(), e);
        }
    }

    @Override
    public void close() {
        try { engine.close(); } catch (Throwable ignored) {}
    }

    // --- helpers ---

    static final class WasmtimeModuleHandle implements WasmModuleHandle {
        final Module module;
        WasmtimeModuleHandle(Module module) { this.module = module; }

        @Override
        public List<ImportDescriptor> imports() {
            List<ImportDescriptor> out = new ArrayList<>();
            for (ImportType imp : module.imports()) {
                ImportDescriptor.Kind kind;
                List<WasmValType> p = Collections.emptyList();
                List<WasmValType> r = Collections.emptyList();
                if (imp.type() == ImportType.Type.FUNC) {
                    kind = ImportDescriptor.Kind.FUNCTION;
                    FuncType ft = imp.func();
                    p = mapTypes(ft.getParams());
                    r = mapTypes(ft.getResults());
                } else {
                    kind = switch (imp.type()) {
                        case MEMORY -> ImportDescriptor.Kind.MEMORY;
                        case GLOBAL -> ImportDescriptor.Kind.GLOBAL;
                        case TABLE -> ImportDescriptor.Kind.TABLE;
                        default -> ImportDescriptor.Kind.OTHER;
                    };
                }
                out.add(new ImportDescriptor(imp.module(), imp.name(), kind, p, r));
            }
            return out;
        }

        @Override
        public List<String> exportedFunctions() {
            // Wasmtime's exports() is per-instance; without an Instance we
            // can't enumerate exports here cheaply. Return empty — call
            // sites use hasExport() on the instance instead.
            return Collections.emptyList();
        }
    }

    private static List<WasmValType> mapTypes(Val.Type[] types) {
        List<WasmValType> out = new ArrayList<>(types.length);
        for (Val.Type t : types) out.add(toApi(t));
        return out;
    }

    static WasmValType toApi(Val.Type t) {
        return switch (t) {
            case I32 -> WasmValType.I32;
            case I64 -> WasmValType.I64;
            case F32 -> WasmValType.F32;
            case F64 -> WasmValType.F64;
            default -> WasmValType.I32;
        };
    }

    /**
     * Bridges the SPI handler signature to wasmtime-java's {@code Func.Handler}.
     *
     * <p>Wasmtime-java hands us a {@code results} array populated with
     * {@code null} entries — the handler is responsible for creating fresh
     * {@code Val} values of the correct type. We capture those types here at
     * construction time from the {@link FuncType} so the call hot path doesn't
     * need to inspect anything wasmtime didn't pre-fill.
     *
     * <p>If the handler throws (including {@link WasmTrap}), wasmtime catches
     * it and converts to a generic trap. We log the original cause first so
     * the underlying error isn't lost in the wasmtime exception chain — the
     * trap message wasmtime surfaces is unhelpful otherwise.
     */
    private static final class HostHandlerAdapter implements Func.Handler {
        private final WasmHostFunc.Handler handler;
        private final Val.Type[] returnTypes;
        private final String moduleName;
        private final String fieldName;

        HostHandlerAdapter(WasmHostFunc.Handler handler, Val.Type[] returnTypes,
                           String moduleName, String fieldName) {
            this.handler = handler;
            this.returnTypes = returnTypes.clone();
            this.moduleName = moduleName;
            this.fieldName = fieldName;
        }

        @Override
        public void call(io.github.kawamuray.wasmtime.Caller caller, Val[] params, Val[] results) {
            long[] args = new long[params.length];
            for (int i = 0; i < params.length; i++) {
                args[i] = toLong(params[i]);
            }
            long[] out;
            try {
                out = handler.invoke(WasmtimeInstance.currentInstance(), args);
            } catch (WasmTrap t) {
                // Surfaces in wasmtime as a generic trap; preserve the kind/message.
                throw t;
            } catch (RuntimeException re) {
                org.slf4j.LoggerFactory.getLogger(HostHandlerAdapter.class)
                        .error("Host function {}::{} threw before wasmtime trap: {}",
                                moduleName, fieldName, re.toString(), re);
                throw re;
            }
            // Always fill in result slots with the declared return type, even
            // if the handler returned null/short — leaving a null in results[]
            // makes wasmtime trap with an opaque "error while executing"
            // message that's very hard to debug.
            for (int i = 0; i < returnTypes.length; i++) {
                long bits = (out != null && i < out.length) ? out[i] : 0L;
                results[i] = fromLong(bits, returnTypes[i]);
            }
        }
    }

    /** Stub for unmatched function imports: returns zero values. */
    private static final class StubHandler implements Func.Handler {
        private final Val.Type[] returns;
        StubHandler(Val.Type[] returns) { this.returns = returns; }
        @Override
        public void call(io.github.kawamuray.wasmtime.Caller caller, Val[] params, Val[] results) {
            for (int i = 0; i < results.length; i++) {
                results[i] = switch (returns[i]) {
                    case I32 -> Val.fromI32(0);
                    case I64 -> Val.fromI64(0);
                    case F32 -> Val.fromF32(0.0f);
                    case F64 -> Val.fromF64(0.0);
                    default -> Val.fromI32(0);
                };
            }
        }
    }

    static long toLong(Val v) {
        return switch (v.getType()) {
            case I32 -> (long) v.i32();
            case I64 -> v.i64();
            case F32 -> Integer.toUnsignedLong(Float.floatToRawIntBits(v.f32()));
            case F64 -> Double.doubleToRawLongBits(v.f64());
            default -> 0L;
        };
    }

    static Val fromLong(long bits, Val.Type type) {
        return switch (type) {
            case I32 -> Val.fromI32((int) bits);
            case I64 -> Val.fromI64(bits);
            case F32 -> Val.fromF32(Float.intBitsToFloat((int) bits));
            case F64 -> Val.fromF64(Double.longBitsToDouble(bits));
            default -> Val.fromI32(0);
        };
    }
}
