package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmModuleHandle;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.wasm.WasmManager;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Manages WASI child processes spawned by the kernel.
 * Each child runs on its own thread with its own {@link WasmInstance}.
 */
public class ProcessManager {

    private final Map<Integer, ProcessEntry> processes = new ConcurrentHashMap<>();
    private final AtomicInteger nextPid = new AtomicInteger(1);
    private final Path storagePath;
    private final NetIpcBridge netIpcBridge;
    private final ChildHostBridge childBridge;

    /** Maps PID → stdout pipe (for parent to drain in process_wait). */
    private final Map<Integer, WasiPipe> childOutputPipes = new ConcurrentHashMap<>();
    /** Maps PID → stdin pipe (for parent to forward keyboard input). */
    private final Map<Integer, WasiPipe> childInputPipes = new ConcurrentHashMap<>();

    public ProcessManager(Path storagePath, NetIpcBridge netIpcBridge, ChildHostBridge childBridge) {
        this.storagePath = storagePath;
        this.netIpcBridge = netIpcBridge;
        this.childBridge = childBridge;
    }

    /**
     * Spawn a child WASI process.
     */
    public int spawn(Path wasmPath, String[] argv, java.util.Map<String, String> envVars) {
        if (!Files.exists(wasmPath)) {
            EvansComputerMod.LOGGER.warn("WASI spawn: file not found: {}", wasmPath);
            return -1;
        }

        int pid = nextPid.getAndIncrement();

        WasiPipe stdoutPipe = new WasiPipe(16384);
        WasiPipe stdinPipe = new WasiPipe(4096);
        childOutputPipes.put(pid, stdoutPipe);
        childInputPipes.put(pid, stdinPipe);

        FdTable fdTable = new FdTable();
        fdTable.insertAt(0, new PipeFd(stdinPipe, true));
        fdTable.insertAt(1, new PipeFd(stdoutPipe, false));
        fdTable.insertAt(2, new PipeFd(stdoutPipe, false));

        String name = wasmPath.getFileName().toString();
        CountDownLatch exitLatch = new CountDownLatch(1);
        ProcessEntry entry = new ProcessEntry(pid, name, exitLatch);
        processes.put(pid, entry);

        byte[] wasmBytes;
        try {
            wasmBytes = Files.readAllBytes(wasmPath);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("WASI spawn: failed to read {}", wasmPath, e);
            processes.remove(pid);
            childOutputPipes.remove(pid);
            return -1;
        }

        Thread childThread = new Thread(() -> {
            int exitCode = runWasiProcess(wasmBytes, argv, fdTable, pid, envVars);
            entry.exitCode = exitCode;
            entry.state = ProcessState.ZOMBIE;
            stdoutPipe.closeWrite();
            stdinPipe.closeRead();
            exitLatch.countDown();
            EvansComputerMod.LOGGER.debug("WASI process {} ({}) exited with code {}",
                    pid, name, exitCode);
        }, "WASI-PID-" + pid);
        childThread.setDaemon(true);
        entry.thread = childThread;
        childThread.start();

        EvansComputerMod.LOGGER.info("Spawned WASI process {} ({})", pid, name);
        return pid;
    }

    public WasiPipe getChildOutputPipe(int pid) { return childOutputPipes.get(pid); }
    public WasiPipe getChildInputPipe(int pid)  { return childInputPipes.get(pid); }

    public int waitForExit(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return -1;

        try {
            entry.exitLatch.await();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }

        int code = entry.exitCode != null ? entry.exitCode : -1;
        processes.remove(pid);
        childOutputPipes.remove(pid);
        childInputPipes.remove(pid);
        return code;
    }

    public ProcessState getState(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return ProcessState.ZOMBIE;
        return entry.state;
    }

    public int kill(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return -1;
        if (entry.thread != null) {
            entry.thread.interrupt();
        }
        return 0;
    }

    public String listProcesses() {
        StringBuilder sb = new StringBuilder("[");
        boolean first = true;
        for (ProcessEntry entry : processes.values()) {
            if (!first) sb.append(",");
            sb.append(String.format("{\"pid\":%d,\"name\":\"%s\",\"state\":\"%s\"}",
                    entry.pid, entry.name, entry.state));
            first = false;
        }
        sb.append("]");
        return sb.toString();
    }

    // --- Private: run the child WASM process ---

    private int runWasiProcess(byte[] wasmBytes, String[] argv, FdTable fdTable, int pid,
                               java.util.Map<String, String> envVars) {
        WasmRuntime runtime = WasmManager.runtime();
        if (runtime == null) {
            EvansComputerMod.LOGGER.error("WASI PID {}: no WASM runtime bound", pid);
            return 1;
        }

        WasiFunctions.WasiState state = new WasiFunctions.WasiState(fdTable, argv, storagePath, envVars);
        WasmInstance instance = null;

        try {
            WasmModuleHandle module = runtime.compile(wasmBytes);

            // Build the import list. Handlers reference state.instance for
            // memory access; we set state.instance below before any host
            // function can run, so the closure-capture of `state` is safe.
            List<WasmHostFunc> imports = new ArrayList<>();
            WasiFunctions.register(state, imports, childBridge);
            WasiFunctions.registerSocketFunctions(state, imports, netIpcBridge, pid);

            instance = runtime.instantiate(module, imports);
            state.instance = instance;

            if (instance.hasExport("_start")) {
                instance.callExport("_start");
                return 0;
            } else if (instance.hasExport("main")) {
                instance.callExport("main");
                return 0;
            } else {
                EvansComputerMod.LOGGER.warn("WASI PID {}: no _start or main function", pid);
                return 127;
            }
        } catch (WasmTrap e) {
            if (e.kind() == WasmTrap.Kind.EXIT) {
                return e.exitCode();
            }
            if (e.kind() == WasmTrap.Kind.INTERRUPTED) {
                EvansComputerMod.LOGGER.info("WASI PID {} interrupted", pid);
                return 130; // SIGINT-equivalent
            }
            // Old wasmtime path used to bury proc_exit() in arbitrary error
            // chains; now WasmTrap.Kind.EXIT covers it cleanly. Anything
            // else here is a real crash.
            EvansComputerMod.LOGGER.error("WASI PID {} crashed: {}", pid, e.getMessage(), e);
            return 1;
        } catch (Throwable e) {
            EvansComputerMod.LOGGER.error("WASI PID {} crashed", pid, e);
            return 1;
        } finally {
            fdTable.closeAll();
            if (instance != null) {
                try { instance.close(); } catch (Exception ignored) {}
            }
        }
    }

    // --- Inner types ---

    public enum ProcessState { RUNNING, ZOMBIE }

    static class ProcessEntry {
        final int pid;
        final String name;
        volatile ProcessState state = ProcessState.RUNNING;
        volatile Integer exitCode;
        Thread thread;
        final CountDownLatch exitLatch;

        ProcessEntry(int pid, String name, CountDownLatch exitLatch) {
            this.pid = pid;
            this.name = name;
            this.exitLatch = exitLatch;
        }
    }
}
