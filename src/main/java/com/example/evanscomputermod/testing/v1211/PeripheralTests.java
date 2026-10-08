package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.compat.create.CreateCompat;
import com.example.evanscomputermod.computer.peripheral.PeripheralHub;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.module.ModDataComponents;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.simibubi.create.AllBlocks;
import com.simibubi.create.content.redstone.link.LinkBehaviour;
import com.simibubi.create.content.redstone.link.RedstoneLinkBlock;
import com.simibubi.create.content.redstone.link.RedstoneLinkBlockEntity;
import com.simibubi.create.foundation.blockEntity.behaviour.BlockEntityBehaviour;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.DirectionalBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.function.BooleanSupplier;

/**
 * Peripherals and module bays, namespace {@code ecm_periph} (1.21.1, needs
 * Create in the run: the runner loads it unless {@code -NoCreate}).
 *
 * <p>Covers installing and ejecting expansion cards and modules through the
 * block's real right-click paths, keeping modules and their settings on the
 * dropped item, block peripherals next to a computer, the Redstone Link
 * module against real Create Redstone Links in both directions, and one
 * end-to-end run of a Python program on a booted computer (peripheral API,
 * errors, and an event from a Create transmitter).
 */
@GameTestHolder(PeripheralTests.NS)
@PrefixGameTestTemplate(false)
public final class PeripheralTests {
    static final String NS = "ecm_periph";
    private static final String STRUCTURE = "gametest_empty";
    private static final long WALL_LIMIT_MS = 60_000;

    private static final BlockPos TERMINAL = new BlockPos(6, 2, 3);
    private static final String DROP_FIRST = "minecraft:cyan_dye";          // drop test only
    private static final String DROP_SECOND = "minecraft:quartz";

    /**
     * Frequency pair per test. Create's link network spans the whole level and
     * the blocks of finished tests stay loaded, so tests on the same pair would
     * hear each other.
     */
    private record Freq(String first, String second) {
        ItemStack firstStack() {
            return new ItemStack(net.minecraft.core.registries.BuiltInRegistries.ITEM.get(
                    net.minecraft.resources.ResourceLocation.parse(first)));
        }

        ItemStack secondStack() {
            return new ItemStack(net.minecraft.core.registries.BuiltInRegistries.ITEM.get(
                    net.minecraft.resources.ResourceLocation.parse(second)));
        }
    }

    private static final Freq TX_FREQ = new Freq("minecraft:red_dye", "minecraft:lapis_lazuli");
    private static final Freq RX_FREQ = new Freq("minecraft:lime_dye", "minecraft:diamond");
    private static final Freq PY_FREQ = new Freq("minecraft:blue_dye", "minecraft:emerald");

    // ------------------------------------------------------------ tests

