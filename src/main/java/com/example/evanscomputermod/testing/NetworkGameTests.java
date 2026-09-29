package com.example.evanscomputermod.testing;

//? if >=26.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.FunctionGameTestInstance;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.gametest.framework.TestData;
import net.minecraft.gametest.framework.TestEnvironmentDefinition;
import net.minecraft.resources.Identifier;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.event.RegisterGameTestsEvent;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.function.Predicate;

/**
 * In-world networking tests: real Terminal blocks joined by real cable
 * blocks, booting the real kernel. Namespace {@code ecm_network}.
 *
 * <p>Layout (all tests): a cable run along y=1 from x=1 to x=9; terminal A
 * sits on it at (1,2,1), terminal B at (9,2,1). A terminal's down face is
 * always its first NIC, eth0, so each terminal's eth0 is cabled to the other.
 *
 * <p>Each passing test logs {@code ECM_NETWORK_TEST_PASS <case>}; the runner
 * (scripts/Test.ps1) counts those markers.
 */
public final class NetworkGameTests {
    public static final String NAMESPACE = "ecm_network";

    private static final DeferredRegister<Consumer<GameTestHelper>> FUNCTIONS =
            DeferredRegister.create(Registries.TEST_FUNCTION, NAMESPACE);

    private static final DeferredHolder<Consumer<GameTestHelper>, Consumer<GameTestHelper>> PING =
            FUNCTIONS.register("ping_between_cabled_terminals", () -> NetworkGameTests::ping);
    private static final DeferredHolder<Consumer<GameTestHelper>, Consumer<GameTestHelper>> PING_CONTROL =
            FUNCTIONS.register("no_cable_no_ping", () -> NetworkGameTests::pingWithoutCable);
    private static final DeferredHolder<Consumer<GameTestHelper>, Consumer<GameTestHelper>> SSH =
            FUNCTIONS.register("ssh_between_cabled_terminals", () -> NetworkGameTests::ssh);

    /** Wall-clock limit per test (see onRegisterTests). */
    private static final long WALL_CLOCK_LIMIT_MS = 60_000;
    /** Tick backstop only; the wall-clock limit is what bounds a test. */
    private static final int BACKSTOP_TICKS = 1_000_000;

    private static final BlockPos A = new BlockPos(1, 2, 1);
    private static final BlockPos B = new BlockPos(9, 2, 1);

    public static void register(IEventBus modBus) {
        FUNCTIONS.register(modBus);
        modBus.addListener(NetworkGameTests::onRegisterTests);
    }

    private static Identifier id(String path) {
        return Identifier.fromNamespaceAndPath(NAMESPACE, path);
    }

