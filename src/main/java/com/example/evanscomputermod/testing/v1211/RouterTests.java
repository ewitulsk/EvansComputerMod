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
    public static void generated_fiber_break_and_crafted_repair(GameTestHelper h) {
        String[] failure = {null};
        long start = System.currentTimeMillis();
        TestDriver.drive(
                h,
                NS,
                "generated_fiber_break_and_crafted_repair",
                () -> {
                    try {
                        var level = h.getLevel();
                        var d = WorldNetwork.get(level);
                        d.plan(level);
                        var a = d.villages.get(0);
                        var b = d.villages.get(1);
                        BlockPos p = new BlockPos((a.x() + b.x()) / 2, 150, (a.z() + b.z()) / 2);
                        var mgr = CableNetworkManager.getInstance();
                        byte[] left = NetworkHub.deriveMac(d.identity(level, 1, "isp.router"), 1),
                                right = NetworkHub.deriveMac(d.identity(level, 2, "isp.router"), 0);
                        level.setBlock(
                                p,
                                com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN
                                        .get()
                                        .defaultBlockState(),
                                3);
                        d.generatedFiber.add(p.asLong());
                        d.applyLinks(level);
                        if (!mgr.areOnSameNetwork(left, right))
                            throw new IllegalStateException("initial fiber missing");
                        level.destroyBlock(p, false);
                        if (!d.brokenFiber.contains(p.asLong())
                                || mgr.areOnSameNetwork(left, right)
                                || mgr.carrierOf(left))
                            throw new IllegalStateException(
                                    "actual block removal did not cut fiber");
                        level.setBlock(
                                p,
                                com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN
                                        .get()
                                        .defaultBlockState(),
                                3);
                        if (d.brokenFiber.contains(p.asLong())
                                || !mgr.areOnSameNetwork(left, right)
                                || !mgr.carrierOf(left))
                            throw new IllegalStateException("replacement did not repair fiber");
                        var saved =
                                d.save(new net.minecraft.nbt.CompoundTag(), level.registryAccess());
                        if (saved.getLongArray("generatedFiber").length == 0)
                            throw new IllegalStateException("fiber was not serialized");
                        d.generatedFiber.remove(p.asLong());
                        level.destroyBlock(p, false);
                        d.setDirty();
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

    @GameTest(
            template = TestDriver.STRUCTURE,
            timeoutTicks = TestDriver.BACKSTOP_TICKS,
            batch = NS + ".worldgen")
    public static void worldgen_village_provisions_uuids(GameTestHelper h) {
        long start = System.currentTimeMillis();
        String[] failure = {null};
        boolean[] placed = {false};
        Set<UUID> previous = new HashSet<>();
        TestDriver.drive(
                h,
                NS,
                "worldgen_village_provisions_uuids",
                () -> {
                    try {
                        var level = h.getLevel();
                        var p = h.absolutePos(new BlockPos(8, 1, 24));
                        if (!placed[0]) {
                            placed[0] = true;
                            for (BlockPos pos :
                                    BlockPos.betweenClosed(
                                            p.offset(-96, -10, -96), p.offset(96, 15, 96)))
                                if (level.getBlockEntity(pos)
                                        instanceof
                                        com.example.evanscomputermod.block.TerminalBlockEntity be)
                                    previous.add(be.getComputerId());
                            level.getServer()
                                    .getCommands()
                                    .performPrefixedCommand(
                                            level.getServer().createCommandSourceStack(),
                                            "place structure evanscomputermod:tech_village "
                                                    + p.getX()
                                                    + " "
                                                    + p.getY()
                                                    + " "
                                                    + p.getZ());
                        }
                        Set<UUID> ids = new HashSet<>();
                        int count = 0;
                        for (BlockPos pos :
                                BlockPos.betweenClosed(
                                        p.offset(-96, -10, -96), p.offset(96, 15, 96)))
                            if (level.getBlockEntity(pos)
                                            instanceof
                                            com.example.evanscomputermod.block.TerminalBlockEntity
                                                            be
                                    && !previous.contains(be.getComputerId())) {
                                count++;
                                ids.add(be.getComputerId());
                                if (!Files.exists(ComputerStorage.path(be).resolve("services.cfg"))
                                        && !Files.exists(
                                                ComputerStorage.path(be).resolve("network.cfg")))
                                    throw new IllegalStateException(
                                            "unprovisioned terminal at " + pos);
                            }
                        if (count != 15 || ids.size() != 15)
                            throw new IllegalStateException(
                                    "expected 15 distinct computers, got "
                                            + count
                                            + "/"
                                            + ids.size());
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
                                        ? "worldgen wall timeout"
                                        : null);
    }
}
//?}