    /** Control: a module can't go in without an expansion card. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".control")
    public static void module_needs_expansion_card(GameTestHelper h) {
        Env e = new Env(h);
        e.run("module_needs_expansion_card", List.of(
                e::placeTerminal,
                () -> {
                    ItemStack module = e.hold(CreateCompat.REDSTONE_LINK_MODULE.get().getDefaultInstance());
                    e.useOn(Direction.WEST, 0.75);
                    check(module.getCount() == 1, "module was consumed without a card");
                    check(e.bays().cardCount() == 0, "a bay opened");
                    check(e.bays().getStack(0).isEmpty(), "module installed without a card");
                    check(e.state().getValue(TerminalBlock.SLOTS[0]) == ModuleSlotVisual.EMPTY, "slot shows a module");
                    return true;
                }));
    }

    /** Cards and modules go in and come out through the block's right-click paths, and the model follows. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".install")
    public static void install_and_eject(GameTestHelper h) {
        Env e = new Env(h);
        e.run("install_and_eject", List.of(
                e::placeTerminal,
                () -> {
                    ItemStack card = e.hold(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get(), 2));
                    e.useOn(Direction.WEST, 0.5);
                    check(card.getCount() == 1, "card not consumed");
                    check(e.state().getValue(TerminalBlock.LEFT_BAY), "left bay not shown");
                    check(!e.state().getValue(TerminalBlock.RIGHT_BAY), "right bay shown too early");

                    e.hold(new ItemStack(CreateCompat.REDSTONE_LINK_MODULE.get(), 2));
                    e.useOn(Direction.WEST, 0.75);   // aim at the upper slot
                    check(e.state().getValue(TerminalBlock.SLOTS[0]) == ModuleSlotVisual.REDSTONE_LINK, "upper slot not shown");
                    e.useOn(Direction.NORTH, 0.5);   // not a bay face: first free slot
                    check(e.state().getValue(TerminalBlock.SLOTS[1]) == ModuleSlotVisual.REDSTONE_LINK, "lower slot not shown");
                    check(e.player.getMainHandItem().isEmpty(), "modules not consumed");

                    e.hold(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get()));
                    e.useOn(Direction.NORTH, 0.5);
                    check(e.state().getValue(TerminalBlock.RIGHT_BAY), "right bay not opened by the second card");
                    return true;
                },
                () -> e.hub().names().contains("left_bay_2"),   // attached on the computer's next tick
                () -> {
                    List<String> names = e.hub().names();
                    check(names.contains("left_bay_1") && names.contains("left_bay_2"), "hub has " + names);
                    check("redstone_link".equals(e.hub().get("left_bay_1").getType()), "wrong type");

                    e.hold(ItemStack.EMPTY);
                    e.player.setShiftKeyDown(true);
                    e.useEmpty(Direction.WEST, 0.75);
                    check(e.bays().getStack(0).isEmpty(), "upper module not ejected");
                    check(e.state().getValue(TerminalBlock.SLOTS[0]) == ModuleSlotVisual.EMPTY, "upper slot still shown");
                    check(e.count(CreateCompat.REDSTONE_LINK_MODULE.get().asItem()) == 1, "ejected module not given back");
                    check(!e.hub().names().contains("left_bay_1"), "ejected module still attached");

                    e.useEmpty(Direction.WEST, 0.75); // upper empty: takes the other module of the bay
                    check(e.bays().getStack(1).isEmpty(), "lower module not ejected");
                    e.useEmpty(Direction.WEST, 0.25); // bay empty: takes the card
                    check(!e.state().getValue(TerminalBlock.LEFT_BAY), "left bay still shown");
                    check(e.count(ModItems.MODULE_EXPANSION_CARD.get()) == 1, "card not given back");
                    check(e.state().getValue(TerminalBlock.RIGHT_BAY), "right bay closed too");
                    return true;
                }));
    }

    /** Breaking the computer keeps its cards, modules and module settings on the item. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".drop")
    public static void drop_keeps_modules_and_settings(GameTestHelper h) {
        Env e = new Env(h);
        ItemStack[] dropped = new ItemStack[1];
        e.run("drop_keeps_modules_and_settings", List.of(
                e::placeTerminal,
                e::installLinkModule,
                () -> {
                    e.call("left_bay_1", "set_channel", 3, DROP_FIRST, DROP_SECOND, "tx");
                    e.call("left_bay_1", "set_output", 3, 9);
                    TerminalBlockEntity be = e.terminal();
                    List<ItemStack> drops = Block.getDrops(e.state(), e.level(), e.abs(), be);
                    check(drops.size() == 1, "drops: " + drops);
                    dropped[0] = drops.get(0);
                    check(dropped[0].has(ModDataComponents.INSTALLED_MODULES.get()), "dropped terminal has no modules");
                    e.level().removeBlock(e.abs(), false);
                    return true;
                },
                () -> {
                    BlockState state = ModBlocks.TERMINAL_BLOCK.get().defaultBlockState();
                    e.level().setBlock(e.abs(), state, 3);
                    state.getBlock().setPlacedBy(e.level(), e.abs(), state, e.player, dropped[0]);
                    return true;
                },
                () -> e.hub().names().contains("left_bay_1"),
                () -> {
                    check(e.state().getValue(TerminalBlock.LEFT_BAY), "bay not restored");
                    check(e.state().getValue(TerminalBlock.SLOTS[0]) == ModuleSlotVisual.REDSTONE_LINK, "module not shown");
                    Map<?, ?> ch = (Map<?, ?>) e.call("left_bay_1", "get_channel", 3);
                    check("tx".equals(ch.get("mode")) && Integer.valueOf(9).equals(ch.get("output"))
                            && DROP_FIRST.equals(ch.get("first")) && DROP_SECOND.equals(ch.get("second")), "settings lost: " + ch);
                    return true;
                }));
    }

    /** A peripheral block next to the computer attaches under its side name and detaches when removed. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".adjacent")
    public static void adjacent_block_attaches_and_detaches(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos left = TERMINAL.west();   // terminal faces north: its left is west
        e.run("adjacent_block_attaches_and_detaches", List.of(
                e::placeTerminal,
                () -> {
                    h.setBlock(left, CreateCompat.REDSTONE_LINK_INTERFACE.get().defaultBlockState());
                    return true;
                },
                () -> e.hub().names().contains("left"),
                () -> {
                    check("redstone_link".equals(e.hub().get("left").getType()), "wrong type on left");
                    check(e.hub().names().size() == 1, "extra attachments: " + e.hub().names());
                    h.setBlock(left, Blocks.AIR.defaultBlockState());
                    return true;
                },
                () -> !e.hub().names().contains("left")));
    }

    /** A computer's tx channel powers a real Create Redstone Link receiver, and turning it off un-powers it. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".tx")
    public static void computer_transmits_to_create_receiver(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos link = new BlockPos(1, 2, 1);
        e.run("computer_transmits_to_create_receiver", List.of(
                e::placeTerminal,
                e::installLinkModule,
                () -> e.placeCreateLink(link, true),
                () -> e.setLinkFrequency(link, TX_FREQ),
                () -> {
                    e.call("left_bay_1", "set_channel", 0, TX_FREQ.first(), TX_FREQ.second(), "tx");
                    e.call("left_bay_1", "set_output", 0, 15);
                    return true;
                },
                () -> e.linkReceived(link) == 15 && h.getBlockState(link).getValue(RedstoneLinkBlock.POWERED),
                () -> {
                    e.call("left_bay_1", "set_output", 0, 0);
                    return true;
                },
                () -> e.linkReceived(link) == 0 && !h.getBlockState(link).getValue(RedstoneLinkBlock.POWERED)));
    }

    /** A real Create transmitter reaches the computer's rx channel; a channel on another frequency stays at 0. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".rx")
    public static void create_transmitter_reaches_computer(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos link = new BlockPos(1, 2, 1);
        e.run("create_transmitter_reaches_computer", List.of(
                e::placeTerminal,
                e::installLinkModule,
                () -> {
                    e.call("left_bay_1", "set_channel", 1, RX_FREQ.first(), RX_FREQ.second(), "rx");
                    e.call("left_bay_1", "set_channel", 2, RX_FREQ.first(), "minecraft:gold_ingot", "rx");
                    return e.placeCreateLink(link, false);
                },
                () -> e.setLinkFrequency(link, RX_FREQ),
                () -> {
                    h.setBlock(link.east(), Blocks.REDSTONE_BLOCK.defaultBlockState());
                    return true;
                },
                () -> Integer.valueOf(15).equals(e.call("left_bay_1", "get_input", 1)),
                () -> {
                    check(Integer.valueOf(0).equals(e.call("left_bay_1", "get_input", 2)), "other frequency received power");
                    h.setBlock(link.east(), Blocks.AIR.defaultBlockState());
                    return true;
                },
                () -> Integer.valueOf(0).equals(e.call("left_bay_1", "get_input", 1))));
    }

    /**
     * End to end on a booted computer: a Python program finds the module,
     * configures a channel, gets a PeripheralError for a bad argument, and
     * receives a redstone_link event when a Create transmitter powers up.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".python")
    public static void python_program_uses_redstone_link(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos link = new BlockPos(1, 2, 1);
        String script = String.join("\n",
                "import peripheral",
                "l = peripheral.find('redstone_link')",
                "print('FOUND', l.type, l.name, peripheral.names())",
                "l.set_channel(1, '" + PY_FREQ.first() + "', '" + PY_FREQ.second() + "', 'rx')",
                "print('MODE', l.get_channel(1)['mode'])",
                "try:",
                "    l.set_output(99, 1)",
                "except peripheral.PeripheralError as err:",
                "    print('ERR', err)",
                "print('READY')",
                "ev = peripheral.pull_event('redstone_link', timeout=30)",
                "print('EVENT', ev)",
                "print('INPUT', l.get_input(1))",
                "print('DONE')",
                "");
        e.run("python_program_uses_redstone_link", List.of(
                e::placeTerminal,
                e::installLinkModule,
                () -> {
                    e.terminal().initializeWasm();
                    return true;
                },
                () -> e.screen().contains("Welcome to Terminal OS"),
                () -> {
                    e.terminal().onStringInput("peripherals\n");   // the shell command
                    return true;
                },
                () -> java.util.regex.Pattern.compile("left_bay_1 +redstone_link").matcher(e.screen()).find(),
                () -> {
                    Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(e.terminal());
                    try {
                        Files.createDirectories(dir);
                        Files.writeString(dir.resolve("periph_test.py"), script);
                    } catch (java.io.IOException ex) {
                        throw new AssertionError("can't write the script: " + ex);
                    }
                    e.terminal().onStringInput("python periph_test.py\n");
                    return true;
                },
                () -> e.screen().contains("READY"),
                () -> {
                    String s = e.screen();
                    check(s.contains("FOUND redstone_link left_bay_1"), "find() failed:\n" + s);
                    check(s.contains("MODE rx"), "set_channel/get_channel failed:\n" + s);
                    check(s.contains("ERR channel must be 0-63, got 99"), "no PeripheralError:\n" + s);
                    return e.placeCreateLink(link, false);
                },
                () -> e.setLinkFrequency(link, PY_FREQ),
                () -> {
                    h.setBlock(link.east(), Blocks.REDSTONE_BLOCK.defaultBlockState());
                    return true;
                },
                () -> e.screen().contains("DONE"),
                () -> {
                    String s = e.screen();
                    check(s.contains("EVENT ('redstone_link', 'left_bay_1', 1, 15, 0)"), "no event:\n" + s);
                    check(s.contains("INPUT 15"), "input not 15:\n" + s);
                    return true;
                }));
    }

    // ------------------------------------------------------------ plumbing

    private static void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }

    /** One test's world and a step list: each step runs every tick until it returns true. */
    private static final class Env {
        final GameTestHelper h;
        final Player player;
        private int step;
        private long started;
        private String failure;

