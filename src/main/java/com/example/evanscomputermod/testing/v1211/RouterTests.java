package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
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
            for (String role : d.infrastructure(v.number())) d.boot(level, v.number(), role);
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
                        for (String role : d.infrastructure(v.number())) {
                            var host =
                                    ComputerHost.get(
                                            level.getServer(), d.identity(level, v.number(), role));
                            // Boot banner only matters before the first command; later output
                            // scrolls it off the screen.
                            if (host.instance() == null
                                    || (phase[0] == 0 && !ScenarioRun.screen(host.headlessDisplay())
                                            .contains("Welcome to Terminal OS"))) return false;
                            if (host.instance().isFaulted()) {
                                failure[0] = "faulted infrastructure " + v.number() + "/" + role;
                                return false;
                            }
                        }
                    var host =
                            ComputerHost.get(level.getServer(), d.identity(level, 1, WorldNetwork.WEB));
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
                    if (phase[0] == 2 && screen.contains("Tech Village 8")) {
                        // Cut the fiber on the short way (1-10-9-8): BGP must reroute 1-2-...-7-8.
                        d.link(level, 8, 9, false);
                        phase[0] = 3;
                        probeAt[0] = 0;
                        return false;
                    }
                    if (phase[0] == 2 && System.currentTimeMillis() > probeAt[0])
                        failure[0] = "HTTP response missing village content:\n" + screen.lines().map(String::stripTrailing).collect(java.util.stream.Collectors.joining("\n"));
                    if (phase[0] == 3) {
                        int last = screen.lastIndexOf("traceroute to");
                        var m = java.util.regex.Pattern.compile("(?m)^\\s*(\\d+)\\s+100\\.72\\.0\\.10\\s*$")
                                .matcher(last < 0 ? "" : screen.substring(last));
                        if (m.find() && Integer.parseInt(m.group(1)) >= 9) {
                            d.link(level, 8, 9, true);
                            // Second cut where the long way transits village 1 (the NAT/uplink router).
                            d.link(level, 2, 3, false);
                            phase[0] = 4;
                            probeAt[0] = 0;
                            return false;
                        }
                        if (System.currentTimeMillis() > probeAt[0]) {
                            host.instance().sendInput("traceroute 100.72.0.10 12\n");
                            for (int r : new int[] {1, 7, 8, 9})
                                ComputerHost.get(level.getServer(), d.identity(level, r, "isp.router")).instance()
                                        .sendInput("show bgp ipv4 unicast summary\nshow ip route\n");
                            probeAt[0] = System.currentTimeMillis() + 8000;
                        }
                    }
                    if (phase[0] == 4) {
                        var two = ComputerHost.get(level.getServer(), d.identity(level, 2, WorldNetwork.WEB));
                        String s2 = ScenarioRun.screen(two.headlessDisplay());
                        int last = s2.lastIndexOf("traceroute to");
                        var m = java.util.regex.Pattern.compile("(?m)^\\s*(\\d+)\\s+100\\.67\\.0\\.10\\s*$")
                                .matcher(last < 0 ? "" : s2.substring(last));
                        if (m.find() && Integer.parseInt(m.group(1)) >= 10) {
                            d.link(level, 2, 3, true);
                            return true;
                        }
                        if (System.currentTimeMillis() > probeAt[0]) {
                            two.instance().sendInput("traceroute 100.67.0.10 14\n");
                            for (int r : new int[] {1, 2, 3, 10})
                                ComputerHost.get(level.getServer(), d.identity(level, r, "isp.router")).instance()
                                        .sendInput("show bgp ipv4 unicast summary\nshow ip route\n");
                            probeAt[0] = System.currentTimeMillis() + 8000;
                        }
                    }
                    return false;
                },
                () -> {
                    if (failure[0] != null) return failure[0];
                    if (System.currentTimeMillis() - start > 55_000) {
                        byte[] cut8 = NetworkHub.deriveMac(d.identity(level, 8, "isp.router"), WorldNetwork.FIBER_NEXT_PORT);
                        StringBuilder dump = new StringBuilder("bootstrap wall timeout (phase " + phase[0] + ", cuts "
                                + d.cuts + ", carrier8.3 mgr=" + CableNetworkManager.getInstance().carrierOf(cut8)
                                + " hub=" + NetworkHub.getInstance().hasCarrier(cut8) + " net="
                                + CableNetworkManager.getInstance().networkOf(cut8) + ")\n");
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
                        dump.append("\nServer 2\n").append(ScenarioRun.screen(ComputerHost.get(level.getServer(),
                                d.identity(level, 2, WorldNetwork.WEB)).headlessDisplay()).lines().map(String::stripTrailing)
                                .collect(java.util.stream.Collectors.joining("\n")));
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
        Object[] state = {null, null};
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
                            for (boolean next : new boolean[] {false, true}) {
                                var panel = level.getBlockState(site.panel(next));
                                if (!panel.is(com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get())
                                        || panel.getValue(com.example.evanscomputermod.block.FiberPatchPanelBlock.FACING) != site.side(next))
                                    throw new IllegalStateException((next ? "next" : "previous") + " patch panel not at predicted "
                                            + site.panel(next) + " facing " + site.side(next) + ": " + panel);
                            }
                            if (visit.datacenter() == null) throw new IllegalStateException("no data center attached to the ISP");
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
                                if (!Files.readString(ComputerStorage.path(be).resolve("etc/chat.conf")).contains(WorldNetwork.chatAddress(d.chatVillage())))
                                    throw new IllegalStateException("no chat.conf for " + t.role());
                                ids.add(be.getComputerId());
                            }
                            BlockPos ispRouter = com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, WorldNetwork.ISP_ROUTER);
                            BlockPos web = com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, WorldNetwork.WEB);
                            for (var role : List.of(WorldNetwork.ISP_ROUTER, WorldNetwork.WEB)) {
                                BlockPos at = role.equals(WorldNetwork.WEB) ? web : ispRouter;
                                if (!(level.getBlockEntity(at) instanceof com.example.evanscomputermod.block.TerminalBlockEntity be)
                                        || !be.getComputerId().equals(d.identity(level, 1, role)))
                                    throw new IllegalStateException(role + " missing at " + at);
                            }
                            int infrastructure = d.chatVillage() == 1 ? 3 : 2;
                            if (ids.size() != net.terminals().size() || visit.computers().size() != ids.size() + infrastructure)
                                throw new IllegalStateException("computers " + visit.computers().size() + " for "
                                        + ids.size() + " house terminals + " + infrastructure + " infrastructure");
                            var wan = com.example.evanscomputermod.testing.TechServerChecks.cableComponent(level, ispRouter.below());
                            for (var t : net.terminals()) {
                                if (t.role().endsWith(".router") && !wan.contains(t.pos().below()))
                                    throw new IllegalStateException(t.role() + " WAN not cabled to the ISP");
                                if (wan.contains(t.pos().above()))
                                    throw new IllegalStateException(t.role() + " LAN touches the access cable");
                            }
                            // The ISP router's own runs: eth2 to the previous panel, eth3 to the next,
                            // eth1 to the data center; no run touches another or the village cable.
                            var prevRun = com.example.evanscomputermod.testing.TechServerChecks.cableComponent(level, net.fiberExit(false));
                            var nextRun = com.example.evanscomputermod.testing.TechServerChecks.cableComponent(level, net.fiberExit(true));
                            var lanRun = com.example.evanscomputermod.testing.TechServerChecks.cableComponent(level, ispRouter.above());
                            if (!prevRun.contains(site.panel(false)) || prevRun.contains(site.panel(true)))
                                throw new IllegalStateException("eth2 run does not reach exactly the previous panel: " + prevRun.size());
                            if (!nextRun.contains(site.panel(true)) || nextRun.contains(site.panel(false)))
                                throw new IllegalStateException("eth3 run does not reach exactly the next panel: " + nextRun.size());
                            if (!lanRun.contains(web.above()))
                                throw new IllegalStateException("server LAN run does not reach the web server's UP face");
                            List<Set<BlockPos>> runs = List.of(prevRun, nextRun, lanRun, wan);
                            for (int i = 0; i < runs.size(); i++)
                                for (int j = i + 1; j < runs.size(); j++)
                                    for (BlockPos p : runs.get(i))
                                        if (runs.get(j).contains(p))
                                            throw new IllegalStateException("cable runs " + i + " and " + j + " touch at " + p);
                            if (!nextRun.contains(site.panel(true).below(3)))
                                throw new IllegalStateException("no riser under the next panel");
                            state[1] = net.fiberExit(true);
                            // Boot the ISP router, the web server and the first house's pair on their
                            // placed blocks (blocks placed by /place-style generation do not auto-start).
                            d.applyLinks(level);
                            var routerBe = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(ispRouter);
                            routerBe.initializeWasm();
                            ((com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(web)).initializeWasm();
                            // The runs sit on the NICs the router configuration expects.
                            if (routerBe.findInterfaceIndexByExitPos(net.fiberExit(false)) != WorldNetwork.FIBER_PREV_PORT
                                    || routerBe.findInterfaceIndexByExitPos(net.fiberExit(true)) != WorldNetwork.FIBER_NEXT_PORT
                                    || routerBe.findInterfaceIndexByExitPos(ispRouter.above()) != WorldNetwork.SERVER_PORT)
                                throw new IllegalStateException("ISP runs are not on eth1/eth2/eth3: "
                                        + routerBe.findInterfaceIndexByExitPos(ispRouter.above()) + "/"
                                        + routerBe.findInterfaceIndexByExitPos(net.fiberExit(false)) + "/"
                                        + routerBe.findInterfaceIndexByExitPos(net.fiberExit(true)));
                            for (var t : net.terminals())
                                if (t.role().startsWith("house1."))
                                    ((com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(t.pos())).initializeWasm();
                            phase[0] = 1;
                            return false;
                        }
                        if (phase[0] == 3) {
                            // Cut the in-building cable from the router's eth3 to the "next" panel the
                            // way a player does (break a riser block): the 1-2 link goes down like a
                            // fiber cut; the 10-1 link (eth2's run) is the control. Putting it back repairs it.
                            var mgr = CableNetworkManager.getInstance();
                            byte[] eth3 = NetworkHub.deriveMac(d.identity(level, 1, WorldNetwork.ISP_ROUTER), WorldNetwork.FIBER_NEXT_PORT);
                            byte[] eth2 = NetworkHub.deriveMac(d.identity(level, 1, WorldNetwork.ISP_ROUTER), WorldNetwork.FIBER_PREV_PORT);
                            if (!d.linkUp(level, 1, 2) || !mgr.carrierOf(eth3) || !mgr.carrierOf(eth2))
                                throw new IllegalStateException("fiber ports down before the cut");
                            BlockPos riser = site.panel(true).below(3);
                            var saved = level.getBlockState(riser);
                            level.destroyBlock(riser, false);
                            if (d.linkUp(level, 1, 2) || mgr.carrierOf(eth3) || mgr.logicalLinkUp("fiber-1-2")
                                    || !d.fiberIntact(level, 1, 2))
                                throw new IllegalStateException("breaking the in-building cable did not cut 1-2 (or blamed the fiber)");
                            if (!d.linkUp(level, 1, 10) || !mgr.carrierOf(eth2))
                                throw new IllegalStateException("control: the eth2 run's link 10-1 must stay up");
                            if (!d.endState(level, 1, true).startsWith("CABLE CUT"))
                                throw new IllegalStateException("end state: " + d.endState(level, 1, true));
                            level.setBlock(riser, saved, 3);
                            if (!d.linkUp(level, 1, 2) || !mgr.carrierOf(eth3) || !mgr.logicalLinkUp("fiber-1-2"))
                                throw new IllegalStateException("restoring the cable did not repair 1-2");
                            return true;
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
                        if (phase[0] == 2 && screen.contains("<h1>Tech Village 1</h1>")) {
                            phase[0] = 3;
                            return false;
                        }
                        if (System.currentTimeMillis() > probe[0]) {
                            pc.getComputer().sendInput(phase[0] == 1 ? "ping 100.65.0.10 -n 1\n" : "curl http://100.65.0.10/\n");
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

    /**
     * Two village data center servers, five ASes apart and next door to the chat village,
     * chat through the ring's chat server over BGP using the /etc/chat.conf they were
     * provisioned with. Control first: pointing chat at a web server (no chatd there) gets
     * the "refused" fix-it hint, which also proves the remote host was reached.
     */
    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".chat")
    public static void chat_between_villages_over_bgp(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        d.applyLinks(level);
        for (var v : d.villages)
            for (String role : d.infrastructure(v.number())) d.boot(level, v.number(), role);
        int k = d.chatVillage(), a = (k + 4) % 10 + 1, b = k % 10 + 1;
        String server = WorldNetwork.chatAddress(k);
        long start = System.currentTimeMillis();
        String[] failure = {null};
        int[] phase = {0};
        long[] probe = {0};
        java.util.function.IntFunction<ComputerHost> web = n -> ComputerHost.get(level.getServer(), d.identity(level, n, WorldNetwork.WEB));
        ComputerHost chatHost = ComputerHost.get(level.getServer(), d.identity(level, k, WorldNetwork.CHAT));
        TestDriver.drive(
                h,
                NS,
                "chat_between_villages_over_bgp",
                () -> {
                    var ha = web.apply(a);
                    var hb = web.apply(b);
                    if (ha.instance() == null || hb.instance() == null || chatHost.instance() == null) return false;
                    String sa = ScenarioRun.screen(ha.headlessDisplay()), sb = ScenarioRun.screen(hb.headlessDisplay());
                    long now = System.currentTimeMillis();
                    switch (phase[0]) {
                        case 0 -> {
                            if (!ScenarioRun.screen(chatHost.headlessDisplay()).contains("chatd listening")) return false;
                            if (sa.contains("1 packets sent, 1 received")) {
                                phase[0] = 1;
                                probe[0] = 0;
                            } else if (now > probe[0]) {
                                ha.instance().sendInput("ping " + server + " -n 1\n");
                                probe[0] = now + 4000;
                            }
                        }
                        case 1 -> {
                            if (probe[0] == 0) {
                                ha.instance().sendInput("chat 100." + (64 + b) + ".0.10\n");
                                probe[0] = 1;
                            } else if (sa.contains("refused")) phase[0] = 2;
                        }
                        case 2 -> {
                            ha.instance().sendInput("chat\n");
                            hb.instance().sendInput("chat\n");
                            phase[0] = 3;
                        }
                        case 3 -> {
                            if (sa.contains("*** Joined") && sb.contains("*** Joined")) {
                                ha.instance().sendInput("hello from village " + a + "\n");
                                phase[0] = 4;
                            }
                        }
                        case 4 -> {
                            if (sb.contains("<v" + a + "-web> hello from village " + a)) {
                                hb.instance().sendInput("/who\n");
                                hb.instance().sendInput("hi back from " + b + "\n");
                                phase[0] = 5;
                            }
                        }
                        case 5 -> {
                            if (sa.contains("<v" + b + "-web> hi back from " + b) && sb.contains("online:")
                                    && sb.contains("v" + a + "-web") ) {
                                ha.instance().sendInput("/quit\n");
                                hb.instance().sendInput("/quit\n");
                                phase[0] = 6;
                            }
                        }
                        case 6 -> {
                            String log = ScenarioRun.screen(chatHost.headlessDisplay());
                            if (sa.contains("*** bye") && sb.contains("*** bye")
                                    && log.contains("v" + a + "-web joined from 100." + (64 + a) + ".0.10")) {
                                EvansComputerMod.LOGGER.info("Chat across villages {} and {} via {}:\n{}\n--- {}\n{}\n--- chatd\n{}", a, b, server,
                                        sa.stripTrailing(), b, sb.stripTrailing(), log.stripTrailing());
                                return true;
                            }
                        }
                        default -> {}
                    }
                    if (now - start > 55_000)
                        failure[0] = "chat phase " + phase[0] + " timed out\n--- web " + a + "\n" + sa.stripTrailing()
                                + "\n--- web " + b + "\n" + sb.stripTrailing() + "\n--- chatd " + k + "\n"
                                + ScenarioRun.screen(chatHost.headlessDisplay()).stripTrailing();
                    return false;
                },
                () -> failure[0]);
    }

    /**
     * The ISP router's runs (eth2/eth3 up the mast to the two panels, eth1 to the data
     * center) fit every style's template for every rotation and every pair of panel sides,
     * end under the planned panels, start on the right NICs and never touch.
     */
    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".layouts")
    public static void isp_cables_fit_every_rotation_and_panel_side(GameTestHelper h) {
        String[] failure = {null};
        TestDriver.drive(
                h,
                NS,
                "isp_cables_fit_every_rotation_and_panel_side",
                () -> {
                    int combos = 0;
                    var level = h.getLevel();
                    for (String style : WorldNetwork.STYLES) {
                        var scan = com.example.evanscomputermod.worldgen.TemplateScan.of(level.getStructureManager(),
                                WorldNetwork.ispTemplate(style)).orElseThrow();
                        var dcScan = com.example.evanscomputermod.worldgen.TemplateScan.of(level.getStructureManager(),
                                WorldNetwork.datacenterTemplate(style)).orElseThrow();
                        if (dcScan.findNbt("ecmRole", WorldNetwork.WEB) == null || dcScan.findNbt("ecmRole", WorldNetwork.CHAT) == null) {
                            failure[0] = style + " data center lacks its servers";
                            return false;
                        }
                        BlockPos router = scan.findNbt("ecmRole", WorldNetwork.ISP_ROUTER);
                        for (var rot : net.minecraft.world.level.block.Rotation.values())
                            for (Direction prev : Direction.Plane.HORIZONTAL)
                                for (Direction next : Direction.Plane.HORIZONTAL) {
                                    if (prev == next) continue;
                                    combos++;
                                    BlockPos origin = new BlockPos(1000, 64, 1000);
                                    var r = com.example.evanscomputermod.worldgen.IspCabling.plan(scan, origin, rot, prev, next, true);
                                    String what = style + " " + rot + " prev=" + prev + " next=" + next;
                                    if (r == null) {
                                        failure[0] = "runs do not fit: " + what;
                                        return false;
                                    }
                                    BlockPos mastTop = com.example.evanscomputermod.worldgen.IspCabling.toWorld(origin, rot,
                                            new BlockPos(10, scan.mastTop(), 10));
                                    var exits = com.example.evanscomputermod.worldgen.IspCabling.nicExits(scan, router, rot);
                                    if (!r.prevRun().get(r.prevRun().size() - 1).above().equals(mastTop.relative(prev))
                                            || !r.nextRun().get(r.nextRun().size() - 1).above().equals(mastTop.relative(next))
                                            || !r.prevExit().equals(com.example.evanscomputermod.worldgen.IspCabling.toWorld(origin, rot, exits.get(2)))
                                            || !r.nextExit().equals(com.example.evanscomputermod.worldgen.IspCabling.toWorld(origin, rot, exits.get(3)))
                                            || !r.lanExit().equals(com.example.evanscomputermod.worldgen.IspCabling.toWorld(origin, rot, exits.get(1)))) {
                                        failure[0] = "runs end or start in the wrong place: " + what;
                                        return false;
                                    }
                                    List<List<Long>> packed = new ArrayList<>();
                                    for (var run : List.of(r.prevRun(), r.nextRun(), r.lanRun())) {
                                        List<Long> cells = new ArrayList<>();
                                        for (BlockPos p : run) cells.add(p.asLong());
                                        if (!CableRouter.connected(cells)) {
                                            failure[0] = "a run is not face-connected: " + what;
                                            return false;
                                        }
                                        packed.add(cells);
                                    }
                                    // Other NICs' faces (eth0 below, the Interface Block's) carry no run.
                                    List<Long> others = new ArrayList<>();
                                    for (int i = 0; i < exits.size(); i++)
                                        if (i == 0 || i > 3)
                                            others.add(com.example.evanscomputermod.worldgen.IspCabling.toWorld(origin, rot, exits.get(i)).asLong());
                                    for (var run : packed)
                                        for (long c : run)
                                            if (others.contains(c)) {
                                                failure[0] = "a run sits on another NIC's face: " + what;
                                                return false;
                                            }
                                    if (!CableRouter.separate(packed)) {
                                        failure[0] = "runs touch: " + what;
                                        return false;
                                    }
                                }
                    }
                    EvansComputerMod.LOGGER.info("ISP cable layouts checked: {}", combos);
                    return combos == 5 * 4 * 12;
                },
                () -> failure[0]);
    }
}
//?}
