package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.*;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.testing.scenario.*;

import net.minecraft.core.*;
import net.minecraft.gametest.framework.*;
import net.neoforged.neoforge.gametest.*;

import java.nio.file.Files;
import java.util.*;

@GameTestHolder("ecm_router")
@PrefixGameTestTemplate(false)
public final class RouterTests {
    private static final String NS = "ecm_router";

    private static Scenario.Decor files(
            Map<String, Map<String, String>> files,
            java.util.function.Consumer<ScenarioRun> links) {
        return new Scenario.Decor() {
            public List<BlockPos> footprint() {
                return List.of();
            }

            public void build(ScenarioRun run) {
                try {
                    for (var node : files.entrySet()) {
                        var be = run.terminal(node.getKey());
                        var path = ComputerStorage.path(be);
                        Files.createDirectories(path);
                        for (var f : node.getValue().entrySet())
                            Files.writeString(path.resolve(f.getKey()), f.getValue());
                    }
                    links.accept(run);
                } catch (Exception e) {
                    throw new IllegalStateException(e);
                }
            }
        };
    }

    private static void link(
            ScenarioRun r, String name, String a, int portA, String b, int portB, boolean up) {
        CableNetworkManager.getInstance()
                .logicalLink(
                        name,
                        NetworkHub.deriveMac(r.terminal(a).getComputerId(), portA),
                        NetworkHub.deriveMac(r.terminal(b).getComputerId(), portB),
                        up);
    }

    private static Scenario.Builder nodes(String name, String... names) {
        var b = Scenario.builder(name, name);
        for (int i = 0; i < names.length; i++) b.host(names[i], new BlockPos(i * 3, 2, 0), null);
        return b;
    }

