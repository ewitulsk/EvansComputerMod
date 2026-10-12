package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.DisplaySyncHub;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.network.ComputerTargets;
import com.example.evanscomputermod.network.DeltaApplier;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.common.util.FakePlayerFactory;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;
import java.util.function.BooleanSupplier;

/**
 * Screen clusters, namespace {@code ecm_screen}.
 */
@GameTestHolder(ScreenTests.NS)
@PrefixGameTestTemplate(false)
public final class ScreenTests {
    static final String NS = "ecm_screen";
    private static final String STRUCTURE = "gametest_empty";
    private static final long WALL_LIMIT_MS = 60_000;

    private static final BlockPos TERMINAL = new BlockPos(6, 2, 3);
    /** West of the north-facing terminal, facing west: the terminal is behind it. */
    private static final BlockPos SCREEN = TERMINAL.west();

    /**
     * A program that owns the Screen at its own resolution (gba, 240x160)
     * keeps it through a block update next to the terminal. The rescan that
     * update causes used to see the display's size differ from the cluster's,
     * replace the display and power the screen off: the Screen went black a
     * moment into a game, as soon as the speaker's cone moved.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".owned")
    public static void program_keeps_screen_through_neighbor_update(GameTestHelper h) {
        TerminalDisplay[] owned = new TerminalDisplay[1];
        long[] at = new long[1];
        int[] dirty = new int[1];
        run(h, "program_keeps_screen_through_neighbor_update", List.of(
                () -> {
                    h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                            .setValue(TerminalBlock.FACING, Direction.NORTH));
                    h.setBlock(SCREEN, ModBlocks.SCREEN_BLOCK.get().defaultBlockState()
                            .setValue(ScreenBlock.FACING, Direction.WEST));
                    return true;
                },
                () -> {
                    if (terminal(h) == null) return false;
                    terminal(h).initializeWasm(); // what opening the GUI does
                    return true;
                },
                () -> terminal(h).hasScreenCluster()
                        && screen(h).contains("Welcome to Terminal OS"),
                () -> {
                    Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(terminal(h));
                    try {
                        Files.createDirectories(dir);
                        Files.copy(repoRoot().resolve("rust/wasm-programs/gba/tests/roms/arm.gba"),
                                dir.resolve("arm.gba"), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                    } catch (java.io.IOException ex) {
                        throw new AssertionError("can't copy the test ROM: " + ex);
                    }
                    terminal(h).onStringInput("gba arm.gba screen --no-sound\n");
                    return true;
                },
                () -> {
                    TerminalDisplay sd = terminal(h).getScreenDisplay();
                    if (sd == null || sd.getGfxWidth() != 240 || sd.getGfxHeight() != 160 || !anchorActive(h)) {
                        return false;
                    }
                    owned[0] = sd;
                    return true;
                },
                () -> {
                    // A block update beside the terminal, like a speaker's cone changing state.
                    h.setBlock(TERMINAL.above(), Blocks.STONE.defaultBlockState());
                    h.setBlock(TERMINAL.above(), Blocks.AIR.defaultBlockState());
                    at[0] = System.currentTimeMillis();
                    return true;
                },
                () -> System.currentTimeMillis() - at[0] > 1000,
                () -> {
                    TerminalDisplay sd = terminal(h).getScreenDisplay();
                    check(sd == owned[0], "the screen's display was replaced");
                    check(sd.getGfxWidth() == 240 && sd.getGfxHeight() == 160,
                            "the screen is " + sd.getGfxWidth() + "x" + sd.getGfxHeight());
                    check(anchorActive(h), "the screen was powered off");
                    dirty[0] = sd.getPixelDirtyCounter();
                    return true;
                },
                // gba is still drawing to it.
                () -> terminal(h).getScreenDisplay().getPixelDirtyCounter() != dirty[0]));
    }

    /**
     * Two players watching one Screen cost one delta build per frame. A
     * running program's real Screen display is streamed by a
     * {@link DisplaySyncHub} to two viewers whose clients apply every packet
     * with the real client code and ack it: each frame is built once and
     * sent to both, and after every pass both clients show exactly the
     * server's picture.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".shared_deltas")
    public static void two_viewers_share_screen_deltas(GameTestHelper h) {
        UUID[] viewers = {UUID.randomUUID(), UUID.randomUUID()};
        DisplaySyncHub hub = new DisplaySyncHub(IComputerHost.DISPLAY_ATTACHED);
        java.util.Map<UUID, TerminalDisplay> clients = new java.util.HashMap<>();
        java.util.Map<UUID, Long> lastGen = new java.util.HashMap<>();
        long[] at = new long[1];
        List<BooleanSupplier> steps = new java.util.ArrayList<>(startGba(h));
        steps.add(() -> {
            at[0] = System.currentTimeMillis();
            return true;
        });
        steps.add(() -> {
            TerminalDisplay sd = terminal(h).getScreenDisplay();
            // Hold the display so gba can't draw between the pass and the comparison.
            synchronized (sd) {
                // The test ROM's picture is static once it has run: change a pixel
                // every tick (gba's next frame puts it back), so there are frames.
                sd.getPixelData()[0] ^= (byte) 0xFF;
                hub.sync(terminal(h).getComputerId(), sd, List.of(viewers), (id, packet) -> {
                    check(packet.displayId() == IComputerHost.DISPLAY_ATTACHED, "wrong display id");
                    var d = packet.parse();
                    TerminalDisplay c = clients.computeIfAbsent(id, k -> new TerminalDisplay(1, 1));
                    if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) DeltaApplier.applyKeyframe(c, d);
                    else DeltaApplier.applyDelta(c, d);
                    lastGen.put(id, d.generation());
                });
                for (UUID v : viewers) {
                    TerminalDisplay c = clients.get(v);
                    check(c != null, "a viewer got nothing");
                    check(java.util.Arrays.equals(c.getPixelData(), sd.getPixelData()),
                            "a client's picture differs from the server's after a pass");
                }
            }
            for (UUID v : viewers) hub.onClientReady(v, lastGen.get(v));
            return System.currentTimeMillis() - at[0] > 2000;
        });
        steps.add(() -> {
            long builds = hub.deltaBuilds(), keyframes = hub.keyframeBuilds(), sent = hub.packetsSent();
            com.example.evanscomputermod.EvansComputerMod.LOGGER.info(
                    "ECM_SCREEN shared deltas: {} delta builds, {} keyframe builds, {} packets sent to 2 viewers",
                    builds, keyframes, sent);
            check(builds >= 10, "there should have been frames: " + builds + " delta builds");
            check(sent == 2 * (builds + keyframes),
                    "each frame should be built once and sent to both viewers: " + sent + " sent for "
                            + (builds + keyframes) + " builds");
            return true;
        });
        run(h, "two_viewers_share_screen_deltas", steps);
    }

    /**
     * Display packets name a computer by UUID and the host checks range:
     * a player next to the terminal reaches it and its keys arrive; the same
     * player too far away, or a UUID no computer has, is refused (controls).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".uuid_input")
    public static void input_by_uuid_checks_range(GameTestHelper h) {
        ServerPlayer[] player = new ServerPlayer[1];
        run(h, "input_by_uuid_checks_range", List.of(
                () -> {
                    h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                            .setValue(TerminalBlock.FACING, Direction.NORTH));
                    return true;
                },
                () -> {
                    if (terminal(h) == null) return false;
                    terminal(h).initializeWasm();
                    return true;
                },
                () -> screen(h).contains("Welcome to Terminal OS") && screen(h).contains("/ >"),
                () -> {
                    player[0] = FakePlayerFactory.get(h.getLevel(),
                            new com.mojang.authlib.GameProfile(UUID.randomUUID(), "[ECM uuid test]"));
                    UUID id = terminal(h).getComputerId();
                    place(h, player[0], TERMINAL.north(20));
                    check(ComputerTargets.resolve(player[0], id, IComputerHost.DISPLAY_OWN) == null,
                            "a player 20 blocks away must be refused");
                    place(h, player[0], TERMINAL.north(2));
                    check(ComputerTargets.resolve(player[0], UUID.randomUUID(), IComputerHost.DISPLAY_OWN) == null,
                            "an unknown computer id must resolve to nothing");
                    check(ComputerTargets.resolve(player[0], id, IComputerHost.DISPLAY_ATTACHED) != null,
                            "the screen display is in range too");
                    IComputerHost host = ComputerTargets.resolve(player[0], id, IComputerHost.DISPLAY_OWN);
                    check(host == terminal(h), "a player 2 blocks away should reach the terminal");
                    // What TerminalInputPacket's handler does with a resolved host.
                    ComputerTargets.instance(host).handleKeyInput("echo uuid_input_ok\n");
                    return true;
                },
                () -> screen(h).contains("\nuuid_input_ok")));
    }

    // ------------------------------------------------------------ helpers

    /** Place the terminal and a Screen beside it, boot, and run gba on the Screen at 240x160. */
    private static List<BooleanSupplier> startGba(GameTestHelper h) {
        return List.of(
                () -> {
                    h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                            .setValue(TerminalBlock.FACING, Direction.NORTH));
                    h.setBlock(SCREEN, ModBlocks.SCREEN_BLOCK.get().defaultBlockState()
                            .setValue(ScreenBlock.FACING, Direction.WEST));
                    return true;
                },
                () -> {
                    if (terminal(h) == null) return false;
                    terminal(h).initializeWasm(); // what opening the GUI does
                    return true;
                },
                () -> terminal(h).hasScreenCluster()
                        && screen(h).contains("Welcome to Terminal OS"),
                () -> {
                    Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(terminal(h));
                    try {
                        Files.createDirectories(dir);
                        Files.copy(repoRoot().resolve("rust/wasm-programs/gba/tests/roms/arm.gba"),
                                dir.resolve("arm.gba"), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                    } catch (java.io.IOException ex) {
                        throw new AssertionError("can't copy the test ROM: " + ex);
                    }
                    terminal(h).onStringInput("gba arm.gba screen --no-sound\n");
                    return true;
                },
                () -> {
                    TerminalDisplay sd = terminal(h).getScreenDisplay();
                    return sd != null && sd.getGfxWidth() == 240 && sd.getGfxHeight() == 160 && anchorActive(h);
                });
    }

