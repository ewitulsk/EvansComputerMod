package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Radio &amp; Wireless, namespace {@code ecm_radio} (1.21.1). Scenario-backed
 * tests run the same definitions as {@code /ecm scenario spawn} through
 * {@link TestDriver#scenario}; this class also holds direct medium checks.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioTests {
    static final String NS = "ecm_radio";
    static final String STRUCTURE = "gametest_radio";

    /** The server's medium is running and carries a frame between two endpoints; a third out of range hears nothing (control). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".medium")
    public static void medium_delivers_frame_in_range_only(GameTestHelper h) {
        RadioMedium medium = RadioMediumHooks.medium();
        String dim = h.getLevel().dimension().location().toString();
        BlockPos o = h.absolutePos(BlockPos.ZERO);
        Channel ch = Channel.wifi24(6);
        TestEndpoint tx = new TestEndpoint(Pose.at(dim, o.getX(), o.getY() + 2, o.getZ()), ch);
        TestEndpoint near = new TestEndpoint(Pose.at(dim, o.getX() + 20, o.getY() + 2, o.getZ()), ch);
        TestEndpoint far = new TestEndpoint(Pose.at(dim, o.getX() + 200_000, o.getY() + 2, o.getZ()), ch);
        String[] failure = {null};
        if (medium == null) failure[0] = "no radio medium running";
        else {
            medium.register(tx);
            medium.register(near);
            medium.register(far);
            medium.transmit(tx, Emission.frame(ch, 20, medium.nowMicros(), 200, "DSSS-1", 1e6, new byte[] {1, 2, 3}));
            medium.unregister(tx);
            medium.unregister(near);
            medium.unregister(far);
        }
        TestDriver.drive(h, NS, "medium_delivers_frame_in_range_only", () -> {
            if (near.got.size() != 1) {
                failure[0] = "near receiver got " + near.got.size() + " frames";
                return false;
            }
            if (!far.got.isEmpty()) {
                failure[0] = "far receiver heard the frame";
                return false;
            }
            return true;
        }, () -> failure[0]);
    }

    /**
     * A bridge port joins a segment, forwards a wireless client's frame onto the
     * cable with the client's MAC, learns that MAC, and receives unicast to it;
     * a NIC on another segment hears nothing (control).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".bridge")
    public static void bridge_port_learns_client_macs(GameTestHelper h) {
        var hub = com.example.evanscomputermod.computer.NetworkHub.getInstance();
        var cables = com.example.evanscomputermod.computer.CableNetworkManager.getInstance();
        byte[] wired = {0x02, 0x7a, 1, 1, 1, 1}, port = {0x02, 0x7b, 2, 2, 2, 2},
                client = {0x02, 0x7c, 3, 3, 3, 3}, other = {0x02, 0x7d, 4, 4, 4, 4};
        List<byte[]> portGot = new CopyOnWriteArrayList<>();
        hub.registerNic(wired, (irq, p) -> {});
        hub.registerNic(other, (irq, p) -> {});
        hub.registerBridgePort(port, portGot::add);
        String link = "radio-test-bridge-" + UUID.randomUUID();
        cables.logicalLink(link, wired, port, true);
        String[] failure = {null};
        try {
            byte[] up = ethernet(new byte[] {-1, -1, -1, -1, -1, -1}, client);
            hub.transmitFromPort(port, up);
            byte[] atWired = hub.receive(wired);
            if (atWired == null || !java.util.Arrays.equals(java.util.Arrays.copyOfRange(atWired, 6, 12), client))
                failure[0] = "wired NIC didn't get the client's broadcast with the client's source MAC";
            else if (!java.util.Arrays.equals(hub.bridgePortOf(client), port))
                failure[0] = "client MAC not learned behind the port";
            else if (hub.receive(other) != null)
                failure[0] = "control: a NIC on no shared segment heard the frame";
            else {
                hub.transmit(wired, ethernet(client, wired));
                if (portGot.size() != 1 || !java.util.Arrays.equals(java.util.Arrays.copyOfRange(portGot.get(0), 0, 6), client))
                    failure[0] = "unicast to the client didn't reach the bridge port: " + portGot.size();
                hub.forgetBridged(client);
                if (failure[0] == null && hub.bridgePortOf(client) != null) failure[0] = "forgetBridged kept the entry";
            }
        } finally {
            cables.removeLogicalLink(link);
            hub.unregisterBridgePort(port);
            hub.unregisterNic(wired);
            hub.unregisterNic(other);
        }
        TestDriver.drive(h, NS, "bridge_port_learns_client_macs", () -> failure[0] == null, () -> failure[0]);
    }

    private static byte[] ethernet(byte[] dst, byte[] src) {
        byte[] f = new byte[60];
        System.arraycopy(dst, 0, f, 0, 6);
        System.arraycopy(src, 0, f, 6, 6);
        f[12] = 0x08;   // IPv4 EtherType; payload zeros
        return f;
    }

    /** Coal in a Burner Generator burns into FE and lights it; an unfuelled one makes nothing (control). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".burner")
    public static void burner_generator_burns_fuel(GameTestHelper h) {
        BlockPos fuelled = new BlockPos(2, 1, 2), empty = new BlockPos(6, 1, 2);
        var block = com.example.evanscomputermod.radio.power.RadioPowerContent.BURNER_GENERATOR.get();
        h.setBlock(fuelled, block.defaultBlockState());
        h.setBlock(empty, block.defaultBlockState());
        var be = (com.example.evanscomputermod.radio.power.BurnerGeneratorBlockEntity) h.getBlockEntity(fuelled);
        var idle = (com.example.evanscomputermod.radio.power.BurnerGeneratorBlockEntity) h.getBlockEntity(empty);
        var rest = be.fuel().insertItem(0, new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.COAL, 2), false);
        String[] failure = {null};
        if (!rest.isEmpty()) failure[0] = "coal rejected";
        if (be.fuel().insertItem(0, new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.DIRT), true).getCount() != 1)
            failure[0] = "dirt accepted as fuel";
        long[] start = {h.getTick()};
        TestDriver.drive(h, NS, "burner_generator_burns_fuel", () -> {
            if (h.getTick() - start[0] < 40) return false;
            int fe = be.energy().getEnergyStored();
            boolean lit = h.getBlockState(fuelled).getValue(com.example.evanscomputermod.radio.power.BurnerGeneratorBlock.LIT);
            if (fe < 40 * 30 || !lit) {
                failure[0] = "after 40 ticks: " + fe + " FE, lit=" + lit;
                return false;
            }
            if (be.fuel().getStackInSlot(0).getCount() != 1) {
                failure[0] = "expected one coal burned, slot has " + be.fuel().getStackInSlot(0);
                return false;
            }
            if (idle.energy().getEnergyStored() != 0
                    || h.getBlockState(empty).getValue(com.example.evanscomputermod.radio.power.BurnerGeneratorBlock.LIT)) {
                failure[0] = "control: unfuelled generator produced energy";
                return false;
            }
            return true;
        }, () -> failure[0]);
    }

    /**
     * A Wireless Controller report goes over the 2.4 GHz medium to a Controller
     * Receiver module: a player 5 blocks away connects as player 1 with the right
     * buttons; one 3 km away never connects (link budget, not a range setting);
     * a computer without a receiver module reports NO_RECEIVER (controls).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".controller")
    public static void controller_reaches_receiver_by_radio(GameTestHelper h) {
        BlockPos pcPos = new BlockPos(10, 2, 10), barePos = new BlockPos(20, 2, 10);
        for (int x = 5; x <= 25; x++) for (int z = 5; z <= 15; z++)
            h.setBlock(new BlockPos(x, 1, z), net.minecraft.world.level.block.Blocks.STONE.defaultBlockState());
        var terminalState = com.example.evanscomputermod.block.ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                .setValue(com.example.evanscomputermod.block.TerminalBlock.FACING, net.minecraft.core.Direction.NORTH);
        h.setBlock(pcPos, terminalState);
        h.setBlock(barePos, terminalState);
        var player = h.makeMockPlayer(net.minecraft.world.level.GameType.CREATIVE);
        String dim = h.getLevel().dimension().location().toString();
        UUID near = UUID.randomUUID(), far = UUID.randomUUID();
        var pressed = new com.example.evanscomputermod.controller.ControllerState(
                com.example.evanscomputermod.controller.ControllerInput.Button.values()[0].bit(), 0, 0, 0, 0, 0, 0);
        String[] failure = {null};
        int[] step = {0}, waited = {0};
        TestDriver.drive(h, NS, "controller_reaches_receiver_by_radio", () -> {
            var pc = (com.example.evanscomputermod.block.TerminalBlockEntity) h.getBlockEntity(pcPos);
            var bare = (com.example.evanscomputermod.block.TerminalBlockEntity) h.getBlockEntity(barePos);
            switch (step[0]) {
                case 0 -> {
                    useBay(h, player, pcPos, new net.minecraft.world.item.ItemStack(com.example.evanscomputermod.item.ModItems.MODULE_EXPANSION_CARD.get()), 0.5);
                    useBay(h, player, pcPos, new net.minecraft.world.item.ItemStack(
                            com.example.evanscomputermod.radio.controller.RadioControllerContent.CONTROLLER_RECEIVER_MODULE.get()), 0.75);
                    step[0] = 1;
                }
                case 1 -> {   // modules load (and register with the medium) on a later tick
                    if (com.example.evanscomputermod.radio.controller.ControllerRadio.receiverOf(pc) != null) step[0] = 2;
                    else if (++waited[0] > 100) failure[0] = "receiver module not installed: " + pc.getPeripheralHub().names();
                }
                case 2 -> {
                    var nearPos = Vec3c.of(h.absolutePos(pcPos)).add(5, 0, 0);
                    UUID who = UUID.randomUUID();
                    var r = com.example.evanscomputermod.radio.controller.ControllerRadio.send(who, dim, nearPos.x, nearPos.y, nearPos.z, 0, near, pressed, pc);
                    var r2 = com.example.evanscomputermod.radio.controller.ControllerRadio.send(who, dim, nearPos.x + 3000, nearPos.y, nearPos.z, 0, far, pressed, pc);
                    var r3 = com.example.evanscomputermod.radio.controller.ControllerRadio.send(who, dim, nearPos.x, nearPos.y, nearPos.z, 0, far, pressed, bare);
                    if (r != com.example.evanscomputermod.radio.controller.ControllerRadio.Result.SENT
                            || r2 != com.example.evanscomputermod.radio.controller.ControllerRadio.Result.SENT) {
                        failure[0] = "send results " + r + " / " + r2;
                        return false;
                    }
                    if (r3 != com.example.evanscomputermod.radio.controller.ControllerRadio.Result.NO_RECEIVER) {
                        failure[0] = "control: bare computer said " + r3;
                        return false;
                    }
                    step[0] = 3;
                }
                case 3 -> {   // the module applies receptions on its tick
                    step[0] = 4;
                }
                default -> {
                    var hub = pc.getControllers();
                    if (hub.playerOf(near) != 1) {
                        failure[0] = "near controller not connected (player " + hub.playerOf(near) + ")";
                        return false;
                    }
                    if (hub.playerOf(far) != 0) {
                        failure[0] = "control: controller 3 km away connected";
                        return false;
                    }
                    com.example.evanscomputermod.radio.controller.ControllerRadio.stop(near);
                    com.example.evanscomputermod.radio.controller.ControllerRadio.stop(far);
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    private record Vec3c(double x, double y, double z) {
        static Vec3c of(BlockPos p) { return new Vec3c(p.getX() + 0.5, p.getY() + 0.5, p.getZ() + 0.5); }
        Vec3c add(double dx, double dy, double dz) { return new Vec3c(x + dx, y + dy, z + dz); }
    }

    /** Click the terminal's left (west) bay with {@code stack}, like a player installing a card or module. */
    private static void useBay(GameTestHelper h, net.minecraft.world.entity.player.Player player, BlockPos rel,
                               net.minecraft.world.item.ItemStack stack, double y) {
        player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, stack);
        BlockPos p = h.absolutePos(rel);
        var face = net.minecraft.core.Direction.WEST;
        var hit = new net.minecraft.world.phys.BlockHitResult(new net.minecraft.world.phys.Vec3(
                p.getX() + 0.5 + face.getStepX() * 0.5, p.getY() + y, p.getZ() + 0.5 + face.getStepZ() * 0.5), face, p, false);
        var state = h.getLevel().getBlockState(p);
        var result = state.useItemOn(stack, h.getLevel(), player, net.minecraft.world.InteractionHand.MAIN_HAND, hit);
        if (result == net.minecraft.world.ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION)
            state.useWithoutItem(h.getLevel(), player, hit);
    }

    /**
     * Two computers with Wi-Fi modules: wlan0 appears, a scan with no access point
     * lists nothing (control), and computer a in monitor mode captures computer b's
     * probe requests into a radiotap pcap and on screen (module, medium, kernel,
     * program end to end). Scenario {@code wifi_monitor}.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_monitor")
    public static void wifi_monitor_captures_probe_requests(GameTestHelper h) {
        var sc = RadioScenarios.ALL.get("wifi_monitor");
        var run = TestDriver.build(h, sc, sc.name);
        boolean[] dumped = {false};
        TestDriver.drive(h, NS, sc.name,
                () -> run.tick() == com.example.evanscomputermod.testing.scenario.ScenarioRun.State.PASSED,
                () -> {
                    if (run.state() != com.example.evanscomputermod.testing.scenario.ScenarioRun.State.FAILED) return null;
                    if (!dumped[0]) {
                        dumped[0] = true;
                        dumpComputerThreads();
                    }
                    return run.failure() + System.lineSeparator() + run.dump();
                });
    }

    /** Stack traces of the computers' worker and program threads (diagnosing a stalled terminal). */
    static void dumpComputerThreads() {
        for (var e : Thread.getAllStackTraces().entrySet()) {
            String n = e.getKey().getName();
            if (!n.startsWith("WASM-Worker") && !n.startsWith("WASI-PID") && !n.startsWith("ecm-wifi")) continue;
            StringBuilder sb = new StringBuilder("THREAD " + n + " " + e.getKey().getState());
            for (var f : e.getValue()) sb.append(System.lineSeparator()).append("    at ").append(f);
            com.example.evanscomputermod.EvansComputerMod.LOGGER.info(sb.toString());
        }
    }

    /**
     * A computer joins a WPA2 network through a virtual access point
     * (AccessPointCore on its own low MAC): wpa_cli + wpa_supplicant complete the
     * 4-way handshake, the AP reports the client authorized, and pings cross the
     * AP to its wired gateway; before association the ping fails (control).
     * Scenario {@code wifi_wpa2_ping}.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_wpa2")
    public static void wifi_wpa2_handshake_and_ping(GameTestHelper h) {
        var sc = RadioScenarios.ALL.get("wifi_wpa2_ping");
        var run = TestDriver.build(h, sc, sc.name);
        TestDriver.drive(h, NS, sc.name, () -> {
            if (run.tick() != com.example.evanscomputermod.testing.scenario.ScenarioRun.State.PASSED) return false;
            com.example.evanscomputermod.testing.scenario.WifiScenarios.VirtualAp.stopAll();
            return true;
        }, () -> {
            if (run.state() != com.example.evanscomputermod.testing.scenario.ScenarioRun.State.FAILED) return null;
            if (com.example.evanscomputermod.testing.scenario.WifiScenarios.VirtualAp.stopAllAndReport()) dumpComputerThreads();
            return run.failure() + System.lineSeparator() + run.dump();
        });
    }

    /**
     * The Wi-Fi module as a Wireless Controller receiver: in wifi mode the computer
     * has no controller receiver (NO_RECEIVER, control) and wlan0's radio is active;
     * after set_mode("controller") a report from 5 blocks away connects as player 1
     * and the kernel-facing radio is off.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_controller")
    public static void wifi_module_controller_mode(GameTestHelper h) {
        BlockPos pcPos = new BlockPos(10, 2, 10);
        var terminalState = com.example.evanscomputermod.block.ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                .setValue(com.example.evanscomputermod.block.TerminalBlock.FACING, net.minecraft.core.Direction.NORTH);
        h.setBlock(pcPos, terminalState);
        var player = h.makeMockPlayer(net.minecraft.world.level.GameType.CREATIVE);
        String dim = h.getLevel().dimension().location().toString();
        UUID ctl = UUID.randomUUID(), who = UUID.randomUUID();
        var pressed = new com.example.evanscomputermod.controller.ControllerState(
                com.example.evanscomputermod.controller.ControllerInput.Button.values()[0].bit(), 0, 0, 0, 0, 0, 0);
        String[] failure = {null};
        int[] step = {0}, waited = {0};
        TestDriver.drive(h, NS, "wifi_module_controller_mode", () -> {
            var pc = (com.example.evanscomputermod.block.TerminalBlockEntity) h.getBlockEntity(pcPos);
            var near = Vec3c.of(h.absolutePos(pcPos)).add(5, 0, 0);
            switch (step[0]) {
                case 0 -> {
                    useBay(h, player, pcPos, new net.minecraft.world.item.ItemStack(com.example.evanscomputermod.item.ModItems.MODULE_EXPANSION_CARD.get()), 0.5);
                    useBay(h, player, pcPos, new net.minecraft.world.item.ItemStack(
                            com.example.evanscomputermod.radio.wifi.RadioWifiContent.WIFI_MODULE.get()), 0.75);
                    step[0] = 1;
                }
                case 1 -> {
                    if (pc.getModuleBays().getModule(0) instanceof com.example.evanscomputermod.radio.wifi.WifiModule w && w.wifiActive()) step[0] = 2;
                    else if (++waited[0] > 100) failure[0] = "Wi-Fi module not live: " + pc.getPeripheralHub().names();
                }
                case 2 -> {
                    var w = (com.example.evanscomputermod.radio.wifi.WifiModule) pc.getModuleBays().getModule(0);
                    if (!"wifi".equals(w.getType()) || !pc.getPeripheralHub().names().contains("left_bay_1")) {
                        failure[0] = "peripheral not attached as left_bay_1: " + pc.getPeripheralHub().names();
                        return false;
                    }
                    var r = com.example.evanscomputermod.radio.controller.ControllerRadio.send(who, dim, near.x(), near.y(), near.z(), 0, ctl, pressed, pc);
                    if (r != com.example.evanscomputermod.radio.controller.ControllerRadio.Result.NO_RECEIVER) {
                        failure[0] = "control: wifi-mode module accepted a controller: " + r;
                        return false;
                    }
                    try {
                        w.set_mode("controller");
                    } catch (com.example.evanscomputermod.api.peripheral.PeripheralException e) {
                        failure[0] = "set_mode: " + e.getMessage();
                        return false;
                    }
                    if (w.wifiActive()) {
                        failure[0] = "controller mode still offers wlan0 to the kernel";
                        return false;
                    }
                    var r2 = com.example.evanscomputermod.radio.controller.ControllerRadio.send(who, dim, near.x(), near.y(), near.z(), 0, ctl, pressed, pc);
                    if (r2 != com.example.evanscomputermod.radio.controller.ControllerRadio.Result.SENT) {
                        failure[0] = "controller-mode send: " + r2;
                        return false;
                    }
                    step[0] = 3;
                }
                case 3 -> step[0] = 4;   // the module applies receptions on its tick
                default -> {
                    var w = (com.example.evanscomputermod.radio.wifi.WifiModule) pc.getModuleBays().getModule(0);
                    if (pc.getControllers().playerOf(ctl) != 1) {
                        if (++waited[0] > 200) failure[0] = "controller not connected through the Wi-Fi module; stats " + w.stats();
                        return false;
                    }
                    if (((Number) w.stats().get("controller_reports")).longValue() < 1) {
                        failure[0] = "module counted no controller reports";
                        return false;
                    }
                    com.example.evanscomputermod.radio.controller.ControllerRadio.stop(ctl);
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    /** Every radio scenario as a test. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".scenarios")
    public static void scenarios_registered(GameTestHelper h) {
        TestDriver.drive(h, NS, "scenarios_registered", () -> RadioScenarios.ALL != null, () -> null);
    }

    static final class TestEndpoint implements RadioEndpoint {
        final UUID id = UUID.randomUUID();
        final Pose pose;
        final Channel channel;
        final List<Reception> got = new CopyOnWriteArrayList<>();

        TestEndpoint(Pose pose, Channel channel) {
            this.pose = pose;
            this.channel = channel;
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return channel; }
        @Override public double maxTxPowerDbm() { return 20; }
        @Override public void onReceive(Reception r) { got.add(r); }
    }

    private RadioTests() {}
}
//?}
