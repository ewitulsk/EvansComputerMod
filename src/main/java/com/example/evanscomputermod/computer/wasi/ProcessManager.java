package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import io.github.kawamuray.wasmtime.*;
import io.github.kawamuray.wasmtime.Val.Type;

import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Manages WASI child processes spawned by the kernel.
 * Each child runs on its own thread with its own Wasmtime Store/Instance.
 */
public class ProcessManager {

    private final Map<Integer, ProcessEntry> processes = new ConcurrentHashMap<>();
    private final AtomicInteger nextPid = new AtomicInteger(1);
    private final Path storagePath;

    /** Maps PID → stdout pipe read end (for parent to drain in process_wait). */
    private final Map<Integer, WasiPipe> childOutputPipes = new ConcurrentHashMap<>();

    public ProcessManager(Path storagePath) {
        this.storagePath = storagePath;
    }

    /**
     * Spawn a child WASI process.
     *
     * @param wasmPath path to the .wasm file (resolved by caller through mount table)
     * @param argv     argument strings (argv[0] = program name)
     * @return PID on success, -1 on failure
     */
    public int spawn(Path wasmPath, String[] argv) {
        if (!Files.exists(wasmPath)) {
            EvansComputerMod.LOGGER.warn("WASI spawn: file not found: {}", wasmPath);
            return -1;
        }

        int pid = nextPid.getAndIncrement();

        // Create pipe: child writes to it, parent reads from it
        WasiPipe pipe = new WasiPipe(16384);
        childOutputPipes.put(pid, pipe);

        // Create child FD table with stdio
        FdTable fdTable = new FdTable();
        fdTable.insertAt(0, new NullFd());                          // stdin (no input for now)
        fdTable.insertAt(1, new PipeFd(pipe, false));              // stdout → pipe write end
        fdTable.insertAt(2, new PipeFd(pipe, false));              // stderr → same pipe (merged)
        // FD 3 = preopened root directory (handled by fd_prestat_get)

        String name = wasmPath.getFileName().toString();
        CountDownLatch exitLatch = new CountDownLatch(1);
        ProcessEntry entry = new ProcessEntry(pid, name, exitLatch);
        processes.put(pid, entry);

        // Read WASM bytes
        byte[] wasmBytes;
        try {
            wasmBytes = Files.readAllBytes(wasmPath);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("WASI spawn: failed to read {}", wasmPath, e);
            processes.remove(pid);
            childOutputPipes.remove(pid);
            return -1;
        }

        // Spawn child thread
        Thread childThread = new Thread(() -> {
            int exitCode = runWasiProcess(wasmBytes, argv, fdTable, pid);
            entry.exitCode = exitCode;
            entry.state = ProcessState.ZOMBIE;
            pipe.closeWrite(); // signal EOF to parent
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

    /**
     * Get the stdout pipe for a child process (for draining in process_wait).
     */
    public WasiPipe getChildPipe(int pid) {
        return childOutputPipes.get(pid);
    }

    /**
     * Wait for a process to exit. Blocks until the process terminates.
     * @return exit code, or -1 if PID not found
     */
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
        // Reap: remove from process table
        processes.remove(pid);
        childOutputPipes.remove(pid);
        return code;
    }

    /**
     * Get the state of a process.
     */
    public ProcessState getState(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return ProcessState.ZOMBIE;
        return entry.state;
    }

    /**
     * Kill a process.
     */
    public int kill(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return -1;
        if (entry.thread != null) {
            entry.thread.interrupt();
        }
        return 0;
    }

    /**
     * List all processes as JSON-like string.
     */
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

    private int runWasiProcess(byte[] wasmBytes, String[] argv, FdTable fdTable, int pid) {
        Engine engine = null;
        Store<WasiFunctions.WasiState> store = null;
        try {
            engine = new Engine();
            WasiFunctions.WasiState state = new WasiFunctions.WasiState(fdTable, argv, storagePath);
            store = new Store<>(state, engine);

            // Load the module
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromBinary(engine, wasmBytes);

            // Register WASI host functions
            List<Func> funcs = new ArrayList<>();
            Map<String, Extern> funcMap = new HashMap<>();
            WasiFunctions.register(store, funcs, funcMap);

            // Match module imports to our functions
            var moduleImports = module.imports();
            List<Extern> imports = new ArrayList<>();

            for (var importType : moduleImports) {
                String importModule = importType.module();
                String importName = importType.name();
                String qualifiedName = importModule + "::" + importName;

                Extern ext = funcMap.get(qualifiedName);
                if (ext == null) ext = funcMap.get(importName);

                if (ext != null) {
                    imports.add(ext);
                } else {
                    // Create a no-op stub for unknown imports
                    if (importType.type() == io.github.kawamuray.wasmtime.ImportType.Type.FUNC) {
                        FuncType ft = importType.func();
                        Func stub = new Func(store, ft, (caller, params, results) -> {
                            for (int i = 0; i < results.length; i++) {
                                results[i] = switch (ft.getResults()[i]) {
                                    case I32 -> Val.fromI32(0);
                                    case I64 -> Val.fromI64(0);
                                    case F32 -> Val.fromF32(0.0f);
                                    case F64 -> Val.fromF64(0.0);
                                    default -> Val.fromI32(0);
                                };
                            }
                        });
                        funcs.add(stub);
                        imports.add(Extern.fromFunc(stub));
                        EvansComputerMod.LOGGER.debug("WASI PID {}: stubbed import {}::{}", pid, importModule, importName);
                    } else {
                        EvansComputerMod.LOGGER.warn("WASI PID {}: cannot stub non-func import {}::{}",
                                pid, importModule, importName);
                        return 1;
                    }
                }
            }

            // Instantiate
            Instance instance = new Instance(store, module, imports);

            // Save memory reference for WASI functions to use
            Memory memory = instance.getMemory(store, "memory").orElse(null);
            store.data().memory = memory;

            // Call entry point
            Optional<Func> startFunc = instance.getFunc(store, "_start");
            if (startFunc.isEmpty()) startFunc = instance.getFunc(store, "main");

            if (startFunc.isPresent()) {
                startFunc.get().call(store);
                return 0;
            } else {
                EvansComputerMod.LOGGER.warn("WASI PID {}: no _start or main function", pid);
                return 127;
            }

        } catch (WasiFunctions.WasiExitException e) {
            return e.exitCode;
        } catch (Throwable e) {
            String msg = e.getMessage() != null ? e.getMessage() : "";
            // Check for proc_exit in the error chain
            if (msg.contains("proc_exit")) {
                // Extract exit code from message like "proc_exit(0)"
                try {
                    int start = msg.indexOf("proc_exit(") + 10;
                    int end = msg.indexOf(")", start);
                    return Integer.parseInt(msg.substring(start, end));
                } catch (Exception ignored) {}
                return 0;
            }
            EvansComputerMod.LOGGER.error("WASI PID {} crashed", pid, e);
            return 1;
        } finally {
            fdTable.closeAll();
            if (store != null) try { store.close(); } catch (Exception ignored) {}
            if (engine != null) try { engine.close(); } catch (Exception ignored) {}
        }
    }

    // --- Inner types ---

    public enum ProcessState {
        RUNNING, ZOMBIE
    }

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
