package com.example.evanscomputermod.computer.pipeline;

import com.example.evanscomputermod.EvansComputerMod;

import java.util.concurrent.ScheduledThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The threads that move display frames (docs/gui/DISPLAY_PIPELINE.md "Pipeline
 * today", DISPLAY_PIPELINE_SPEC.md Part A1): diffing, encoding, compressing and
 * sending for every computer, off the Minecraft server thread. A small pool,
 * {@code display.pumpThreads} big (default {@code min(4, cores / 2)}).
 *
 * <p>Work is keyed per computer through a {@link Lane}: one computer's passes
 * never overlap (each runs after the last one finished), and a request made
 * while a pass is queued or running is folded into one more pass, never a
 * backlog. Different computers' lanes run at the same time on different
 * threads, so one slow computer no longer delays the others.
 *
 * <p>Sends from these threads go through {@code PacketDistributor.sendToPlayer},
 * which is safe off the server thread (Netty encodes on the connection's
 * event loop). What needs the server thread (who watches what) is computed
 * once per tick and published as a {@link ViewerSnapshot}.
 */
public final class DisplayPump {
    private DisplayPump() {}

    private static final AtomicInteger THREADS = new AtomicInteger();
    private static volatile ScheduledThreadPoolExecutor pool;

    /** Passes run so far, and the nanoseconds they took (all lanes). */
    private static final AtomicLong PASSES = new AtomicLong(), PASS_NANOS = new AtomicLong(), MAX_PASS_NANOS = new AtomicLong();
    /** Display work that still ran on the server thread (should stay near 0; diagnostics). */
    private static final AtomicLong SERVER_THREAD_NANOS = new AtomicLong();

    /** The default pool size: {@code min(4, cores / 2)}, at least 1. */
    public static int defaultThreads() {
        return Math.max(1, Math.min(4, Runtime.getRuntime().availableProcessors() / 2));
    }

    /** The shared scheduler (created on first use with {@code display.pumpThreads} threads). */
    public static ScheduledThreadPoolExecutor scheduler() {
        ScheduledThreadPoolExecutor p = pool;
        if (p != null) return p;
        synchronized (DisplayPump.class) {
            if (pool == null) {
                int n = Math.max(1, com.example.evanscomputermod.EcmConfig.displayPumpThreads());
                ScheduledThreadPoolExecutor e = new ScheduledThreadPoolExecutor(n, r -> {
                    Thread t = new Thread(r, "ECM-Display-Pump-" + THREADS.incrementAndGet());
                    t.setDaemon(true);
                    return t;
                });
                e.setRemoveOnCancelPolicy(true);
                pool = e;
            }
            return pool;
        }
    }

    /** Pool threads now. */
    public static int threads() {
        ScheduledThreadPoolExecutor p = pool;
        return p == null ? 0 : p.getCorePoolSize();
    }

    /** Run {@code r} on the pump after {@code delayNanos}. */
    public static void schedule(Runnable r, long delayNanos) {
        scheduler().schedule(guard(r), Math.max(0, delayNanos), TimeUnit.NANOSECONDS);
    }

    private static Runnable guard(Runnable r) {
        return () -> {
            try {
                r.run();
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.debug("display pump task failed", t);
            }
        };
    }

    /** Is the calling thread one of the pump's? */
    public static boolean onPump() {
        return Thread.currentThread().getName().startsWith("ECM-Display-Pump-");
    }

    /**
     * Count display work a caller had to do on the server thread anyway
     * (a host without a lane, a test driving a hub directly). Reported by
     * {@code /ecm gui stats}; the goal is about zero.
     */
    public static void noteServerThread(long nanos) {
        SERVER_THREAD_NANOS.addAndGet(nanos);
    }

    public static long passes() {
        return PASSES.get();
    }

    public static long passNanos() {
        return PASS_NANOS.get();
    }

    public static long maxPassNanos() {
        return MAX_PASS_NANOS.get();
    }

    public static long serverThreadNanos() {
        return SERVER_THREAD_NANOS.get();
    }

    /** Is {@code t} the Minecraft server thread? */
    public static boolean isServerThread(Thread t) {
        return t.getName().equals("Server thread");
    }

    /**
     * One computer's serial queue on the pump: {@link #request()} runs the
     * task soon, at most once at a time, and coalesces requests that come in
     * while a run is queued or in progress into exactly one more run.
     */
    public static final class Lane {
        private static final int IDLE = 0, QUEUED = 1, RUNNING = 2, AGAIN = 3;
        private final AtomicInteger state = new AtomicInteger(IDLE);
        private final Runnable task;
        private final String name;
        private volatile boolean closed;
        private long passes, nanos;

        public Lane(String name, Runnable task) {
            this.name = name;
            this.task = task;
        }

        /** Run the task once more, soon (coalesced with any run already queued). */
        public void request() {
            if (closed) return;
            while (true) {
                int s = state.get();
                if (s == QUEUED || s == AGAIN) return;
                if (s == IDLE) {
                    if (state.compareAndSet(IDLE, QUEUED)) {
                        scheduler().execute(this::run);
                        return;
                    }
                } else if (state.compareAndSet(RUNNING, AGAIN)) {
                    return;
                }
            }
        }

        /** {@link #request()} after {@code delayNanos} (a timer: presentation times, pacing). */
        public void requestIn(long delayNanos) {
            if (closed) return;
            if (delayNanos <= 0) {
                request();
                return;
            }
            scheduler().schedule(this::request, delayNanos, TimeUnit.NANOSECONDS);
        }

        private void run() {
            state.set(RUNNING);
            long t0 = System.nanoTime();
            try {
                if (!closed) task.run();
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.debug("display pass {} failed", name, t);
            } finally {
                long dt = System.nanoTime() - t0;
                synchronized (this) {
                    passes++;
                    nanos += dt;
                }
                PASSES.incrementAndGet();
                PASS_NANOS.addAndGet(dt);
                MAX_PASS_NANOS.accumulateAndGet(dt, Math::max);
                if (!state.compareAndSet(RUNNING, IDLE)) {
                    // A request came in during the run: one more pass.
                    state.set(QUEUED);
                    if (!closed) scheduler().execute(this::run);
                    else state.set(IDLE);
                }
            }
        }

        /** Stop taking requests (the computer is gone); a run in progress finishes. */
        public void close() {
            closed = true;
        }

        public boolean closed() {
            return closed;
        }

        /** Passes run and their total time (diagnostics). */
        public synchronized long passes() {
            return passes;
        }

        public synchronized long nanos() {
            return nanos;
        }

        /** Is a pass queued or running? */
        public boolean busy() {
            return state.get() != IDLE;
        }
    }
}
