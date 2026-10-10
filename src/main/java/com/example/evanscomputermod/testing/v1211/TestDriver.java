package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.core.BlockPos;
import net.minecraft.gametest.framework.GameTestAssertException;
import net.minecraft.gametest.framework.GameTestHelper;

import java.util.function.BooleanSupplier;
import java.util.function.Supplier;

/**
 * Shared plumbing for the 1.21.1 GameTests (annotation-based API).
 *
 * <p>Tests are bounded by each run's wall-clock limit, not ticks: the test
 * server ticks unthrottled while computers run in real time, so
 * {@link #BACKSTOP_TICKS} must be far beyond a minute of ticks or it fires
 * first and hides the real failure.
 */
final class TestDriver {
    static final int BACKSTOP_TICKS = Integer.MAX_VALUE / 2;
    static final String STRUCTURE = "gametest_switch";

    /** Layout origin inside the structure: bounding box (plus the 1-block clear margin) at (0,1,0). */
    static BlockPos origin(Scenario s) {
        BlockPos min = s.min();
        return new BlockPos(1 - min.getX(), 1 - min.getY(), 1 - min.getZ());
    }

    static ScenarioRun build(GameTestHelper h, Scenario s, String tag) {
        ScenarioRun run = new ScenarioRun(s, h.getLevel(), h.absolutePos(origin(s)),
                msg -> EvansComputerMod.LOGGER.info("[{}] {}", tag, msg.replaceAll("§.", "")), 1, true);
        run.build();
        return run;
    }

    /**
     * Run {@code step} every tick until it returns true, then log
     * {@code ECM_<NS>_TEST_PASS <name>}. A non-null {@code failure} ends the
     * test at once: throwing inside succeedWhen only means "not yet" (it is
     * retried until the tick limit), so failures go through their own
     * sequence, whose thenFail ends the test immediately.
     */
    static void drive(GameTestHelper h, String namespace, String name, BooleanSupplier step, Supplier<String> failure) {
        h.startSequence()
                .thenWaitUntil(() -> {
                    if (failure.get() == null) throw new GameTestAssertException("not failed");
                })
                .thenFail(() -> new GameTestAssertException(failure.get()));
        String marker = "ECM_" + namespace.replaceFirst("^ecm_", "").toUpperCase() + "_TEST_PASS";
        h.succeedWhen(() -> {
            if (failure.get() != null || !step.getAsBoolean()) throw new GameTestAssertException("running " + name);
            EvansComputerMod.LOGGER.info("{} {}", marker, name);
        });
    }

    /**
     * A whole scenario as a test. A {@link Scenario#realTime} scenario (SDR
     * sample clocks run on game time) holds the otherwise unthrottled test
     * server to 20 ticks per second while it runs, as a normal server does.
     */
    static void scenario(GameTestHelper h, String namespace, Scenario s) {
        scenario(h, namespace, s, false);
    }

    /** {@link #scenario}; with {@code clearAfter} the layout is removed once it passes (stops anything left running). */
    static void scenario(GameTestHelper h, String namespace, Scenario s, boolean clearAfter) {
        ScenarioRun run = build(h, s, s.name);
        long[] start = {0}, ticks = {0};
        drive(h, namespace, s.name, () -> {
            if (s.realTime) {
                long now = System.nanoTime();
                if (start[0] == 0) start[0] = now;
                long due = start[0] + ++ticks[0] * 50_000_000L;
                if (due > now) java.util.concurrent.locks.LockSupport.parkNanos(due - now);
            }
            if (run.tick() != ScenarioRun.State.PASSED) return false;
            if (clearAfter) run.clear();
            return true;
        }, () -> run.state() == ScenarioRun.State.FAILED ? run.failure() + "\n" + run.dump() : null);
    }

    private TestDriver() {}
}
//?}
