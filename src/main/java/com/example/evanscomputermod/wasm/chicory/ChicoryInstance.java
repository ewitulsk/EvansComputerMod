package com.example.evanscomputermod.wasm.chicory;

import com.example.evanscomputermod.api.wasm.WasmExport;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.example.evanscomputermod.api.wasm.WasmTrap;

import com.dylibso.chicory.runtime.ChicoryInterruptedException;
import com.dylibso.chicory.runtime.ExportFunction;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.runtime.Memory;
import com.dylibso.chicory.runtime.TrapException;
import com.dylibso.chicory.runtime.WasmRuntimeException;
import com.dylibso.chicory.wasm.ChicoryException;

import java.util.concurrent.atomic.AtomicReference;

/**
 * Chicory-backed {@link WasmInstance}.
 *
 * <p>Cancellation strategy: Chicory's {@code InterpreterMachine} polls
 * {@link Thread#isInterrupted()} at every call and at backwards branches.
 * {@link #requestInterrupt()} both flips an internal flag (so host functions
 * can observe it) and calls {@link Thread#interrupt()} on the thread that's
 * currently inside {@code callExport}. After the call unwinds we clear the
 * thread interrupt status before returning, so the caller's thread state
 * isn't polluted.
 */
final class ChicoryInstance implements WasmInstance {

    private Instance inst;
    private ChicoryMemory memory;

    /** Thread currently inside a callExport / WasmExport.call. Set on entry, cleared on exit. */
    private final AtomicReference<Thread> currentThread = new AtomicReference<>();
    private volatile boolean interruptRequested;

    /** Two-phase init: Runtime constructs the shell, then attaches the Instance once built. */
    void attach(Instance inst) {
        this.inst = inst;
        Memory m = inst.memory();
        this.memory = (m == null) ? null : new ChicoryMemory(m);
    }

    @Override
    public boolean hasExport(String name) {
        try {
            inst.export(name);
            return true;
        } catch (Throwable t) {
            return false;
        }
    }

    @Override
    public long[] callExport(String name, long... args) throws WasmTrap {
        ExportFunction fn;
        try {
            fn = inst.export(name);
        } catch (Throwable t) {
            throw new WasmTrap(WasmTrap.Kind.LINK_ERROR, "missing export: " + name, t);
        }
        return invoke(fn, args);
    }

    @Override
    public WasmExport export(String name) {
        ExportFunction fn;
        try {
            fn = inst.export(name);
        } catch (Throwable t) {
            return null;
        }
        return args -> invoke(fn, args);
    }

    @Override
    public WasmMemory memory() {
        return memory;
    }

    @Override
    public void requestInterrupt() {
        interruptRequested = true;
        Thread t = currentThread.get();
        if (t != null) t.interrupt();
    }

    @Override
    public void clearInterrupt() {
        interruptRequested = false;
        // Clear the JVM thread's interrupt flag if it's the current thread.
        // (Thread.interrupted() is the only way to clear it, and only the
        // owning thread can do it for itself.) Callers usually do this in
        // a finally after catching INTERRUPTED on the worker thread.
        Thread.interrupted();
    }

    @Override
    public void close() {
        // Chicory instances don't hold OS handles; nothing to release.
        currentThread.set(null);
    }

    // --- internal ---

    private long[] invoke(ExportFunction fn, long[] args) throws WasmTrap {
        Thread me = Thread.currentThread();
        Thread prev = currentThread.getAndSet(me);
        try {
            long[] result = fn.apply(args);
            return result == null ? new long[0] : result;
        } catch (ChicoryRuntime.WasmTrapBridge e) {
            // Already-classified trap came up from a host function (e.g. proc_exit).
            throw e.trap;
        } catch (ChicoryInterruptedException e) {
            throw new WasmTrap(WasmTrap.Kind.INTERRUPTED, "interrupted", e);
        } catch (TrapException e) {
            String msg = e.getMessage() == null ? "" : e.getMessage();
            if (msg.contains("out of memory") || msg.contains("memory access out of bounds")) {
                throw new WasmTrap(WasmTrap.Kind.OOM, msg, e);
            }
            throw new WasmTrap(WasmTrap.Kind.EXEC_ERROR, msg, e);
        } catch (WasmRuntimeException e) {
            throw new WasmTrap(WasmTrap.Kind.EXEC_ERROR, e.getMessage(), e);
        } catch (ChicoryException e) {
            // Last-ditch: includes UninstantiableException, InvalidException, etc.
            throw new WasmTrap(WasmTrap.Kind.EXEC_ERROR, e.getMessage(), e);
        } finally {
            currentThread.compareAndSet(me, prev);
            // If we set the interrupt for a cancellation that ran to completion
            // anyway (e.g. the WASM finished before checkInterrupted fired), we
            // still owe the caller a clean thread state.
            if (interruptRequested) {
                Thread.interrupted();
            }
        }
    }
}
