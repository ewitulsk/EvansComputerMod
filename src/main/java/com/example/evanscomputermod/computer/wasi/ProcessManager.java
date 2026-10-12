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
 *
 * <p><b>Pause and resume.</b> A child can be paused (no CPU, memory and stack
 * kept) and resumed later: {@link #pause(int)}, {@link #resume(int)},
 * {@link #isPaused(int)}. Each child has a {@link PauseGate}; see it for the
 * safe points per runtime and the clock rules (sleeps and the monotonic clock
 * stop while paused). Every host call is bracketed by the gate, so a paused
 * child neither starts a host call nor returns from one until resumed:
 * <ul>
 *   <li>Sleeps ({@code poll_oneoff} clock subscriptions, {@code sleep_ms})
 *       park in the gate and their deadlines move out by the paused time.</li>
 *   <li>Other blocking calls (socket recv/accept through the kernel, pipe
 *       reads, display vblank waits) keep waiting on their own; if they
 *       complete while the child is paused, the result is held until resume.
 *       Socket timeouts are the kernel's and follow the kernel's clock, which
 *       only stops when the whole computer sleeps.</li>
 *   <li>A kill always wins over a pause: the child unwinds (exit code 130).</li>
 * </ul>
 *
 * <p><b>Groups.</b> Every child has a group id (its own pid by default, or
 * one given at spawn / {@link #setGroup}). {@link #pauseGroup(int)} pauses
 * every member and marks the group paused, so members spawned or moved into
 * it later start paused, until {@link #resumeGroup(int)}. An app's processes
 * share one group and suspend together. Pausing one pid never pauses others.
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

    /** Woken whenever a child writes output or exits (wakes the kernel worker). */
    private volatile Runnable activityListener = () -> {};

    /** Called with a child's pid on its own thread just before it is reported exited. */
    private volatile java.util.function.IntConsumer exitListener = pid -> {};

    /** The pid of the child running on the current thread, or 0 off a child thread. */
    private static final ThreadLocal<Integer> CURRENT_PID = ThreadLocal.withInitial(() -> 0);

    /**
     * The pid of the program whose host function is running on this thread, or
     * 0 (the kernel) when called from any other thread. Host functions use it
     * to tell which program owns a device.
     */
    public static int currentPid() {
        return CURRENT_PID.get();
    }

    public void setExitListener(java.util.function.IntConsumer l) {
        this.exitListener = l != null ? l : pid -> {};
    }

    public void setActivityListener(Runnable r) {
        this.activityListener = r != null ? r : () -> {};
    }

    public ProcessManager(Path storagePath, NetIpcBridge netIpcBridge, ChildHostBridge childBridge) {
        this.storagePath = storagePath;
        this.netIpcBridge = netIpcBridge;
        this.childBridge = childBridge;
    }

    /** Groups currently paused with {@link #pauseGroup(int)}. */
    private final Set<Integer> pausedGroups = ConcurrentHashMap.newKeySet();
    /** Pause reasons applied to every child, including ones spawned later (computer sleep). */
    private volatile int globalPauseReasons;

    /**
     * Spawn a child WASI process in its own group (group id = pid).
     */
    public int spawn(Path wasmPath, String[] argv, java.util.Map<String, String> envVars) {
        return spawn(wasmPath, argv, envVars, 0);
    }

    /**
     * Spawn a child WASI process in process group {@code group} ({@code <= 0}:
     * a new group named after its pid). A child spawned into a paused group,
     * or while the computer sleeps, starts paused.
     */
    public int spawn(Path wasmPath, String[] argv, java.util.Map<String, String> envVars, int group) {
        return spawn(wasmPath, argv, envVars, group, false);
    }

    /**
     * Spawn a child the kernel doesn't manage (the GUI shell, apps the
     * compositor launches): its output goes to a small log ({@link #detachedOutput})
     * instead of a pipe someone must drain, and it is reaped as soon as it exits.
     */
    public int spawnDetached(Path wasmPath, String[] argv, java.util.Map<String, String> envVars) {
        return spawn(wasmPath, argv, envVars, 0, true);
    }

    private int spawn(Path wasmPath, String[] argv, java.util.Map<String, String> envVars, int group, boolean detached) {
        if (!Files.exists(wasmPath)) {
            EvansComputerMod.LOGGER.warn("WASI spawn: file not found: {}", wasmPath);
            return -1;
        }

        int pid = nextPid.getAndIncrement();

        WasiPipe stdoutPipe = new WasiPipe(16384);
        WasiPipe stdinPipe = new WasiPipe(4096);
        stdoutPipe.setOnActivity(() -> activityListener.run());
        childOutputPipes.put(pid, stdoutPipe);
        childInputPipes.put(pid, stdinPipe);

        FdTable fdTable = new FdTable();
        fdTable.insertAt(0, new PipeFd(stdinPipe, true));
        String name = wasmPath.getFileName().toString();
        CountDownLatch exitLatch = new CountDownLatch(1);
        ProcessEntry entry = new ProcessEntry(pid, name, exitLatch);
        entry.detached = detached;
        if (detached) {
            fdTable.insertAt(1, entry.log);
            fdTable.insertAt(2, entry.log);
        } else {
            fdTable.insertAt(1, new PipeFd(stdoutPipe, false));
            fdTable.insertAt(2, new PipeFd(stdoutPipe, false));
        }
        entry.group = group > 0 ? group : pid;
        if (pausedGroups.contains(entry.group)) entry.gate.pause(PauseGate.REASON_PROCESS);
        int global = globalPauseReasons;
        if (global != 0) entry.gate.pause(global);
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

        Thread childThread = new PausableThread(() -> {
            CURRENT_PID.set(pid);
            int exitCode = runWasiProcess(wasmBytes, argv, fdTable, pid, envVars, entry.gate);
            try {
                exitListener.accept(pid);
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.warn("WASI process {} exit cleanup failed", pid, t);
            }
            entry.exitCode = exitCode;
            entry.state = ProcessState.ZOMBIE;
            // Requests the dead child left queued can never be answered.
            netIpcBridge.cancelPending(pid);
            stdoutPipe.closeWrite();
            stdinPipe.closeRead();
            exitLatch.countDown();
            if (entry.detached) {
                entry.exitReported = true;
                reap(pid);
            }
            activityListener.run();
            EvansComputerMod.LOGGER.debug("WASI process {} ({}) exited with code {}",
                    pid, name, exitCode);
        }, "WASI-PID-" + pid, entry.gate);
        childThread.setDaemon(true);
        entry.thread = childThread;
        childThread.start();

        EvansComputerMod.LOGGER.info("Spawned WASI process {} ({})", pid, name);
        return pid;
    }

    public WasiPipe getChildOutputPipe(int pid) { return childOutputPipes.get(pid); }

    /** The program file name of a live child ({@code gba.wasm}), or null. */
    public String nameOf(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry == null ? null : entry.name;
    }

    /** The last few KiB a detached child printed, or null for an unknown or non-detached pid. */
    public String detachedOutput(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry == null || !entry.detached ? null : entry.log.text();
    }
    public WasiPipe getChildInputPipe(int pid)  { return childInputPipes.get(pid); }

    // --- Non-blocking accessors for the kernel (event-driven job control) ---

    /**
     * Exit status without blocking.
     * @return the exit code if the process has exited, {@code null} if it is
     *         still running; {@code Integer.MIN_VALUE} if the pid is unknown.
     */
    public Integer tryWait(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return Integer.MIN_VALUE;
        if (entry.state != ProcessState.ZOMBIE) return null;
        entry.exitReported = true;
        return entry.exitCode != null ? entry.exitCode : -1;
    }

    /**
     * Non-blocking read of the child's terminal output. Once the exit has
     * been reported and the pipe is drained, the process entry is reaped.
     * @return bytes read, 0 if none available, -1 if the pid is unknown
     */
    public int readOutput(int pid, byte[] buf) {
        WasiPipe pipe = childOutputPipes.get(pid);
        if (pipe == null) return -1;
        int n = pipe.tryRead(buf);
        if (n == 0) {
            ProcessEntry entry = processes.get(pid);
            if (entry != null && entry.exitReported && !pipe.hasData()) {
                reap(pid);
            }
        }
        return n;
    }

    /** Non-blocking write to the child's stdin. Returns bytes accepted or -1. */
    public int writeInput(int pid, byte[] data) {
        WasiPipe pipe = childInputPipes.get(pid);
        if (pipe == null) return -1;
        return pipe.tryWrite(data, 0, data.length);
    }

    private void reap(int pid) {
        processes.remove(pid);
        childOutputPipes.remove(pid);
        childInputPipes.remove(pid);
    }

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
        stop(entry);
        return 0;
    }

    /** How long a killed program may keep running before its wasm code is trapped. */
    private static final long KILL_GRACE_MS = 300;

    private static final java.util.concurrent.ScheduledExecutorService KILL_WATCHDOG =
            java.util.concurrent.Executors.newSingleThreadScheduledExecutor(r -> {
                Thread t = new Thread(r, "ECM-Kill-Watchdog");
                t.setDaemon(true);
                return t;
            });

    /**
     * Stop a program. Interrupting its thread ends any blocking host call
     * (sleep, socket, vblank wait, ...). A program spinning in pure wasm code
     * makes no host calls, so if it is still alive after a short grace period
     * its instance is interrupted too (the runtime traps the running code).
     */
    private void stop(ProcessEntry entry) {
        Thread t = entry.thread;
        if (t == null || !t.isAlive()) return;
        t.interrupt();
        KILL_WATCHDOG.schedule(() -> {
            WasmInstance inst = entry.instance;
            if (t.isAlive() && inst != null) {
                EvansComputerMod.LOGGER.info("WASI process {} still running {} ms after kill; trapping it",
                        entry.pid, KILL_GRACE_MS);
                try {
                    inst.requestInterrupt();
                } catch (Throwable e) {
                    EvansComputerMod.LOGGER.debug("requestInterrupt failed for pid {}", entry.pid, e);
                }
            }
        }, KILL_GRACE_MS, java.util.concurrent.TimeUnit.MILLISECONDS);
    }

    /**
     * Interrupt every running child process. Their blocking host functions
     * (e.g. NetIpcBridge.callBlocking, poll_oneoff sleeps) will throw
     * InterruptedException and unwind; their Rust main loops then observe
     * the resulting errors and return naturally. Called from
     * {@code ComputerInstance.interrupt()} on Ctrl+T so killed children
     * release their kernel-side IPC sessions instead of sitting blocked
     * for the full 30-second socket timeout.
     */
    public void killAll() {
        for (ProcessEntry entry : processes.values()) {
            stop(entry);
        }
    }

    // --- Pause / resume ---

    /**
     * Pause a child: it stops at its next safe point (see {@link PauseGate})
     * and keeps its memory. Non-blocking; {@link #awaitHalted} waits for the
     * stop. Idempotent. Returns 0, or -1 for an unknown or exited pid.
     */
    public int pause(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null || entry.state != ProcessState.RUNNING) return -1;
        entry.gate.pause(PauseGate.REASON_PROCESS);
        return 0;
    }

    /**
     * Resume a child paused with {@link #pause(int)} or {@link #pauseGroup}.
     * It stays paused while its computer sleeps. Returns 0, or -1 if unknown.
     */
    public int resume(int pid) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return -1;
        entry.gate.resume(PauseGate.REASON_PROCESS);
        return 0;
    }

    /** True if the child has any pause reason (its own, its group's, or computer sleep). */
    public boolean isPaused(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry != null && entry.gate.isPaused();
    }

    /**
     * Wait until a paused child has stopped executing guest code (parked, or
     * held inside a host call). Returns false on timeout, or if the pid is
     * unknown, not paused, or exited. A host call in progress may still
     * finish its own work (e.g. complete a write); the guest just won't see
     * its result until resume.
     */
    public boolean awaitHalted(int pid, long timeoutMs) throws InterruptedException {
        ProcessEntry entry = processes.get(pid);
        if (entry == null) return false;
        long deadline = System.nanoTime() + timeoutMs * 1_000_000L;
        while (true) {
            if (entry.state != ProcessState.RUNNING || !entry.gate.isPaused()) return false;
            if (entry.gate.isHalted()) return true;
            if (System.nanoTime() >= deadline) return false;
            Thread.sleep(1);
        }
    }

    /** Total wall time this child has spent paused, in nanoseconds (0 if unknown). */
    public long pausedNanos(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry == null ? 0 : entry.gate.pausedNanos();
    }

    /** The child's running instance, for tests (null if not started or unknown). */
    WasmInstance instanceOf(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry == null ? null : entry.instance;
    }

    /** The child's process group, or -1 if unknown. */
    public int getGroup(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry == null ? -1 : entry.group;
    }

    /**
     * Move a child into a group. Joining a paused group pauses it; leaving a
     * paused group does not resume it (use {@link #resume(int)}).
     */
    public int setGroup(int pid, int group) {
        ProcessEntry entry = processes.get(pid);
        if (entry == null || group <= 0) return -1;
        entry.group = group;
        if (pausedGroups.contains(group)) entry.gate.pause(PauseGate.REASON_PROCESS);
        return 0;
    }

    /** Pause every member of a group, now and later. Returns how many live members were paused. */
    public int pauseGroup(int group) {
        pausedGroups.add(group);
        int n = 0;
        for (ProcessEntry entry : processes.values()) {
            if (entry.group == group && entry.state == ProcessState.RUNNING) {
                entry.gate.pause(PauseGate.REASON_PROCESS);
                n++;
            }
        }
        return n;
    }

    /** Resume every member of a group and unmark it. Returns how many members were resumed. */
    public int resumeGroup(int group) {
        pausedGroups.remove(group);
        int n = 0;
        for (ProcessEntry entry : processes.values()) {
            if (entry.group == group) {
                entry.gate.resume(PauseGate.REASON_PROCESS);
                n++;
            }
        }
        return n;
    }

    public boolean isGroupPaused(int group) {
        return pausedGroups.contains(group);
    }

    /**
     * Add a pause reason to every child, including children spawned until
     * {@link #resumeAll(int)} (whole-computer sleep uses
     * {@link PauseGate#REASON_COMPUTER}).
     */
    public void pauseAll(int reason) {
        globalPauseReasons |= reason;
        for (ProcessEntry entry : processes.values()) {
            entry.gate.pause(reason);
        }
    }

    /** Clear a reason set by {@link #pauseAll(int)}; per-process pauses stay. */
    public void resumeAll(int reason) {
        globalPauseReasons &= ~reason;
        for (ProcessEntry entry : processes.values()) {
            entry.gate.resume(reason);
        }
    }

    /** True if every live child has stopped executing guest code (vacuously true with none). */
    public boolean allHalted() {
        for (ProcessEntry entry : processes.values()) {
            Thread t = entry.thread;
            if (entry.state == ProcessState.RUNNING && t != null && t.isAlive() && !entry.gate.isHalted()) {
                return false;
            }
        }
        return true;
    }

    /**
     * Snapshot of currently-known PIDs (live or zombie). The returned list
     * is a defensive copy and safe to iterate without holding any locks.
     * Used by the worker thread to reap dead sessions on its main loop.
     */
    public java.util.List<Integer> snapshotPids() {
        return new java.util.ArrayList<>(processes.keySet());
    }

    /**
     * True if a process for {@code pid} exists and its worker thread is
     * still alive. False for unknown PIDs or after the thread has exited.
     */
    public boolean isAlive(int pid) {
        ProcessEntry entry = processes.get(pid);
        return entry != null && entry.thread != null && entry.thread.isAlive();
    }

    /**
     * List all processes as JSON-like string.
     */
    public String listProcesses() {
        StringBuilder sb = new StringBuilder("[");
        boolean first = true;
        for (ProcessEntry entry : processes.values()) {
            if (!first) sb.append(",");
            String st = entry.state == ProcessState.RUNNING && entry.gate.isPaused()
                    ? "PAUSED" : entry.state.toString();
            sb.append(String.format("{\"pid\":%d,\"name\":\"%s\",\"state\":\"%s\"}",
                    entry.pid, entry.name, st));
            first = false;
        }
        sb.append("]");
        return sb.toString();
    }

    // --- Private: run the child WASM process ---

    private int runWasiProcess(byte[] wasmBytes, String[] argv, FdTable fdTable, int pid,
                               java.util.Map<String, String> envVars, PauseGate gate) {
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
            // Every host call is a pause point: a paused child neither starts
            // one nor returns from one into guest code until resumed.
            imports.replaceAll(gate::wrap);

            instance = runtime.instantiate(module, imports);
            state.instance = instance;
            ProcessEntry self = processes.get(pid);
            if (self != null) self.instance = instance;

            String entryPoint = instance.hasExport("_start") ? "_start"
                    : instance.hasExport("main") ? "main" : null;
            if (entryPoint != null) {
                gate.checkpoint(); // spawned paused: don't run a single instruction
                gate.setWasmActive(true);
                try {
                    instance.callExport(entryPoint);
                } finally {
                    gate.setWasmActive(false);
                }
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
                Thread.interrupted(); // clear the flag so the finally block runs cleanly
                return 130; // SIGINT-equivalent
            }
            // Old wasmtime path used to bury proc_exit() in arbitrary error
            // chains; now WasmTrap.Kind.EXIT covers it cleanly. Anything
            // else here is a real crash.
            EvansComputerMod.LOGGER.error("WASI PID {} crashed: {}", pid, e.getMessage(), e);
            return 1;
        } catch (Throwable e) {
            // NetIpcBridge.callBlocking and poll_oneoff throw a plain
            // RuntimeException("... interrupted") when Ctrl+T fires
            // processManager.killAll(). A runtime that doesn't classify that
            // as WasmTrap.Kind.INTERRUPTED lands here; treat it as a clean
            // shutdown (exit code 130 = 128 + SIGINT), not a crash.
            String msg = e.getMessage() == null ? "" : e.getMessage();
            if (msg.contains("interrupted") || Thread.currentThread().isInterrupted()) {
                EvansComputerMod.LOGGER.debug("WASI PID {} interrupted, exiting", pid);
                Thread.interrupted(); // clear the flag so the finally block runs cleanly
                return 130;
            }
            EvansComputerMod.LOGGER.error("WASI PID {} crashed", pid, e);
            return 1;
        } finally {
            fdTable.closeAll();
            if (state.peripheralEvents != null && childBridge != null) {
                childBridge.peripheralUnsubscribe(state.peripheralEvents);
            }
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
        /** Set once the kernel has observed the exit (tryWait). */
        volatile boolean exitReported;
        Thread thread;
        /** The running instance, for trapping a program that ignores its thread interrupt. */
        volatile WasmInstance instance;
        final CountDownLatch exitLatch;
        /** Pause switch of this child's thread. */
        final PauseGate gate = new PauseGate();
        /** Process group; {@link #pauseGroup(int)} acts on all members. */
        volatile int group;
        /** Started by the host, not the kernel (see {@link #spawnDetached}). */
        volatile boolean detached;
        /** Output of a detached child. */
        final TailFd log = new TailFd();

        ProcessEntry(int pid, String name, CountDownLatch exitLatch) {
            this.pid = pid;
            this.name = name;
            this.exitLatch = exitLatch;
        }
    }
}
