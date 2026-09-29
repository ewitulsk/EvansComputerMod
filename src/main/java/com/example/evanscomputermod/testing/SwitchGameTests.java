package com.example.evanscomputermod.testing;

//? if >=26.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.example.evanscomputermod.testing.scenario.SwitchScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.FunctionGameTestInstance;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.gametest.framework.TestData;
import net.minecraft.gametest.framework.TestEnvironmentDefinition;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.event.RegisterGameTestsEvent;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/**
 * The switching debug scenarios ({@link SwitchScenarios}, the same ones
 * {@code /ecm scenario spawn} builds) as GameTests. Namespace
 * {@code ecm_switch}; each passing test logs {@code ECM_SWITCH_TEST_PASS <name>}.
 *
 * <p>Each scenario gets its own test environment, which puts it in its own
 * batch: batches run one after another, so the scenarios don't compete for
 * CPU (they run up to six kernels each against a wall-clock limit).
 */
public final class SwitchGameTests {
    public static final String NAMESPACE = "ecm_switch";
    private static final String STRUCTURE = "gametest_switch";
    /** Must match scripts/gen-gametest-structure.py. */
    private static final BlockPos STRUCTURE_SIZE = new BlockPos(20, 8, 12);
    /**
     * Tick backstop only; each scenario has a wall-clock limit (see
     * NetworkGameTests). The test server ticks unthrottled (~30k ticks/s
     * here), so this must be far beyond a minute of ticks, or it fires first
     * and hides the scenario's own failure report.
     */
    private static final int BACKSTOP_TICKS = Integer.MAX_VALUE / 2;

    private static final DeferredRegister<Consumer<GameTestHelper>> FUNCTIONS =
            DeferredRegister.create(Registries.TEST_FUNCTION, NAMESPACE);
    private static final Map<String, DeferredHolder<Consumer<GameTestHelper>, Consumer<GameTestHelper>>> TESTS =
            new LinkedHashMap<>();

    static {
        for (Scenario s : SwitchScenarios.ALL.values()) {
            TESTS.put(s.name, FUNCTIONS.register(s.name, () -> h -> run(h, s)));
        }
    }

    public static void register(IEventBus modBus) {
        FUNCTIONS.register(modBus);
        modBus.addListener(SwitchGameTests::onRegisterTests);
    }

    private static Identifier id(String path) {
        return Identifier.fromNamespaceAndPath(NAMESPACE, path);
    }

    private static void onRegisterTests(RegisterGameTestsEvent event) {
        Identifier structure = Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, STRUCTURE);
        for (Scenario s : SwitchScenarios.ALL.values()) {
            BlockPos need = s.max().subtract(s.min()).offset(3, 3, 3); // margins, see origin()
            if (need.getX() > STRUCTURE_SIZE.getX() || need.getY() > STRUCTURE_SIZE.getY()
                    || need.getZ() > STRUCTURE_SIZE.getZ()) {
                throw new IllegalStateException(s.name + " needs a " + need.toShortString()
                        + " structure; enlarge " + STRUCTURE + " in scripts/gen-gametest-structure.py");
            }
            Holder<TestEnvironmentDefinition<?>> env =
                    event.registerEnvironment(id("env_" + s.name), new TestEnvironmentDefinition.AllOf(List.of()));
            event.registerTest(id(s.name), new FunctionGameTestInstance(TESTS.get(s.name).getKey(),
                    new TestData<>(env, structure, BACKSTOP_TICKS, 0, true)));
        }
    }

    /** Layout origin inside the structure: its bounding box (plus the 1-block clear margin) starts at (0,1,0). */
    private static BlockPos origin(Scenario s) {
        BlockPos min = s.min();
        return new BlockPos(1 - min.getX(), 1 - min.getY(), 1 - min.getZ());
    }

    private static void run(GameTestHelper h, Scenario s) {
        BlockPos rel = origin(s);
        ScenarioRun run = new ScenarioRun(s, h.getLevel(), h.absolutePos(rel),
                msg -> EvansComputerMod.LOGGER.info("[{}] {}", s.name, msg.replaceAll("§.", "")), 1, true);
        run.build();
        // Throwing inside succeedWhen only means "not yet" (it is retried until
        // the tick limit), so a failure goes through its own sequence, whose
        // thenFail ends the test at once.
        h.startSequence()
                .thenWaitUntil(() -> {
                    if (run.state() != ScenarioRun.State.FAILED) throw h.assertionException("not failed");
                })
                .thenFail(() -> h.assertionException(rel, Component.literal(run.failure() + "\n" + run.dump())));
        h.succeedWhen(() -> {
            if (run.tick() != ScenarioRun.State.PASSED) {
                throw h.assertionException("still running " + s.name);
            }
            EvansComputerMod.LOGGER.info("ECM_SWITCH_TEST_PASS {}", s.name);
        });
    }

    private SwitchGameTests() {}
}
//?}
