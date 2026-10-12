package com.example.evanscomputermod.computer.wasi;

import java.util.concurrent.locks.LockSupport;

/**
 * Pause switch for one execution thread: a WASI child ({@link PausableThread})
 * or a computer's kernel worker.
 *
 * <p>Other threads request a pause with {@link #pause(int)}; the owning thread
 * stops the next time it reaches a <em>safe point</em> and stays parked, with
 * its memory and stack intact, until every pause reason is cleared with
 * {@link #resume(int)}. Safe points are:
 * <ul>
 *   <li>entry to and return from every host call of a WASI child (both
 *       runtimes; see {@code ProcessManager});</li>
 *   <li>loop back-edges and calls inside wasm code, on Chicory only
 *       ({@link PausableThread} hooks the runtime's interruption poll);</li>
 *   <li>the waits inside pause-aware blocking host calls
 *       ({@link #sleepNanos}), so a sleeping program parks too;</li>
 *   <li>the top of a kernel worker's loop.</li>
 * </ul>
 *
 * <p><b>Time.</b> The gate accumulates how long it has been paused
 * ({@link #pausedNanos()}). A child's WASI monotonic clock and its sleeps are
 * measured in <em>unpaused</em> time (like Linux's {@code CLOCK_MONOTONIC},
 * which stops during suspend): a 5 s sleep that is paused for 10 s after 1 s
 * wakes 4 s after the resume. The realtime clock keeps following the wall.
 *
 * <p>Pause reasons are bits, so a process paused on its own
 * ({@link #REASON_PROCESS}) stays paused when its computer wakes from sleep
 * ({@link #REASON_COMPUTER}), and the other way round.
 *
 * <p>A thread interrupt (kill) always wins: a parked thread returns from its
 * park with the interrupt flag still set and unwinds.
 */
public final class PauseGate {

    /** {@code ProcessManager.pause(pid)} / {@code pauseGroup}. */
    public static final int REASON_PROCESS = 1;
    /** Whole-computer sleep ({@code ComputerInstance.sleep()}). */
    public static final int REASON_COMPUTER = 2;

    private final Object lock = new Object();
    /** Non-zero while a pause is requested. Written under {@link #lock}. */
    private volatile int reasons;
    /** Wall-nanos of finished pauses. Guarded by {@link #lock}. */
    private long pausedNanosDone;
    /** {@link System#nanoTime()} when the current pause began. Guarded by {@link #lock}. */
    private long pauseStartNanos;

    private volatile Thread owner;
    /** True while the owner sits in {@link #awaitResume()}. */
    private volatile boolean parked;
    /** Nesting depth of host calls on the owner (written by the owner only). */
    private volatile int hostDepth;
    /** True while the owner runs guest code that may be parked at a back-edge. */
    private volatile boolean wasmActive;

    /** The thread this gate stops. Unparked on pause and resume. */
    public void bind(Thread thread) {
        this.owner = thread;
    }

    /**
     * Add a pause reason. Returns true if the gate went from running to paused.
     * Non-blocking: use {@link #isHalted()} to learn when the owner has stopped.
     */
    public boolean pause(int reason) {
        boolean started;
        synchronized (lock) {
            int before = reasons;
            started = before == 0;
            if (started) pauseStartNanos = System.nanoTime();
            reasons = before | reason;
        }
        // A pause-aware sleep parks in parkNanos: wake it so it notices.
        Thread t = owner;
        if (started && t != null) LockSupport.unpark(t);
        return started;
    }

    /** Clear a pause reason. Returns true if the gate went from paused to running. */
    public boolean resume(int reason) {
        boolean ended;
        synchronized (lock) {
            int before = reasons;
            int after = before & ~reason;
            ended = before != 0 && after == 0;
            if (ended) {
                pausedNanosDone += System.nanoTime() - pauseStartNanos;
                lock.notifyAll();
            }
            reasons = after;
        }
        Thread t = owner;
        if (ended && t != null) LockSupport.unpark(t);
        return ended;
    }

