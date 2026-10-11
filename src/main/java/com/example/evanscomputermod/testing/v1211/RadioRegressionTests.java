package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaAnalyzerItem;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.controller.ControllerRadio;
import com.example.evanscomputermod.radio.controller.ControllerReceiverModule;
import com.example.evanscomputermod.radio.controller.RadioControllerContent;
import com.example.evanscomputermod.radio.hazard.RadioOwners;
import com.example.evanscomputermod.radio.medium.WorldMediumContent;
import com.example.evanscomputermod.radio.microwave.MicrowaveContent;
import com.example.evanscomputermod.radio.microwave.dish.DishBlock;
import com.example.evanscomputermod.radio.power.RadioPowerContent;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import com.example.evanscomputermod.testing.scenario.ScenarioPlayer;
import com.example.evanscomputermod.testing.scenario.WifiScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.phys.AABB;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.function.BooleanSupplier;

/**
 * Regression tests for the radio bug list (namespace {@code ecm_radio}): recipes, the
 * controller's polarization, a monopole over metal, the analyzer's truncation note, dish
 * drops, the Interface Block next to an Access Point, the Access Point's reset, load
 * validation and kick, a disabled Burner Generator, tuner burnout events, feed point owner
 * records, the RF world's chunk cache and 5 GHz Wi-Fi with keep-alives.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioRegressionTests {
    private static final String NS = RadioTests.NS;
    private static final String STRUCTURE = RadioTests.STRUCTURE;

    private RadioRegressionTests() {}

    static void check(boolean ok, String[] failure, String why) {
        if (!ok && failure[0] == null) failure[0] = why;
    }

    // ------------------------------------------------------------ 1: recipes

    /** Every radio item can be crafted in survival: the Handheld Radio and Controller Receiver Module included. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_recipes")
    public static void radio_items_have_recipes(GameTestHelper h) {
        String[] items = {"handheld_radio", "controller_receiver_module", "wireless_controller", "wifi_module", "access_point",
                "sdr_basic", "sdr_standard", "sdr_advanced", "antenna_analyzer", "rf_meter", "rf_wrench", "antenna_tuner",
                "amplifier_100w", "amplifier_1kw", "amplifier_10kw", "microwave_radio", "dish_small", "dish_medium", "dish_large",
                "copper_wire", "antenna_wire", "heavy_cable", "antenna_rod", "lattice_mast", "insulator", "feed_point",
                "coax_cable", "hardline", "lightning_arrestor"};
        var server = h.getLevel().getServer();
        List<String> missing = new ArrayList<>();
        for (String id : items) {
            var item = BuiltInRegistries.ITEM.get(EvansComputerMod.id(id));
            boolean found = false;
            for (var r : server.getRecipeManager().getRecipes()) {
                try {
                    if (r.value().getResultItem(server.registryAccess()).is(item)) { found = true; break; }
                } catch (RuntimeException ignored) {}
            }
            if (!found) missing.add(id);
        }
        String[] failure = {missing.isEmpty() ? null : "no recipe for " + missing};
        TestDriver.drive(h, NS, "radio_items_have_recipes", () -> failure[0] == null, () -> failure[0]);
    }

    // ------------------------------------------------------------ 20: Burner Generator disabled

    /**
     * With the Burner Generator disabled in the server config: a generator holding FE pushes none
     * of it into an amplifier next to it and its capability gives nothing; the server-start hook
     * removes its recipe (the condition alone can't see the server config on a world's first load).
     * Control: enabled again, the same generator charges the amplifier.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_burner")
    public static void disabled_burner_generator_delivers_nothing(GameTestHelper h) {
        BlockPos gen = new BlockPos(3, 2, 3), amp = new BlockPos(4, 2, 3);
        h.setBlock(gen, RadioPowerContent.BURNER_GENERATOR.get().defaultBlockState());
        h.setBlock(amp, com.example.evanscomputermod.radio.amp.RadioAmpContent.AMPLIFIER_100W.get().defaultBlockState());
        var be = (com.example.evanscomputermod.radio.power.BurnerGeneratorBlockEntity) h.getBlockEntity(gen);
        ((com.example.evanscomputermod.energy.RadioEnergyStorage) be.energy()).generate(20_000);
        var server = h.getLevel().getServer();
        String[] failure = {null};
        int[] step = {0}, ticks = {0};
        RadioConfig.overrideBurnerGeneratorEnabled(false);
        var removed = RadioPowerContent.applyRecipeConfig(server);
        check(!removed.isEmpty(), failure, "the burner generator's recipe wasn't removed while disabled");
        var cap = h.getLevel().getCapability(Capabilities.EnergyStorage.BLOCK, h.absolutePos(gen), Direction.EAST);
        check(cap != null && cap.extractEnergy(1000, true) == 0 && !cap.canExtract(), failure, "a disabled generator's capability gives FE");
        TestDriver.drive(h, NS, "disabled_burner_generator_delivers_nothing", () -> {
            var ampCap = h.getLevel().getCapability(Capabilities.EnergyStorage.BLOCK, h.absolutePos(amp), null);
            int fe = ampCap == null ? -1 : ampCap.getEnergyStored();
            if (step[0] == 0) {
                if (++ticks[0] < 20) return false;
                if (fe != 0) failure[0] = "disabled generator pushed " + fe + " FE into the amplifier";
                RadioConfig.overrideBurnerGeneratorEnabled(null);
                var all = new ArrayList<>(server.getRecipeManager().getRecipes());
                all.addAll(removed);
                server.getRecipeManager().replaceRecipes(all);
                step[0] = 1;
                ticks[0] = 0;
                return false;
            }
            if (fe > 0) return true;   // control: enabled, it charges the amplifier
            if (++ticks[0] > 40) failure[0] = "control: enabled generator didn't charge the amplifier";
            return false;
        }, () -> {
            if (failure[0] != null) RadioConfig.overrideBurnerGeneratorEnabled(null);
            return failure[0];
        });
    }

    // ------------------------------------------------------------ 2: controller polarization

    /**
     * A controller 5 blocks from a Controller Receiver module arrives at about the free-space level
     * (-54 dBm: 0 dBm, -2 dBi controller, vertical dipole, 54 dB at 2.44 GHz): co-polarized, not
     * 20 dB down as a horizontal controller against the vertical receiver was.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_ctrl_pol")
    public static void controller_is_co_polarized_with_receiver(GameTestHelper h) {
        BlockPos pcPos = new BlockPos(10, 2, 10);
        var terminalState = com.example.evanscomputermod.block.ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                .setValue(com.example.evanscomputermod.block.TerminalBlock.FACING, Direction.NORTH);
        h.setBlock(pcPos, terminalState);
        var player = h.makeMockPlayer(GameType.CREATIVE);
        String dim = h.getLevel().dimension().location().toString();
        UUID ctrl = UUID.randomUUID();
        var pressed = new com.example.evanscomputermod.controller.ControllerState(0, 0, 0, 0, 0, 0, 0);
        double[] best = {Double.NEGATIVE_INFINITY};
        String[] failure = {null};
        int[] step = {0}, waited = {0}, sends = {0};
        TestDriver.drive(h, NS, "controller_is_co_polarized_with_receiver", () -> {
            var pc = (com.example.evanscomputermod.block.TerminalBlockEntity) h.getBlockEntity(pcPos);
            switch (step[0]) {
                case 0 -> {
                    useBay(h, player, pcPos, new ItemStack(com.example.evanscomputermod.item.ModItems.MODULE_EXPANSION_CARD.get()), 0.5);
                    useBay(h, player, pcPos, new ItemStack(RadioControllerContent.CONTROLLER_RECEIVER_MODULE.get()), 0.75);
                    step[0] = 1;
                }
                case 1 -> {
                    if (ControllerRadio.receiverOf(pc) != null) step[0] = 2;
                    else if (++waited[0] > 100) failure[0] = "receiver module not installed";
                }
                default -> {
                    var rx = (ControllerReceiverModule) ControllerRadio.receiverOf(pc);
                    Object rssi = rx.stats().get("last_rssi_dbm");
                    if (rssi instanceof Double d) best[0] = Math.max(best[0], d);
                    if (sends[0] >= 10) {
                        ControllerRadio.stop(ctrl);
                        EvansComputerMod.LOGGER.info("[ecm_radio] controller 5 blocks away: best {} dBm", best[0]);
                        if (best[0] < -64) failure[0] = "controller heard at " + best[0] + " dBm (expected about -54: cross-polarized?)";
                        return failure[0] == null;
                    }
                    var at = h.absolutePos(pcPos);
                    ControllerRadio.send(UUID.randomUUID(), dim, at.getX() + 5.5, at.getY() + 0.5, at.getZ() + 0.5, 0, ctrl, pressed, pc);
                    sends[0]++;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    /** Click the terminal's left (west) bay with {@code stack}, like a player installing a card or module. */
    static void useBay(GameTestHelper h, Player player, BlockPos rel, ItemStack stack, double y) {
        player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, stack);
        BlockPos p = h.absolutePos(rel);
        var face = Direction.WEST;
        var hit = new net.minecraft.world.phys.BlockHitResult(new net.minecraft.world.phys.Vec3(
                p.getX() + 0.5 + face.getStepX() * 0.5, p.getY() + y, p.getZ() + 0.5 + face.getStepZ() * 0.5), face, p, false);
        var state = h.getLevel().getBlockState(p);
        var result = state.useItemOn(stack, h.getLevel(), player, net.minecraft.world.InteractionHand.MAIN_HAND, hit);
        if (result == net.minecraft.world.ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION)
            state.useWithoutItem(h.getLevel(), player, hit);
    }

    // ------------------------------------------------------------ 5: monopole over metal

    /**
     * A vertical feed point on an iron block with 6 copper wire blocks above it: the iron block is
     * the ground plane (a monopole over metal, ~quarter-wave resonance near 11 MHz), not a lower
     * arm. Control: the same over grass is a monopole too; iron bars under the feed still join.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_monopole")
    public static void monopole_over_metal_uses_it_as_ground(GameTestHelper h) {
        BlockPos metalFeed = new BlockPos(5, 3, 5), grassFeed = new BlockPos(20, 3, 5), barsFeed = new BlockPos(35, 4, 5);
        h.setBlock(metalFeed.below(), Blocks.IRON_BLOCK);
        h.setBlock(grassFeed.below(), Blocks.GRASS_BLOCK);
        h.setBlock(barsFeed.below(2), Blocks.STONE);
        h.setBlock(barsFeed.below(), Blocks.IRON_BARS);
        for (BlockPos f : List.of(metalFeed, grassFeed, barsFeed)) vertical(h, f, 6);
        String[] failure = {null};
        RadioAntennaTests.steps(h, "monopole_over_metal_uses_it_as_ground", failure, List.of(
                () -> antenna(h, metalFeed).solved() && antenna(h, grassFeed).solved() && antenna(h, barsFeed).solved(),
                () -> {
                    Antenna metal = antenna(h, metalFeed), grass = antenna(h, grassFeed), bars = antenna(h, barsFeed);
                    EvansComputerMod.LOGGER.info("[ecm_radio] monopole over iron: {} / {}", metal.summary(), metal.details());
                    EvansComputerMod.LOGGER.info("[ecm_radio] monopole over grass: {} / {}", grass.summary(), grass.details());
                    check(!connected(h, metalFeed, Direction.DOWN), failure, "the feed's lower lug joined the iron block");
                    check(metal.graph().monopole, failure, "over iron it isn't fed as a monopole: " + metal.details());
                    check(metal.graph().groundName.startsWith("metal"), failure, "ground: " + metal.graph().groundName);
                    check(metal.graph().blockCount == 7, failure, "graph blocks " + metal.graph().blockCount + " (iron joined?)");
                    double f = metal.resonantHz();
                    check(f > 8e6 && f < 14e6, failure, "monopole over iron resonates at " + f);
                    check(Math.abs(f - grass.resonantHz()) < 0.15 * grass.resonantHz(), failure,
                            "iron vs grass resonance " + f + " vs " + grass.resonantHz());
                    check(metal.swrAt(f) < 3, failure, "SWR " + metal.swrAt(f));
                    check(connected(h, barsFeed, Direction.DOWN) && !bars.graph().monopole, failure,
                            "control: iron bars under the feed should still join as a lower element");
                    return true;
                }));
    }

    static void vertical(GameTestHelper h, BlockPos feed, int n) {
        RadioAntennaTests.placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.Y), feed);
        for (int i = 1; i <= n; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feed.above(i));
        RadioAntennaTests.placeConnected(h, RadioAntennaContent.INSULATOR.get().defaultBlockState(), feed.above(n + 1));
    }

    static Antenna antenna(GameTestHelper h, BlockPos feed) {
        return AntennaManager.get(h.getLevel(), h.absolutePos(feed));
    }

    static boolean connected(GameTestHelper h, BlockPos rel, Direction side) {
        var s = h.getBlockState(rel);
        return s.getBlock() instanceof ConductorBlock && s.getValue(ConductorBlock.property(side));
    }

    // ------------------------------------------------------------ 21: analyzer text

    /**
     * An antenna touching more metal blocks than the walker follows says so (analyzer details and
     * the {@code antenna} program's text); a pending antenna's line says "solving" once.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_analyzer")
    public static void analyzer_reports_truncation_and_solving_once(GameTestHelper h) {
        BlockPos feed = new BlockPos(5, 4, 20);
        RadioAntennaTests.placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X), feed);
        for (int i = 1; i <= 3; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feed.east(i));
        for (int i = 1; i <= 3; i++) RadioAntennaTests.placeConnected(h, RadioAntennaContent.COPPER_WIRE.get().defaultBlockState(), feed.west(i));
        // A 9 x 9 slab of iron blocks (81) joined to the east arm's end: more than MAX_FOREIGN (64).
        for (int dx = 0; dx < 9; dx++)
            for (int dz = -4; dz <= 4; dz++) h.setBlock(feed.east(4 + dx).south(dz), Blocks.IRON_BLOCK);
        String[] failure = {null};
        boolean[] sawPending = {false};
        RadioAntennaTests.steps(h, "analyzer_reports_truncation_and_solving_once", failure, List.of(
                () -> {
                    Antenna a = antenna(h, feed);
                    if (a.pending()) {
                        sawPending[0] = true;
                        String line = AntennaAnalyzerItem.reportLines(h.getLevel(), a).get(0);
                        int n = line.split("solving", -1).length - 1;
                        check(n == 1, failure, "pending line says 'solving' " + n + " times: " + line);
                    }
                    return a.present() && a.graph() != null;
                },
                () -> {
                    Antenna a = antenna(h, feed);
                    String details = a.details();
                    EvansComputerMod.LOGGER.info("[ecm_radio] truncated antenna: {} / {}", a.summary(), details);
                    check(a.graph().truncated, failure, "the walk wasn't truncated");
                    check(details.contains("only part of the antenna was analysed") && details.contains("64 touching metal blocks"),
                            failure, "truncation not reported: " + details);
                    return true;
                }));
    }

    // ------------------------------------------------------------ 27: dish drops

    /**
     * Breaking a non-controller part of a dish follows vanilla's rules: with a pickaxe in survival
     * one dish item drops; bare-handed in survival nothing (the dish needs a pickaxe); in creative
     * nothing. The whole dish goes each time.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_dish")
    public static void dish_part_break_follows_tool_rules(GameTestHelper h) {
        DishBlock dish = MicrowaveContent.DISH_MEDIUM.get();
        BlockPos[] at = {new BlockPos(5, 2, 30), new BlockPos(15, 2, 30), new BlockPos(25, 2, 30)};
        for (BlockPos p : at) dish.place(h.getLevel(), h.absolutePos(p), Direction.NORTH);
        ScenarioPlayer sp = new ScenarioPlayer(h.getLevel());
        var fp = sp.entity();
        Object[][] cases = {{GameType.SURVIVAL, new ItemStack(Items.IRON_PICKAXE), 1}, {GameType.SURVIVAL, ItemStack.EMPTY, 0},
                {GameType.CREATIVE, new ItemStack(Items.IRON_PICKAXE), 0}};
        String[] failure = {null};
        for (int i = 0; i < 3; i++) {
            BlockPos ctrl = h.absolutePos(at[i]);
            BlockPos part = null;
            for (int k = 0; k < 4 && part == null; k++) {
                BlockPos q = DishBlock.partPos(com.example.evanscomputermod.radio.microwave.dish.DishSize.MEDIUM, ctrl, Direction.NORTH, k);
                if (!q.equals(ctrl)) part = q;
            }
            fp.setGameMode((GameType) cases[i][0]);
            fp.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, ((ItemStack) cases[i][1]).copy());
            try {
                sp.breakBlock(part, Direction.NORTH);
            } catch (RuntimeException e) {
                check(false, failure, "case " + i + ": " + e);
            }
        }
        int[] ticks = {0};
        TestDriver.drive(h, NS, "dish_part_break_follows_tool_rules", () -> {
            if (++ticks[0] < 10) return false;
            for (int i = 0; i < 3; i++) {
                BlockPos c = h.absolutePos(at[i]);
                int drops = 0;
                for (ItemEntity e : h.getLevel().getEntitiesOfClass(ItemEntity.class, new AABB(c).inflate(3)))
                    if (e.getItem().is(MicrowaveContent.DISH_MEDIUM_ITEM.get())) drops += e.getItem().getCount();
                check(drops == (int) cases[i][2], failure, "case " + i + " (" + cases[i][0] + ", " + cases[i][1] + "): " + drops + " dish item(s) dropped");
                check(!(h.getLevel().getBlockState(c).getBlock() instanceof DishBlock), failure, "case " + i + ": the dish's controller is still there");
            }
            return true;
        }, () -> failure[0]);
    }

    // ------------------------------------------------------------ 24: Interface Block ↔ Access Point

    /**
     * An Access Point placed against an Interface Block on a computer: the Interface Block draws its
     * arm to the AP and the AP's port joins that face's segment (the AP lights as cabled).
     * Control: an AP standing alone stays uncabled.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_interface")
    public static void interface_block_connects_to_access_point(GameTestHelper h) {
        BlockPos pc = new BlockPos(10, 2, 10), iface = pc.east(), ap = iface.east(), lone = new BlockPos(30, 2, 10);
        ScenarioPlayer sp = new ScenarioPlayer(h.getLevel());
        h.setBlock(pc.below(), Blocks.STONE);
        h.setBlock(iface.below(), Blocks.STONE);
        h.setBlock(ap.below(), Blocks.STONE);
        h.setBlock(lone.below(), Blocks.STONE);
        sp.place(com.example.evanscomputermod.block.ModBlocks.TERMINAL_BLOCK.get(), h.absolutePos(pc), Direction.UP);
        sp.place(com.example.evanscomputermod.block.ModBlocks.INTERFACE_BLOCK.get(), h.absolutePos(iface), Direction.UP);
        sp.place(AccessPointContent.ACCESS_POINT.get(), h.absolutePos(ap), Direction.UP);
        sp.place(AccessPointContent.ACCESS_POINT.get(), h.absolutePos(lone), Direction.UP);
        var facing = h.getBlockState(pc).getValue(com.example.evanscomputermod.block.TerminalBlock.FACING);
        sp.openScreen(h.absolutePos(pc), facing);   // boots it: the computer discovers its faces
        String[] failure = {null};
        long start = System.currentTimeMillis();
        TestDriver.drive(h, NS, "interface_block_connects_to_access_point", () -> {
            var s = h.getBlockState(iface);
            boolean arm = s.getValue(com.example.evanscomputermod.block.InterfaceBlock.EAST);
            var be = (AccessPointBlockEntity) h.getBlockEntity(ap);
            var loneBe = (AccessPointBlockEntity) h.getBlockEntity(lone);
            if (arm && be.cabled() && be.directNic() != null) {
                check(!loneBe.cabled(), failure, "control: an AP touching nothing is cabled");
                return failure[0] == null;
            }
            if (System.currentTimeMillis() - start > 30_000)
                failure[0] = "interface arm " + arm + ", AP cabled " + be.cabled() + ", direct NIC " + (be.directNic() != null);
            return false;
        }, () -> failure[0]);
    }

    // ------------------------------------------------------------ 15: Access Point permissions, load, kick

    /**
     * The wrench reset follows the screen's rule: a stranger is refused (nothing changes), the
     * owner resets and stays the owner. Switching to an open network erases the passphrase. Saved
     * settings that don't validate are reset when the AP is loaded into a level. Kicking works with
     * garbage in the MAC filter box; applying with it is refused.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_ap")
    public static void access_point_reset_load_and_kick(GameTestHelper h) {
        BlockPos rel = new BlockPos(4, 2, 12), loaded = new BlockPos(8, 2, 12);
        h.setBlock(rel, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
        var be = (AccessPointBlockEntity) h.getBlockEntity(rel);
        Player owner = h.makeMockPlayer(GameType.SURVIVAL), stranger = h.makeMockPlayer(GameType.SURVIVAL);
        be.claim(owner);
        String[] failure = {null};
        var wpa = new ApSettings("reg-ap", false, Security.WPA2_PSK, 6, 20, false, null, null);
        check(be.applySettings(wpa, "secret-pass-1") == null, failure, "settings not applied");
        check(!be.factoryReset(stranger), failure, "a stranger's wrench reset was accepted");
        check(be.settings().security() == Security.WPA2_PSK && be.hasPassphrase() && owner.getUUID().equals(be.owner()), failure,
                "a refused reset changed something: " + be.settings() + " owner " + be.owner());
        check(be.factoryReset(owner) && be.settings().security() == Security.OPEN && owner.getUUID().equals(be.owner()), failure,
                "owner reset: " + be.settings() + " owner " + be.owner());
        check(be.applySettings(wpa, "secret-pass-1") == null, failure, "settings not re-applied");
        check(be.applySettings(new ApSettings("reg-ap", false, Security.OPEN, 6, 20, false, null, null), "") == null && !be.hasPassphrase(),
                failure, "an open network kept the WPA2 passphrase");
        // Kick with a half-typed filter box.
        var view = be.view(null);
        var kick = ApPackets.fromForm(view, ApPackets.Action.KICK, "x", false, Security.OPEN, 6, 20, false, null, "zz:not-a-mac", "", 0x0211_2233_4455L);
        check(kick.action() == ApPackets.Action.KICK && kick.kickMac() == 0x0211_2233_4455L, failure, "kick packet not built");
        boolean refused = false;
        try {
            ApPackets.fromForm(view, ApPackets.Action.APPLY, "x", false, Security.OPEN, 6, 20, false, null, "zz:not-a-mac", "", 0);
        } catch (IllegalArgumentException e) {
            refused = true;
        }
        check(refused, failure, "control: applying with a bad MAC list was accepted");
        // A save with settings that no longer validate (SSID too long) loading as a chunk does: no level yet.
        CompoundTag bad = be.saveWithFullMetadata(h.getLevel().registryAccess());
        bad.putString("Ssid", "this-ssid-is-much-longer-than-thirty-two-bytes");
        bad.putString("Security", "WPA2_PSK");
        BlockPos abs = h.absolutePos(loaded);
        h.setBlock(loaded, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
        BlockEntity fresh = BlockEntity.loadStatic(abs, h.getBlockState(loaded), bad, h.getLevel().registryAccess());
        check(fresh instanceof AccessPointBlockEntity, failure, "loadStatic gave " + fresh);
        if (fresh != null) h.getLevel().setBlockEntity(fresh);
        int[] ticks = {0};
        TestDriver.drive(h, NS, "access_point_reset_load_and_kick", () -> {
            if (failure[0] != null) return false;
            if (++ticks[0] < 5) return false;
            var l = (AccessPointBlockEntity) h.getBlockEntity(loaded);
            check(!l.settings().ssid().startsWith("this-ssid"), failure, "invalid SSID kept: " + l.settings().ssid());
            return true;
        }, () -> failure[0]);
    }

    // ------------------------------------------------------------ 28: tuner burnout event, owner records

    /**
     * A tuner reaching burnout posts an AntennaOverloadEvent first (cancelling it spares the tuner,
     * as for every other chain failure); a feed point's owner record goes when the feed point does.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_tuner")
    public static void tuner_burnout_posts_overload_and_owner_records_clear(GameTestHelper h) {
        BlockPos tuner = new BlockPos(4, 2, 14), feed = new BlockPos(8, 2, 14);
        h.setBlock(tuner, com.example.evanscomputermod.radio.amp.RadioAmpContent.ANTENNA_TUNER.get().defaultBlockState());
        var be = h.getBlockEntity(tuner);
        String[] failure = {null};
        int[] events = {0};
        BlockPos absTuner = h.absolutePos(tuner);
        java.util.function.Consumer<com.example.evanscomputermod.radio.api.event.AntennaOverloadEvent> listener = e -> {
            if (e.weakestLink() != null && e.weakestLink().equals(absTuner)) {
                events[0]++;
                e.setCanceled(true);
            }
        };
        NeoForge.EVENT_BUS.addListener(listener);
        try {
            var f = be.getClass().getDeclaredField("theta");
            f.setAccessible(true);
            f.setDouble(be, 1.5);
        } catch (ReflectiveOperationException e) {
            failure[0] = "can't heat the tuner: " + e;
        }
        var level = h.getLevel();
        RadioAntennaTests.placeConnected(h, RadioAntennaContent.FEED_POINT.get().defaultBlockState(), feed);
        RadioOwners.set(level, h.absolutePos(feed), UUID.randomUUID());
        int before = RadioOwners.count(level);
        h.setBlock(feed, Blocks.AIR);
        check(RadioOwners.count(level) == before - 1 && RadioOwners.get(level, h.absolutePos(feed)) == null, failure,
                "the owner record of a removed feed point is still there");
        int[] ticks = {0};
        TestDriver.drive(h, NS, "tuner_burnout_posts_overload_and_owner_records_clear", () -> {
            if (++ticks[0] < 45) return false;
            NeoForge.EVENT_BUS.unregister(listener);
            check(events[0] >= 1, failure, "no AntennaOverloadEvent for the tuner burnout");
            check(h.getBlockState(tuner).getBlock() instanceof com.example.evanscomputermod.radio.amp.TunerBlock, failure,
                    "the tuner burnt out although the event was cancelled");
            return failure[0] == null;
        }, () -> {
            if (failure[0] != null) NeoForge.EVENT_BUS.unregister(listener);
            return failure[0];
        });
    }

    // ------------------------------------------------------------ 8: RF world chunk cache

    /**
     * The medium's view of the level doesn't keep a chunk lookup from an earlier tick: a chunk that
     * wasn't loaded (read as unknown) is seen once it has loaded.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".reg_rfworld")
    public static void rf_world_sees_a_chunk_after_it_loads(GameTestHelper h) {
        var level = h.getLevel();
        var world = WorldMediumContent.rfWorld(level);
        String[] failure = {null};
        int x = h.absolutePos(BlockPos.ZERO).getX() + 40_000, z = h.absolutePos(BlockPos.ZERO).getZ() + 40_000;
        int y = level.getMinBuildHeight() + 1;   // bedrock / ground in any world type
        if (world == null) failure[0] = "no medium running";
        else if (level.getChunkSource().getChunkNow(x >> 4, z >> 4) != null) failure[0] = "the far chunk is already loaded";
        else check(world.block(x, y, z) == null, failure, "an unloaded, unsummarised chunk read as known");
        int[] step = {0};
        TestDriver.drive(h, NS, "rf_world_sees_a_chunk_after_it_loads", () -> {
            if (failure[0] != null) return false;
            switch (step[0]++) {
                case 0 -> level.getChunk(x >> 4, z >> 4);   // loads (generates) it
                case 1, 2 -> {}
                default -> {
                    var b = world.block(x, y, z);
                    check(b != null, failure, "a chunk loaded two ticks ago still reads as unknown (stale cache)");
                    return failure[0] == null;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    // ------------------------------------------------------------ 3, 25: 5 GHz Wi-Fi, keep-alive

    /**
     * {@code wifi_5ghz}: a computer joins a channel-36 Access Point with {@code wifi connect} and stays
     * associated while idle. The AP here drops clients idle for 5 s (instead of 300 s) so the 12 s of
     * silence would lose the link without the station's keep-alives.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_5ghz")
    public static void wifi_5ghz(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("wifi_5ghz"), true, run -> {
            if (run.level().getBlockEntity(run.abs(WifiScenarios.FIVE_AP)) instanceof AccessPointBlockEntity ap)
                ap.setInactivityTimeoutForTest(5_000);
            else throw new IllegalStateException("no Access Point at " + run.abs(WifiScenarios.FIVE_AP));
        });
    }
}
//?}