    private static void onRegisterTests(RegisterGameTestsEvent event) {
        Holder<TestEnvironmentDefinition<?>> env =
                event.registerEnvironment(id("default"), new TestEnvironmentDefinition.AllOf(List.of()));
        Identifier structure = Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "gametest_empty");
        // Why not the usual 1200-tick ceiling: the computers run in REAL time
        // on their own worker threads, while the GameTest server ticks as fast
        // as it can (1200 ticks pass in ~3 s). So each test is bounded by a
        // 60 s WALL-CLOCK limit in Script (the ModTesting.md one-minute rule),
        // and the tick limit is only a backstop.
        event.registerTest(id("ping_between_cabled_terminals"),
                new FunctionGameTestInstance(PING.getKey(), new TestData<>(env, structure, BACKSTOP_TICKS, 0, true)));
        event.registerTest(id("no_cable_no_ping"),
                new FunctionGameTestInstance(PING_CONTROL.getKey(), new TestData<>(env, structure, BACKSTOP_TICKS, 0, true)));
        event.registerTest(id("ssh_between_cabled_terminals"),
                new FunctionGameTestInstance(SSH.getKey(), new TestData<>(env, structure, BACKSTOP_TICKS, 0, true)));
    }

    // ------------------------------------------------------------ tests

    private static void ping(GameTestHelper h) {
        build(h, true);
        script(h, "ping_between_cabled_terminals")
                .boot()
                .send(A, "ifconfig eth0 10.0.0.1/24")
                .expect(A, s -> s.contains("eth0: inet 10.0.0.1/24"), "A configured")
                .send(B, "ifconfig eth0 10.0.0.2/24")
                .expect(B, s -> s.contains("eth0: inet 10.0.0.2/24"), "B configured")
                .send(A, "ping 10.0.0.2 -n 2")
                .expect(A, s -> s.contains("Reply from 10.0.0.2"), "ping reply over the cable")
                .run();
    }

    /** Control: identical setup without the cable must NOT get replies. */
    private static void pingWithoutCable(GameTestHelper h) {
        build(h, false);
        script(h, "no_cable_no_ping")
                .boot()
                .send(A, "ifconfig eth0 10.0.0.1/24")
                .expect(A, s -> s.contains("eth0: inet 10.0.0.1/24"), "A configured")
                .send(B, "ifconfig eth0 10.0.0.2/24")
                .expect(B, s -> s.contains("eth0: inet 10.0.0.2/24"), "B configured")
                .send(A, "ping 10.0.0.2 -n 1")
                .expect(A, s -> s.contains("1 packets sent, 0 received"), "no reply without a cable")
                .check(A, s -> !s.contains("Reply from 10.0.0.2"), "a reply arrived with no cable")
                .run();
    }

    private static void ssh(GameTestHelper h) {
        build(h, true);
        script(h, "ssh_between_cabled_terminals")
                .boot()
                .send(A, "ifconfig eth0 10.0.0.1/24")
                .expect(A, s -> s.contains("eth0: inet 10.0.0.1/24"), "A configured")
                .send(B, "ifconfig eth0 10.0.0.2/24")
                .expect(B, s -> s.contains("eth0: inet 10.0.0.2/24"), "B configured")
                .send(B, "sshd &")
                .expect(B, s -> s.contains("sshd: starting on port 22"), "sshd listening on B")
                .send(A, "ssh root@10.0.0.2")
                .expect(A, s -> s.contains("logged in as root"), "login banner from B")
                .send(A, "echo over-ssh-ok")
                .expect(A, s -> s.lines().anyMatch(l -> l.trim().equals("over-ssh-ok")), "command ran on B")
                .send(A, "exit")
                .expect(A, s -> s.contains("Connection closed."), "logout")
                .run();
    }

    // ------------------------------------------------------------ fixture

    private static void build(GameTestHelper h, boolean cable) {
        if (cable) {
            for (int x = 1; x <= 9; x++) {
                h.setBlock(new BlockPos(x, 1, 1), ModBlocks.NETWORK_CABLE.get());
            }
        }
        h.setBlock(A, ModBlocks.TERMINAL_BLOCK.get());
        h.setBlock(B, ModBlocks.TERMINAL_BLOCK.get());
    }

    private static Script script(GameTestHelper h, String name) {
        return new Script(h, name);
    }

    /** Screen text of a terminal (rows joined by newlines). */
    static String screen(TerminalDisplay d) {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < d.getHeight(); y++) {
            for (int x = 0; x < d.getWidth(); x++) {
                int c = d.getCharAt(x, y) & 0xFF;
                sb.append(c >= 32 && c < 127 ? (char) c : ' ');
            }
            sb.append('\n');
        }
        return sb.toString();
    }

    /**
     * A sequence of terminal steps run one per tick inside succeedWhen: a
     * step either sends a line or waits for text on a terminal's screen.
     * If the test times out, the failure names the step it was stuck on
     * and dumps both screens.
     */
    private static final class Script {
        private interface Step {
            boolean run(); // true = done
            String describe();
        }

        private final GameTestHelper h;
        private final String name;
        private final List<Step> steps = new ArrayList<>();
        private int index = 0;

        Script(GameTestHelper h, String name) {
            this.h = h;
            this.name = name;
        }

        private TerminalBlockEntity be(BlockPos p) {
            return h.getBlockEntity(p, TerminalBlockEntity.class);
        }

        Script boot() {
            steps.add(new Step() {
                boolean started;
                public boolean run() {
                    if (!started) {
                        be(A).initializeWasm();
                        be(B).initializeWasm();
                        started = true;
                    }
                    return screen(be(A).getDisplay()).contains("Welcome to Terminal OS")
                            && screen(be(B).getDisplay()).contains("Welcome to Terminal OS");
                }
                public String describe() { return "both terminals booted"; }
            });
            return this;
        }

        Script send(BlockPos p, String line) {
            steps.add(new Step() {
                public boolean run() {
                    be(p).onStringInput(line + "\n");
                    return true;
                }
                public String describe() { return "send '" + line + "' to " + p; }
            });
            return this;
        }

        Script expect(BlockPos p, Predicate<String> cond, String what) {
            steps.add(new Step() {
                public boolean run() { return cond.test(screen(be(p).getDisplay())); }
                public String describe() { return "wait for " + what + " on " + p; }
            });
            return this;
        }

        /** An immediate assertion (fails the test at once if false). */
        Script check(BlockPos p, Predicate<String> cond, String failure) {
            steps.add(new Step() {
                public boolean run() {
                    if (!cond.test(screen(be(p).getDisplay()))) {
                        h.fail(net.minecraft.network.chat.Component.literal(failure + "\n" + dump()), p);
                    }
                    return true;
                }
                public String describe() { return "check: not(" + failure + ")"; }
            });
            return this;
        }

        private String dump() {
            return "--- A ---\n" + screen(be(A).getDisplay()).stripTrailing()
                    + "\n--- B ---\n" + screen(be(B).getDisplay()).stripTrailing();
        }

        void run() {
            long start = System.currentTimeMillis();
            h.succeedWhen(() -> {
                while (index < steps.size() && steps.get(index).run()) {
                    index++;
                }
                if (index < steps.size() && System.currentTimeMillis() - start > WALL_CLOCK_LIMIT_MS) {
                    h.fail(net.minecraft.network.chat.Component.literal("timed out after "
                            + WALL_CLOCK_LIMIT_MS / 1000 + " s at step " + (index + 1) + "/" + steps.size() + ": "
                            + steps.get(index).describe() + "\n" + dump()), A);
                }
                if (index < steps.size()) {
                    throw h.assertionException("step " + (index + 1) + "/" + steps.size() + ": "
                            + steps.get(index).describe() + "\n" + dump());
                }
                EvansComputerMod.LOGGER.info("ECM_NETWORK_TEST_PASS {}", name);
            });
        }
    }

    private NetworkGameTests() {}
}
//?}