    /** True while any pause reason is set (the owner may not have stopped yet). */
    public boolean isPaused() {
        return reasons != 0;
    }

    public boolean hasReason(int reason) {
        return (reasons & reason) != 0;
    }

    /** True while the owner is parked in this gate. */
    public boolean isParked() {
        return parked;
    }

    /**
     * True if the owner is not executing guest code: parked, or inside a host
     * call (which parks before returning to the guest while paused).
     */
    public boolean isHalted() {
        return parked || hostDepth > 0;
    }

    /** Total wall time spent paused, including a pause in progress. */
    public long pausedNanos() {
        synchronized (lock) {
            long d = pausedNanosDone;
            return reasons != 0 ? d + (System.nanoTime() - pauseStartNanos) : d;
        }
    }

    /** {@link System#nanoTime()} minus paused time: this thread's unpaused clock. */
    public long activeNanoTime() {
        return System.nanoTime() - pausedNanos();
    }

    /**
     * Park the calling (owner) thread while a pause is requested. Returns
     * {@code true} when running again, {@code false} if the thread was
     * interrupted while paused (its interrupt flag is left set).
     */
    public boolean awaitResume() {
        if (reasons == 0) return true;
        Thread me = Thread.currentThread();
        synchronized (lock) {
            if (reasons == 0) return true;
            parked = true;
            try {
                while (reasons != 0) {
                    lock.wait();
                }
                return true;
            } catch (InterruptedException e) {
                me.interrupt();
                return false;
            } finally {
                parked = false;
            }
        }
    }

    /**
     * Owner-side safe point in host code: park while paused; if the thread is
     * killed while paused, unwind instead of returning to the guest.
     */
    public void checkpoint() {
        if (reasons != 0 && !awaitResume()) {
            throw new RuntimeException("WASI child interrupted while paused");
        }
    }

    // --- host-call bracketing (owner thread only) ---

    /** Before a host call: stop here if paused, then mark the thread as in host code. */
    public void enterHost() {
        checkpoint();
        hostDepth++;
    }

    /** After a host call returned (or threw). Call {@link #checkpoint()} before resuming the guest. */
    public void leaveHost() {
        hostDepth--;
    }

    /** Bracket guest execution that may be parked from the runtime's interruption poll. */
    public void setWasmActive(boolean active) {
        this.wasmActive = active;
    }

    /** Whether {@link PausableThread#isInterrupted()} may park here: in guest code, not in a host call. */
    boolean mayParkInGuest() {
        return wasmActive && hostDepth == 0 && !parked;
    }

    // --- pause-aware waiting ---

    /** The gate of the calling thread, or {@code null} off a {@link PausableThread}. */
    public static PauseGate current() {
        return Thread.currentThread() instanceof PausableThread p ? p.gate() : null;
    }

    /** {@link #pausedNanos()} of the calling thread's gate, or 0. */
    public static long currentPausedNanos() {
        PauseGate g = current();
        return g == null ? 0 : g.pausedNanos();
    }

    /**
     * Sleep for {@code nanos} of the calling thread's unpaused time: while its
     * gate is paused the sleep parks in the gate and the deadline moves out by
     * the paused duration. Off a pausable thread this is a plain sleep.
     *
     * @throws RuntimeException with {@code interruptMessage} if interrupted
     */
    public static void sleepNanos(long nanos, String interruptMessage) {
        PauseGate g = current();
        Thread me = Thread.currentThread();
        long deadline = (g == null ? System.nanoTime() : g.activeNanoTime()) + nanos;
        while (true) {
            if (g != null && g.reasons != 0 && !g.awaitResume()) {
                throw new RuntimeException(interruptMessage);
            }
            long remaining = deadline - (g == null ? System.nanoTime() : g.activeNanoTime());
            if (remaining <= 0) return;
            LockSupport.parkNanos(remaining);
            if (me.isInterrupted()) {
                throw new RuntimeException(interruptMessage);
            }
        }
    }
}