    private static Map<String, String> router(String cfg) {
        return Map.of(
                "router.cfg",
                "configure terminal\nip routing\n" + cfg + "end\n",
                "services.cfg",
                "router on\n");
    }

    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".bootstrap")
    public static void virtual_ten_villages_boot_and_reach_server(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        d.applyLinks(level);
        for (var v : d.villages)
            for (String role : List.of("isp.router", "isp.server")) d.boot(level, v.number(), role);
        long start = System.currentTimeMillis();
        String[] failure = {null};
        int[] phase = {0};
        long[] probeAt = {0};
        TestDriver.drive(
                h,
                NS,
                "virtual_ten_villages_boot_and_reach_server",
                () -> {
                    for (var v : d.villages)
                        for (String role : List.of("isp.router", "isp.server")) {
                            var host =
                                    ComputerHost.get(
                                            level.getServer(), d.identity(level, v.number(), role));
                            if (host.instance() == null
                                    || !ScenarioRun.screen(host.headlessDisplay())
                                            .contains("Welcome to Terminal OS")) return false;
                            if (host.instance().isFaulted()) {
                                failure[0] = "faulted infrastructure " + v.number() + "/" + role;
                                return false;
                            }
                        }
                    var host =
                            ComputerHost.get(level.getServer(), d.identity(level, 1, "isp.server"));
                    if (phase[0] == 0) {
                        for (var v : d.villages)
                            ComputerHost.get(
                                            level.getServer(),
                                            d.identity(level, v.number(), "isp.router"))
                                    .instance()
                                    .sendInput(
                                            "router\n"
                                                + "show bgp ipv4 unicast summary\n"
                                                + "show bgp ipv4 unicast\n");
                        phase[0] = 1;
                        return false;
                    }
                    if (System.currentTimeMillis() - start < 6000) return false;
                    if (phase[0] == 1 && System.currentTimeMillis() > probeAt[0]) {
                        host.instance().sendInput("ping 100.72.0.10 -n 1\n");
                        probeAt[0] = System.currentTimeMillis() + 5000;
                    }
                    String screen = ScenarioRun.screen(host.headlessDisplay());
                    if (phase[0] == 1 && screen.contains("1 packets sent, 1 received")) {
                        host.instance().sendInput("curl http://100.72.0.10/index.html\n");
                        probeAt[0] = System.currentTimeMillis() + 5000;
                        phase[0] = 2;
                        return false;
                    }
                    if (phase[0] == 2 && screen.contains("Tech Village 8")) return true;
                    if (phase[0] == 2 && System.currentTimeMillis() > probeAt[0])
                        failure[0] = "HTTP response missing village content:\n" + screen.lines().map(String::stripTrailing).collect(java.util.stream.Collectors.joining("\n"));
                    return false;
                },
                () -> {
                    if (failure[0] != null) return failure[0];
                    if (System.currentTimeMillis() - start > 55_000) {
                        StringBuilder dump = new StringBuilder("bootstrap wall timeout\n");
                        for (var v : d.villages)
                            dump.append("Village ")
                                    .append(v.number())
                                    .append('\n')
                                    .append(
                                            ScenarioRun.screen(
                                                    ComputerHost.get(
                                                                    level.getServer(),
                                                                    d.identity(
                                                                            level,
                                                                            v.number(),
                                                                            "isp.router"))
                                                            .headlessDisplay()).lines().map(String::stripTrailing).collect(java.util.stream.Collectors.joining("\n")));
                        return dump.toString();
                    }
                    return null;
                });
    }

    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".physical")
    public static void generated_fiber_path_cut_and_repair(GameTestHelper h) {
        String[] failure = {null};
        long start = System.currentTimeMillis();
        TestDriver.drive(
                h,
                NS,
                "generated_fiber_path_cut_and_repair",
                () -> {
                    try {
                        var level = h.getLevel();
                        var d = WorldNetwork.get(level);
                        var ring = d.ring(level);
                        var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
                        // Chord 1-2: generate the chunk holding its midpoint through normal
                        // chunk generation (superflat here: the fiber feature must still run).
                        int[][] path = ring.path(0);
                        if (!FiberLine.faceConnected(path))
                            throw new IllegalStateException("chord path not face-connected");
                        int mid = path.length / 2;
                        int cx = path[mid][0] >> 4, cz = path[mid][2] >> 4;
                        level.getChunk(cx, cz);
                        int inChunk = 0;
                        for (int i = 1; i + 1 < path.length; i++) {
                            if ((path[i][0] >> 4) != cx || (path[i][2] >> 4) != cz) continue;
                            BlockPos p = new BlockPos(path[i][0], path[i][1], path[i][2]);
                            var s = level.getBlockState(p);
                            if (!s.is(span))
                                throw new IllegalStateException("path block " + i + " at " + p + " is " + s);
                            for (int j : new int[] {i - 1, i + 1}) {
                                var dir = Direction.fromDelta(path[j][0] - path[i][0], path[j][1] - path[i][1], path[j][2] - path[i][2]);
                                if (!s.getValue(com.example.evanscomputermod.block.NetworkCableBlock.getPropertyForDirection(dir)))
                                    throw new IllegalStateException("span " + i + " lacks arm " + dir);
                            }
                            inChunk++;
                        }
                        if (inChunk < 16) throw new IllegalStateException("only " + inChunk + " path blocks in chunk");
                        d.applyLinks(level);
                        var mgr = CableNetworkManager.getInstance();
                        byte[] left = NetworkHub.deriveMac(d.identity(level, 1, "isp.router"), WorldNetwork.FIBER_NEXT_PORT),
                                right = NetworkHub.deriveMac(d.identity(level, 2, "isp.router"), WorldNetwork.FIBER_PREV_PORT),
                                control = NetworkHub.deriveMac(d.identity(level, 2, "isp.router"), WorldNetwork.FIBER_NEXT_PORT);
                        if (!mgr.areOnSameNetwork(left, right)) throw new IllegalStateException("initial fiber missing");
                        BlockPos p = new BlockPos(path[mid][0], path[mid][1], path[mid][2]);
                        // Control: removing a span placed beside the path changes nothing.
                        level.setBlock(p.above(2), span.defaultBlockState(), 3);
                        level.destroyBlock(p.above(2), false);
                        if (!d.brokenFiber.isEmpty() || !mgr.carrierOf(left))
                            throw new IllegalStateException("off-path span affected the ring");
                        level.destroyBlock(p, false);
                        if (d.linkUp(level, 1, 2) || mgr.areOnSameNetwork(left, right) || mgr.carrierOf(left))
                            throw new IllegalStateException("actual block removal did not cut fiber");
                        if (!d.linkUp(level, 2, 3) || !mgr.carrierOf(control))
                            throw new IllegalStateException("control: chord 2-3 must stay intact");
                        var saved = d.save(new net.minecraft.nbt.CompoundTag(), level.registryAccess());
                        if (saved.getLongArray("brokenFiber").length != 1)
                            throw new IllegalStateException("break was not serialized");
                        if (!level.getBlockState(p.relative(Direction.fromDelta(path[mid - 1][0] - path[mid][0], path[mid - 1][1] - path[mid][1], path[mid - 1][2] - path[mid][2]))).is(span))
                            throw new IllegalStateException("neighbour span vanished");
                        level.setBlock(p, span.defaultBlockState(), 3);
                        if (!d.brokenFiber.isEmpty() || !d.linkUp(level, 1, 2) || !mgr.areOnSameNetwork(left, right)
                                || !mgr.carrierOf(left))
                            throw new IllegalStateException("replacement did not repair fiber");
                        var repaired = level.getBlockState(p);
                        var back = Direction.fromDelta(path[mid - 1][0] - path[mid][0], path[mid - 1][1] - path[mid][1], path[mid - 1][2] - path[mid][2]);
                        if (!repaired.getValue(com.example.evanscomputermod.block.NetworkCableBlock.getPropertyForDirection(back)))
                            throw new IllegalStateException("replaced span did not rejoin the path");
                        d.link(level, 1, 2, false);
                        if (d.linkUp(level, 1, 2) || mgr.carrierOf(left))
                            throw new IllegalStateException("admin cut ignored");
                        d.link(level, 1, 2, true);
                        if (!mgr.carrierOf(left)) throw new IllegalStateException("admin repair ignored");
                        return true;
                    } catch (Exception e) {
                        failure[0] = e.toString();
                        return false;
                    }
                },
                () ->
                        failure[0] != null
                                ? failure[0]
                                : System.currentTimeMillis() - start > 55_000
                                        ? "physical fiber wall timeout"
                                        : null);
    }

    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".nat")
    public static void forwards_nat_dhcp_and_disabled_control(GameTestHelper h) {
        var b = nodes("forwards_nat_dhcp_and_disabled_control", "client", "router", "server");
        b.decor(
                files(
                        Map.of(
                                "client",
                                Map.of("network.cfg", "iface eth0 dhcp\n"),
                                "server",
                                Map.of("network.cfg", "iface eth0 10.0.0.3/24\n"),
                                "router",
                                router(
                                        "interface eth0\n"
                                            + "ip address 192.168.1.1/24\n"
                                            + "ip nat inside\n"
                                            + "exit\n"
                                            + "interface eth1\n"
                                            + "ip address 10.0.0.2/24\n"
                                            + "ip nat outside\n"
                                            + "exit\n"
                                            + "dhcp-server vrf default\n"
                                            + "pool lan\n"
                                            + "range 192.168.1.10 192.168.1.20\n"
                                            + "default-router 192.168.1.1\n"
                                            + "lease 60\n"
                                            + "enable\n")),
                        r -> {
                            link(r, "test-nat-lan", "client", 0, "router", 0, true);
                            link(r, "test-nat-wan", "router", 1, "server", 0, true);
                        }));
        b.waitMs(1500, "DHCP handshake");
        b.ping("client", "10.0.0.3", 2, 2, "client crosses NAT");
        b.send("router", "router");
        b.expect("router", "router#", "router CLI");
        b.send("router", "show ip nat translations");
        b.expect("router", "192\\.168\\.1\\.10.*10\\.0\\.0\\.2", "NAPT records inside and outside");
        b.send("router", "exit");
        b.expect("router", "/ >", "shell");
        b.send("router", "router off");
        b.expect("router", "Router stopped", "off");
        b.ping("client", "10.0.0.3", 1, 0, "forwarding disabled");
        TestDriver.scenario(h, NS, b.build());
    }

    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".headless")
    public static void headless_detach_reattach_no_reboot(GameTestHelper h) {
        var b = nodes("headless_detach_reattach_no_reboot", "pc");
        b.send("pc", "echo before-detach");
        b.expect("pc", "^before-detach$", "pre-detach kernel");
        ComputerInstance[] original = {null};
        b.mutate(
                r -> {
                    var be = r.terminal("pc");
                    be.getModuleBays().installCard(0);
                    be.getModuleBays()
                            .installModule(ModItems.ALWAYS_ON_MODULE.get().getDefaultInstance(), 0);
                    original[0] = be.getComputer();
                    be.onChunkUnloaded();
                    var host = ComputerHost.get(r.level().getServer(), be.getComputerId());
                    if (!host.isHeadless() || host.instance() != original[0])
                        throw new IllegalStateException("instance lost on unload");
                    original[0].sendInput("sleep 1 &\n");
                },
                "detach actual unload hook");
        b.waitMs(1300, "kernel and child run while detached");
        b.mutate(
                r -> {
                    var be = r.terminal("pc");
                    be.onLoad();
                    if (be.getComputer() != original[0])
                        throw new IllegalStateException("reattach rebooted the kernel");
                },
                "reattach same instance");
        b.send("pc", "echo after-reattach");
        b.expect("pc", "^after-reattach$", "live kernel after attach");
        TestDriver.scenario(h, NS, b.build());
    }

    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".fiber")
    public static void fiber_cut_bgp_reroutes_traffic(GameTestHelper h) {
        var b = nodes("fiber_cut_bgp_reroutes_traffic", "r1", "r2", "r3", "client", "server");
        Map<String, Map<String, String>> config = new HashMap<>();
        for (int i = 1; i <= 3; i++) {
            int prev = i == 1 ? 3 : i - 1, next = i == 3 ? 1 : i + 1;
            String cfg =
                    "interface eth0\nip address 172.31."
                            + prev
                            + ".2/30\nexit\ninterface eth1\nip address 172.31."
                            + i
                            + ".1/30\nexit\ninterface eth2\nip address 100."
                            + (64 + i)
                            + ".0.1/24\nexit\nrouter bgp "
                            + (65000 + i)
                            + "\nbgp router-id 100."
                            + (64 + i)
                            + ".0.1\ntimers bgp 1 3\nneighbor 172.31."
                            + prev
                            + ".1 remote-as "
                            + (65000 + prev)
                            + "\nneighbor 172.31."
                            + i
                            + ".2 remote-as "
                            + (65000 + next)
                            + "\naddress-family ipv4 unicast\nneighbor 172.31."
                            + prev
                            + ".1 activate\nneighbor 172.31."
                            + i
                            + ".2 activate\nnetwork 100."
                            + (64 + i)
                            + ".0.0/24\n";
            config.put("r" + i, router(cfg));
        }
        config.put(
                "client",
                Map.of(
                        "network.cfg",
                        "iface eth0 100.65.0.10/24\nroute default via 100.65.0.1 dev eth0\n"));
        config.put(
                "server",
                Map.of(
                        "network.cfg",
                        "iface eth0 100.67.0.10/24\nroute default via 100.67.0.1 dev eth0\n"));
        b.decor(
                files(
                        config,
                        r -> {
                            for (int i = 1; i <= 3; i++)
                                link(
                                        r,
                                        "test-fiber-" + i,
                                        "r" + i,
                                        1,
                                        "r" + (i == 3 ? 1 : i + 1),
                                        0,
                                        true);
                            link(r, "test-fiber-client", "r1", 2, "client", 0, true);
                            link(r, "test-fiber-server", "r3", 2, "server", 0, true);
                        }));
        b.waitMs(4000, "BGP initial convergence");
        b.ping("client", "100.67.0.10", 1, 1, "direct ring path");
        b.mutate(r -> link(r, "test-fiber-3", "r3", 1, "r1", 0, false), "cut direct logical fiber");
        b.waitMs(4500, "hold timer expires and alternate route converges");
        b.ping("client", "100.67.0.10", 1, 1, "long path through r2");
        b.mutate(r -> link(r, "test-fiber-1", "r1", 1, "r2", 0, false), "isolate source control");
        b.waitMs(4000, "withdrawal");
        b.ping("client", "100.67.0.10", 1, 0, "no path after isolation");
        TestDriver.scenario(h, NS, b.build());
    }

    /**
     * The GameTest world disables structures, so the village is generated at its planned
     * site the way {@code /place structure} does. Checks the predicted mast/panel position,
     * identities and provisioning, the physical cabling, and a house PC end to end over the
     * real cable: DHCP from its router (whose WAN leases from the ISP), ping and HTTP to the
     * village server. Natural generation is covered by the client checks.
     */
    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".worldgen")
    public static void village_network_cabled_and_house_pc_online(GameTestHelper h) {
        long start = System.currentTimeMillis();
        String[] failure = {null};
        Object[] state = {null};
        int[] phase = {0};
        long[] probe = {0};
        TestDriver.drive(
                h,
                NS,
                "village_network_cabled_and_house_pc_online",
                () -> {
                    try {
                        var level = h.getLevel();
                        var d = WorldNetwork.get(level);
                        d.plan(level);
                        var site = d.villages.get(0);
                        if (phase[0] == 0) {
                            var startPiece = com.example.evanscomputermod.worldgen.TechVillageLocator.placeForTest(level, 1);
                            var visit = com.example.evanscomputermod.worldgen.TechVillageLocator.resolve(level, 1, startPiece);
                            state[0] = visit;
                            com.example.evanscomputermod.worldgen.TechVillageLocator.hold(level, startPiece, true);
                            if (!level.getBlockState(site.patchPanel()).is(com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get()))
                                throw new IllegalStateException("patch panel not at predicted " + site.patchPanel() + ": "
                                        + level.getBlockState(site.patchPanel()));
                            var net = visit.network();
                            if (net == null || net.terminals().size() < 4)
                                throw new IllegalStateException("network piece missing or < 2 houses");
                            Set<UUID> ids = new HashSet<>();
                            for (var t : net.terminals()) {
                                if (!(level.getBlockEntity(t.pos()) instanceof com.example.evanscomputermod.block.TerminalBlockEntity be))
                                    throw new IllegalStateException("no terminal for " + t.role());
                                if (!be.getComputerId().equals(d.identity(level, 1, t.role())))
                                    throw new IllegalStateException("wrong identity for " + t.role());
                                if (!Files.exists(ComputerStorage.path(be).resolve(t.role().endsWith(".router") ? "router.cfg" : "network.cfg")))
                                    throw new IllegalStateException("unprovisioned " + t.role());
                                ids.add(be.getComputerId());
                            }
                            if (ids.size() != net.terminals().size() || visit.computers().size() != ids.size() + 2)
                                throw new IllegalStateException("computers " + visit.computers().size() + " for "
                                        + ids.size() + " house terminals");
                            BlockPos ispRouter = com.example.evanscomputermod.worldgen.TechVillageLocator.world(visit.isp(), new BlockPos(8, 1, 6));
                            var wan = com.example.evanscomputermod.testing.TechServerChecks.cableComponent(level, ispRouter.below());
                            for (var t : net.terminals()) {
                                if (t.role().endsWith(".router") && !wan.contains(t.pos().below()))
                                    throw new IllegalStateException(t.role() + " WAN not cabled to the ISP");
                                if (wan.contains(t.pos().above()))
                                    throw new IllegalStateException(t.role() + " LAN touches the access cable");
                            }
                            // Boot the ISP pair and the first house's pair on their placed blocks
                            // (blocks placed by /place-style generation do not auto-start).
                            d.applyLinks(level);
                            for (BlockPos local : List.of(new BlockPos(8, 1, 6), new BlockPos(10, 1, 6)))
                                ((com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(
                                        com.example.evanscomputermod.worldgen.TechVillageLocator.world(visit.isp(), local))).initializeWasm();
                            for (var t : net.terminals())
                                if (t.role().startsWith("house1."))
                                    ((com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(t.pos())).initializeWasm();
                            phase[0] = 1;
                            return false;
                        }
                        var visit = (com.example.evanscomputermod.worldgen.TechVillageLocator.Visit) state[0];
                        var pcPos = visit.network().terminals().stream().filter(t -> t.role().equals("house1.pc")).findFirst().orElseThrow().pos();
                        var pc = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(pcPos);
                        if (pc.getComputer() == null) return false;
                        String screen = ScenarioRun.screen(pc.getDisplay());
                        if (!screen.contains("/ >")) return false;
                        if (phase[0] == 1 && screen.contains("1 packets sent, 1 received")) {
                            phase[0] = 2;
                            probe[0] = 0;
                        }
                        if (phase[0] == 2 && screen.contains("Tech Village 1")) return true;
                        if (System.currentTimeMillis() > probe[0]) {
                            pc.getComputer().sendInput(phase[0] == 1 ? "ping 100.65.0.10 -n 1\n" : "curl http://100.65.0.10/index.html\n");
                            probe[0] = System.currentTimeMillis() + 5000;
                        }
                        if (System.currentTimeMillis() - start > 50_000) {
                            var router = visit.network().terminals().stream().filter(t -> t.role().equals("house1.router")).findFirst().orElseThrow().pos();
                            var be = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(router);
                            failure[0] = "house PC offline (phase " + phase[0] + "):\n" + screen.stripTrailing()
                                    + "\n--- house router ---\n" + ScenarioRun.screen(be.getDisplay()).stripTrailing();
                        }
                        return false;
                    } catch (Exception e) {
                        failure[0] = e.toString();
                        return false;
                    }
                },
                () ->
                        failure[0] != null
                                ? failure[0]
                                : System.currentTimeMillis() - start > 55_000
                                        ? "worldgen wall timeout"
                                        : null);
    }
}
//?}