    private static void place(GameTestHelper h, ServerPlayer player, BlockPos rel) {
        BlockPos abs = h.absolutePos(rel);
        player.moveTo(abs.getX() + 0.5, abs.getY(), abs.getZ() + 0.5);
    }

    private static TerminalBlockEntity terminal(GameTestHelper h) {
        return h.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity t ? t : null;
    }

    private static String screen(GameTestHelper h) {
        TerminalBlockEntity t = terminal(h);
        return t == null ? "" : ScenarioRun.screen(t.getDisplay());
    }

    private static boolean anchorActive(GameTestHelper h) {
        return h.getBlockEntity(SCREEN) instanceof ScreenBlockEntity s && s.isActive();
    }

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        if (p == null) throw new AssertionError("repository root not found from " + Path.of("").toAbsolutePath());
        return p;
    }

    private static void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }

    /** Each step runs every tick until it returns true. */
    private static void run(GameTestHelper h, String name, List<BooleanSupplier> steps) {
        int[] step = {0};
        long[] started = {0};
        String[] failure = {null};
        TestDriver.drive(h, NS, name, () -> {
            if (started[0] == 0) started[0] = System.currentTimeMillis();
            if (System.currentTimeMillis() - started[0] > WALL_LIMIT_MS) {
                failure[0] = "timed out at step " + (step[0] + 1) + "/" + steps.size() + "\n" + screen(h);
                return false;
            }
            try {
                while (step[0] < steps.size() && steps.get(step[0]).getAsBoolean()) step[0]++;
            } catch (AssertionError | RuntimeException ex) {
                failure[0] = "step " + (step[0] + 1) + ": " + ex.getMessage();
                return false;
            }
            return step[0] >= steps.size();
        }, () -> failure[0]);
    }
}
//?}