        Env(GameTestHelper h) {
            this.h = h;
            this.player = h.makeMockPlayer(GameType.SURVIVAL);
        }

        void run(String name, List<BooleanSupplier> steps) {
            List<BooleanSupplier> all = new ArrayList<>(steps);
            TestDriver.drive(h, NS, name, () -> {
                if (started == 0) started = System.currentTimeMillis();
                if (System.currentTimeMillis() - started > WALL_LIMIT_MS) {
                    failure = "timed out at step " + (step + 1) + "/" + all.size() + " " + linkWait + "\n" + screen();
                    return false;
                }
                try {
                    while (step < all.size() && all.get(step).getAsBoolean()) step++;
                } catch (AssertionError | RuntimeException ex) {
                    failure = "step " + (step + 1) + ": " + ex.getMessage();
                    return false;
                }
                return step >= all.size();
            }, () -> failure);
        }

        ServerLevel level() {
            return h.getLevel();
        }

        BlockPos abs() {
            return h.absolutePos(TERMINAL);
        }

        BlockState state() {
            return h.getBlockState(TERMINAL);
        }

        TerminalBlockEntity terminal() {
            return (TerminalBlockEntity) h.getBlockEntity(TERMINAL);
        }

        ModuleBays bays() {
            return terminal().getModuleBays();
        }

