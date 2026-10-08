package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;

import org.junit.jupiter.api.Test;

import java.lang.management.ManagementFactory;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.SplittableRandom;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Medium benchmark (lane 3F/8C): transmit throughput at 100 / 1,000 / 10,000
 * radios on channels 1/6/11, single-threaded and on 4 threads, in a dense
 * layout (everyone in link-budget range) and a sparse one (0 dBm radios spread
 * over 100 km). Prints the numbers; asserts only sane minimums, and that
 * {@code transmit} never reads the world (no raycasts on the hot path).
 */
public class MediumBench {

    static final Channel[] PLAN = {Channel.wifi24(1), Channel.wifi24(6), Channel.wifi24(11)};

    record Run(int radios, boolean dense, int threads, double framesPerS, double deliveriesPerS, double bytesPerFrame) {}

    Run run(int n, boolean dense, int threads, int frames) throws Exception {
        AtomicLong clock = new AtomicLong(1_000_000);
        GridWorld world = new GridWorld(64, RfBlock.DIRT);
        WorldRadioMedium.WorldAccess access = new WorldRadioMedium.WorldAccess() {
            @Override public RfWorld world(String d) { return world; }
            @Override public WorldRadioMedium.Conditions conditions(String d) { return null; }
        };
        WorldRadioMedium m = new WorldRadioMedium(clock::get, 1);
        SplittableRandom rnd = new SplittableRandom(42);
        double side = dense ? Math.sqrt(n) * 8 : 100_000;
        List<TestEp> eps = new ArrayList<>();
        for (int i = 0; i < n; i++) {
            TestEp e = new TestEp(rnd.nextDouble(side), 65.5, rnd.nextDouble(side), PLAN[i % 3], false);
            eps.add(e);
            m.register(e);
        }
        for (int t = 0; t < 3; t++) m.tick(t, access);   // trace what the budget allows
        long readsBefore = world.reads;
        AtomicLong delivered = new AtomicLong();
        double power = dense ? 15 : 0;
        int per = frames / threads;
        Runnable[] jobs = new Runnable[threads];
        long[] allocated = new long[threads];
        for (int t = 0; t < threads; t++) {
            int tt = t;
            jobs[t] = () -> {
                var mx = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
                long a0 = mx.getCurrentThreadAllocatedBytes();
                SplittableRandom r = new SplittableRandom(tt);
                byte[] payload = new byte[100];
                for (int k = 0; k < per; k++) {
                    TestEp from = eps.get(r.nextInt(n));
                    long now = clock.addAndGet(500);
                    m.transmit(from, Emission.frame(from.ch, power, now, 300, "OFDM-24", 24e6, payload));
                }
                allocated[tt] = mx.getCurrentThreadAllocatedBytes() - a0;
            };
        }
        long countBefore = eps.stream().mapToLong(e -> e.asked.get()).sum();
        long t0 = System.nanoTime();
        if (threads == 1) jobs[0].run();
        else {
            ExecutorService pool = Executors.newFixedThreadPool(threads);
            List<Future<?>> fs = new ArrayList<>();
            for (Runnable j : jobs) fs.add(pool.submit(j));
            for (Future<?> f : fs) f.get(120, TimeUnit.SECONDS);
            pool.shutdown();
        }
        double s = (System.nanoTime() - t0) / 1e9;
        assertEquals(readsBefore, world.reads, "transmit read the world (raycast on the hot path)");
        long visited = eps.stream().mapToLong(e -> e.asked.get()).sum() - countBefore;
        long alloc = 0;
        for (long a : allocated) alloc += a;
        Run run = new Run(n, dense, threads, per * threads / s, visited / s, alloc / (double) (per * threads));
        System.out.printf(Locale.ROOT, "MediumBench radios=%d layout=%s threads=%d: %.0f frames/s, %.0f receiver checks/s, "
                        + "%.0f B allocated/frame, cache %d links%n",
                n, dense ? "dense" : "sparse", threads, run.framesPerS(), run.deliveriesPerS(), run.bytesPerFrame(), m.cachedLinks());
        return run;
    }

    @Test
    void throughputAt100_1000_10000Radios() throws Exception {
        run(100, true, 1, 2000);   // warm-up
        for (int n : new int[] {100, 1_000, 10_000}) {
            int frames = n == 10_000 ? 600 : n == 1_000 ? 3_000 : 20_000;
            Run dense = run(n, true, 1, frames);
            Run dense4 = run(n, true, 4, frames);
            Run sparse = run(n, false, 1, frames * 4);
            Run sparse4 = run(n, false, 4, frames * 4);
            // Sane minimums only: these are measurements, not limits.
            assertTrue(dense.framesPerS() > 30, "dense " + dense);
            assertTrue(dense4.framesPerS() > 30, "dense, 4 threads " + dense4);
            assertTrue(sparse.framesPerS() > 1_000, "sparse " + sparse);
            assertTrue(sparse4.framesPerS() > 1_000, "sparse, 4 threads " + sparse4);
        }
    }
}
