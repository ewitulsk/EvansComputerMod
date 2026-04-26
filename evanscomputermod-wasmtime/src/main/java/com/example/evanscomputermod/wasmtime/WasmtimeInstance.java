package com.example.evanscomputermod.wasmtime;

import com.example.evanscomputermod.api.wasm.WasmExport;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.example.evanscomputermod.api.wasm.WasmTrap;

import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Memory;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.WasmtimeException;

import java.util.List;
import java.util.Optional;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Wasmtime-backed {@link WasmInstance}. Pinned to a single owning thread —
 * the wasmtime store is not thread-safe and re-entry into the store from
 * another thread will deadlock or corrupt state.
 *
 * <p>Cancellation: the engine is configured with epoch interruption; calling
 * {@link #requestInterrupt()} bumps the engine's epoch which causes any
 * running WASM call to trap with an {@code epoch-deadline-exceeded} error.
 * That error is caught and re-thrown as
 * {@link WasmTrap.Kind#INTERRUPTED}.
 */
final class WasmtimeInstance implements WasmInstance {

    private final Engine engine;
    private final Store<Void> store;
    private final Instance inst;
    private final Memory memory;
    private final WasmtimeMemory memoryWrapper;
    private final List<Func> ownedFuncs;

    /**
     * Stash a reference to the active instance so host-function adapters
     * (which only have a {@code Caller} from wasmtime) can pass a real
     * {@link WasmInstance} into the SPI handler. Set per-thread inside
     * {@link #invoke}.
     */
    private static final ThreadLocal<WasmtimeInstance> CURRENT = new ThreadLocal<>();

    static WasmtimeInstance currentInstance() {
        return CURRENT.get();
    }

    private final AtomicReference<Thread> currentThread = new AtomicReference<>();
    private volatile boolean interruptRequested;

    WasmtimeInstance(Engine engine, Store<Void> store, Instance inst,
                     Memory memory, List<Func> ownedFuncs) {
        this.engine = engine;
        this.store = store;
        this.inst = inst;
        this.memory = memory;
        this.memoryWrapper = (memory == null) ? null : new WasmtimeMemory(store, memory);
        this.ownedFuncs = ownedFuncs;
    }

    @Override
    public boolean hasExport(String name) {
        return inst.getFunc(store, name).isPresent();
    }

    @Override
    public long[] callExport(String name, long... args) throws WasmTrap {
        Optional<Func> f = inst.getFunc(store, name);
        if (f.isEmpty()) {
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, "missing export: " + name);
        }
        return invoke(f.get(), null, args);
    }

    @Override
    public WasmExport export(String name) {
        Optional<Func> f = inst.getFunc(store, name);
        if (f.isEmpty()) return null;
        Func fn = f.get();
        // Wasmtime's Func has no signature info accessible from outside, so we
        // can't pre-determine result arity; defer to the call site.
        return args -> invoke(fn, null, args);
    }

    @Override
    public WasmMemory memory() {
        return memoryWrapper;
    }

    @Override
    public void requestInterrupt() {
        interruptRequested = true;
        try {
            engine.incrementEpoch();
        } catch (Throwable t) {
            // best-effort
        }
        Thread t = currentThread.get();
        if (t != null) t.interrupt();
    }

    @Override
    public void clearInterrupt() {
        interruptRequested = false;
        store.setEpochDeadline(1);  // re-arm
        Thread.interrupted();
    }

    @Override
    public void close() {
        for (Func f : ownedFuncs) {
            try { f.close(); } catch (Throwable ignored) {}
        }
        ownedFuncs.clear();
        try { inst.close(); } catch (Throwable ignored) {}
        try { store.close(); } catch (Throwable ignored) {}
    }

    // --- internal ---

    private long[] invoke(Func fn, Val.Type[] returnHint, long[] args) throws WasmTrap {
        Thread me = Thread.currentThread();
        Thread prev = currentThread.getAndSet(me);
        WasmtimeInstance prevInstance = CURRENT.get();
        CURRENT.set(this);
        try {
            // Convert args to Val[]. We don't know parameter types at this layer,
            // so we send raw i64 — wasmtime-java will coerce based on signature.
            // For correctness, callers must encode per WasmHostFunc rules.
            Val[] params = new Val[args.length];
            for (int i = 0; i < args.length; i++) {
                // Default to i64; if the function actually takes i32/f32/f64,
                // wasmtime-java throws a type mismatch. Most call sites in the
                // mod use i32, so we down-cast: if the value fits in 32 bits,
                // emit i32; otherwise i64.
                long v = args[i];
                if (((int) v) == v) {
                    params[i] = Val.fromI32((int) v);
                } else {
                    params[i] = Val.fromI64(v);
                }
            }

            Val[] results = fn.call(store, params);
            long[] out = new long[results.length];
            for (int i = 0; i < results.length; i++) {
                out[i] = WasmtimeRuntime.toLong(results[i]);
            }
            return out;
        } catch (WasmTrap e) {
            throw e; // already classified by a host function
        } catch (WasmtimeException e) {
            String msg = e.getMessage() == null ? "" : e.getMessage();
            if (interruptRequested
                    || msg.contains("epoch-deadline-exceeded")
                    || msg.contains("interrupted")
                    || msg.contains("interrupt")) {
                throw new WasmTrap(WasmTrap.Kind.INTERRUPTED, msg, e);
            }
            if (msg.contains("out of memory") || msg.contains("memory access out of bounds")) {
                throw new WasmTrap(WasmTrap.Kind.OOM, msg, e);
            }
            throw new WasmTrap(WasmTrap.Kind.EXEC_ERROR, msg, e);
        } finally {
            CURRENT.set(prevInstance);
            currentThread.compareAndSet(me, prev);
            if (interruptRequested) {
                Thread.interrupted();
            }
        }
    }
}