        PeripheralHub hub() {
            return terminal().getPeripheralHub();
        }

        String screen() {
            TerminalBlockEntity t = h.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity te ? te : null;
            return t == null ? "" : ScenarioRun.screen(t.getDisplay());
        }

        boolean placeTerminal() {
            h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.NORTH));
            return true;
        }

        /** Card in the left bay, Redstone Link module in left_bay_1; true once attached. */
        boolean installLinkModule() {
            if (e().bays().cardCount() == 0) {
                hold(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get()));
                useOn(Direction.WEST, 0.5);
                hold(CreateCompat.REDSTONE_LINK_MODULE.get().getDefaultInstance());
                useOn(Direction.WEST, 0.75);
            }
            return hub().names().contains("left_bay_1");
        }

        private Env e() {
            return this;
        }

        ItemStack hold(ItemStack stack) {
            player.setItemInHand(InteractionHand.MAIN_HAND, stack);
            return stack;
        }

        int count(net.minecraft.world.item.Item item) {
            int n = 0;
            for (ItemStack s : player.getInventory().items) if (s.is(item)) n += s.getCount();
            return n;
        }

        private BlockHitResult hit(Direction face, double y) {
            Vec3 c = Vec3.atCenterOf(abs());
            Vec3 at = new Vec3(c.x + face.getStepX() * 0.5, abs().getY() + y, c.z + face.getStepZ() * 0.5);
            return new BlockHitResult(at, face, abs(), false);
        }

        /** Right-click with the held item, the way the server does it. */
        void useOn(Direction face, double y) {
            ItemStack held = player.getMainHandItem();
            var result = state().useItemOn(held, level(), player, InteractionHand.MAIN_HAND, hit(face, y));
            if (result == net.minecraft.world.ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION) {
                state().useWithoutItem(level(), player, hit(face, y));
            }
        }

        /** Right-click with an empty hand (sneak state as set on the player). */
        void useEmpty(Direction face, double y) {
            state().useWithoutItem(level(), player, hit(face, y));
        }

        /** Call a peripheral method through the hub, as a program would; errors throw. */
        Object call(String name, String method, Object... args) {
            byte[] frame = hub().call(name, method, PeripheralValues.encode(List.of(args)), level().getServer());
            try {
                Object v = PeripheralValues.decode(java.util.Arrays.copyOfRange(frame, 1, frame.length));
                if (frame[0] != PeripheralValues.STATUS_OK) throw new AssertionError(method + " failed: " + v);
                return v;
            } catch (PeripheralValues.DecodeException ex) {
                throw new AssertionError("bad frame from " + method + ": " + ex.getMessage());
            }
        }

        private boolean wantReceiver;

        /**
         * A Create Redstone Link standing on stone. It is placed as a
         * transmitter, like a player places one: Create only attaches the link
         * behaviour of a freshly placed block through the transmitter's first
         * tick; {@link #setLinkFrequency} then toggles it to receiver mode.
         */
        boolean placeCreateLink(BlockPos pos, boolean receiver) {
            wantReceiver = receiver;
            h.setBlock(pos.below(), Blocks.STONE.defaultBlockState());
            h.setBlock(pos, AllBlocks.REDSTONE_LINK.getDefaultState()
                    .setValue(DirectionalBlock.FACING, Direction.UP)
                    .setValue(RedstoneLinkBlock.RECEIVER, false));
            return true;
        }

        private int linkTicks;
        String linkWait = "";

        /** Set the link's frequency slots once Create has initialised its behaviour (a couple of ticks). */
        boolean setLinkFrequency(BlockPos pos, Freq freq) {
            BlockState st = h.getBlockState(pos);
            var be = h.getBlockEntity(pos);
            LinkBehaviour b = be == null ? null : BlockEntityBehaviour.get(be, LinkBehaviour.TYPE);
            linkWait = "link " + st + " be=" + be + " behaviour=" + b;
            if (b == null || ++linkTicks < 3) return false;
            if (wantReceiver != st.getValue(RedstoneLinkBlock.RECEIVER)) {
                h.setBlock(pos, st.setValue(RedstoneLinkBlock.RECEIVER, wantReceiver)); // the sneak-click toggle
                linkTicks = 0;
                return false;
            }
            if (b.isListening() != wantReceiver) return false; // Create swaps the behaviour on its next tick
            b.setFrequency(true, freq.firstStack());
            b.setFrequency(false, freq.secondStack());
            linkTicks = 0;
            return true;
        }

        int linkReceived(BlockPos pos) {
            return h.getBlockEntity(pos) instanceof RedstoneLinkBlockEntity be ? be.getReceivedSignal() : -1;
        }
    }

    private PeripheralTests() {}
}
//?}
