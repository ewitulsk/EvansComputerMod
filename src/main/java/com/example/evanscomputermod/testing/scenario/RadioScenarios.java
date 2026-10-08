package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ComputerStorage;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.Locale;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicInteger;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

/**
 * Radio &amp; Wireless scenarios (1.21.1), spawnable with
 * {@code /ecm scenario spawn <name>} and reused by {@code RadioTests}.
 * Each radio feature adds its scenarios here (one {@code add(...)} line each).
 */
public final class RadioScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        // Features register their scenarios below, one line each.
        add(dhcpLan());
        add(WifiWalls.scenario());
    }

    static void add(Scenario s) {
        ALL.put(s.name, s);
    }

    static final String DHCPD_CONF = "# LAN pool served by dhcpd on eth0\n"
            + "pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600\n";

    /** Writes a file into a scenario computer's storage when the layout is built. */
    static Scenario.Decor writeFile(String node, String path, String text) {
        return new Scenario.Decor() {
            @Override
            public List<BlockPos> footprint() {
                return List.of();
            }

            @Override
            public void build(ScenarioRun r) {
                try {
                    Path file = ComputerStorage.path(r.terminal(node)).resolve(path);
                    Files.createDirectories(file.getParent());
                    Files.writeString(file, text);
                } catch (java.io.IOException e) {
                    throw new IllegalStateException("writing " + path + " on " + node, e);
                }
            }
        };
    }

    /**
     * Phase 1 gate: a DHCP server is software a player runs. Computer A
     * gets /etc/dhcpd.conf (written when the layout is built, shown with
     * cat; the Java host has no shell redirection) and runs {@code dhcpd}
     * on eth0; computer B, cabled to it, gets a lease with
     * {@code dhclient eth0}. Control first: with no server running,
     * dhclient gets no lease.
     */
    static Scenario dhcpLan() {
        var b = Scenario.builder("dhcp_lan",
                        "dhcpd on one computer leases an address to dhclient on another over a cable;"
                                + " control: no server, no lease.")
                .timeLimit(55_000);
        b.host("server", new BlockPos(0, 1, 0), "192.168.50.1/24");
        b.host("client", new BlockPos(4, 1, 0), null);
        b.link("lan", "server", Direction.DOWN, "client", Direction.DOWN,
                Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.decor(writeFile("server", "etc/dhcpd.conf", DHCPD_CONF));
        b.note("The server's /etc/dhcpd.conf is written for you (edit /etc/dhcpd.conf to change it);"
                + " leases are kept in /var/dhcpd.leases.");

        b.note("Control: nothing serves DHCP yet");
        b.send("client", "dhclient -t 4 eth0");
        b.expect("client", "no lease on eth0 after 4s", "no lease without a server");
        b.expect("client", "/ >", "dhclient returned to shell");
        b.send("client", "dhclient -x eth0");
        b.expect("client", "DHCP client stopped", "client stopped for a fresh start");
        b.expect("client", "/ >", "shell");

        b.note("Server: address, /etc/dhcpd.conf, dhcpd");
        b.send("server", "ifconfig eth0 192.168.50.1/24");
        b.expect("server", "eth0: inet 192\\.168\\.50\\.1/24", "server address");
        b.expect("server", "/ >", "shell");
        b.send("server", "cat /etc/dhcpd.conf");
        b.expect("server", "^pool eth0 192\\.168\\.50\\.10 192\\.168\\.50\\.100 router 192\\.168\\.50\\.1 dns 1\\.1\\.1\\.1 lease 3600",
                "the pool written to /etc/dhcpd.conf");
        b.expect("server", "/ >", "shell");
        b.send("server", "dhcpd &");
        b.expect("server", "serving eth0 192\\.168\\.50\\.10-192\\.168\\.50\\.100/24 as 192\\.168\\.50\\.1",
                "dhcpd serving the pool");

        b.note("Client: dhclient eth0");
        b.send("client", "dhclient eth0");
        b.expect("client", "bound to 192\\.168\\.50\\.10/24", "client leased 192.168.50.10");
        b.expect("client", "router 192\\.168\\.50\\.1, dns 1\\.1\\.1\\.1", "router and DNS options");
        b.expect("server", "DHCPACK on 192\\.168\\.50\\.10 to", "server logged the ACK");
        b.expect("client", "/ >", "shell");
        b.ping("client", "192.168.50.1", 1, 1, "the leased address works");
        b.expect("client", "/ >", "shell");
        b.send("client", "dhclient -s eth0");
        b.expect("client", "state BOUND", "status shows BOUND");
        b.expect("client", "/ >", "shell");
        b.send("client", "dhclient -r eth0");
        b.expect("client", "released 192\\.168\\.50\\.10", "lease released");
        b.expect("server", "DHCPRELEASE of 192\\.168\\.50\\.10", "server freed the lease");
        return b.build();
    }

    /**
     * wifi_walls: seven 2.4 GHz lanes, each a transmitter and a receiver 14 blocks
     * apart with one wall material between them (open air, glass, wood, leaves,
     * stone, water in a glass tank, iron). The medium traces each lane, prints
     * its path gain and sends a frame; open air, glass, wood and leaves deliver,
     * stone, water and iron do not. Swap any wall block and run
     * {@code /ecm radio link} on the lane to see the new loss.
     */
    static final class WifiWalls {
        static final String[] MATERIALS = {"air", "glass", "wood", "leaves", "stone", "water", "iron"};
        static final boolean[] DELIVERS = {true, true, true, true, false, false, false};
        static final int LENGTH = 14;
        static final Map<ScenarioRun, List<RadioEndpoint[]>> RADIOS =
                java.util.Collections.synchronizedMap(new java.util.WeakHashMap<>());

        static int laneZ(int i) {
            return i * 4;
        }

        static BlockState wall(String m) {
            return switch (m) {
                case "glass" -> Blocks.GLASS.defaultBlockState();
                case "wood" -> Blocks.OAK_PLANKS.defaultBlockState();
                case "leaves" -> Blocks.OAK_LEAVES.defaultBlockState().setValue(LeavesBlock.PERSISTENT, true);
                case "stone" -> Blocks.STONE.defaultBlockState();
                case "iron" -> Blocks.IRON_BLOCK.defaultBlockState();
                default -> Blocks.AIR.defaultBlockState();
            };
        }

        /** A lane radio: a vertical dipole at a block centre, tuned to channel 6, counting what it hears. */
        static final class LaneRadio implements RadioEndpoint {
            final UUID id = UUID.randomUUID();
            final Pose pose;
            final AtomicInteger got = new AtomicInteger();

            LaneRadio(String dim, BlockPos p) {
                pose = Pose.at(dim, p.getX() + 0.5, p.getY() + 0.5, p.getZ() + 0.5);
            }

            @Override public UUID id() { return id; }
            @Override public Pose pose() { return pose; }
            @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
            @Override public Channel tunedChannel() { return Channel.wifi24(6); }
            @Override public double maxTxPowerDbm() { return 20; }
            @Override public void onReceive(Reception r) { got.incrementAndGet(); }
        }

        static Scenario.Decor lanes() {
            return new Scenario.Decor() {
                @Override
                public List<BlockPos> footprint() {
                    List<BlockPos> f = new ArrayList<>();
                    for (int x = -1; x <= LENGTH + 1; x++)
                        for (int z = -2; z <= laneZ(MATERIALS.length - 1) + 2; z++) f.add(new BlockPos(x, 0, z));
                    for (int i = 0; i < MATERIALS.length; i++)
                        for (int y = 1; y <= 3; y++)
                            for (int dz = -1; dz <= 1; dz++) f.add(new BlockPos(LENGTH / 2, y, laneZ(i) + dz));
                    return f;
                }

                @Override
                public void build(ScenarioRun r) {
                    var level = r.level();
                    for (BlockPos p : footprint())
                        if (p.getY() == 0) level.setBlock(r.abs(p), Blocks.STONE.defaultBlockState(), 3);
                    int wx = LENGTH / 2;
                    for (int i = 0; i < MATERIALS.length; i++) {
                        int z = laneZ(i);
                        if (MATERIALS[i].equals("water")) {
                            for (int x = wx - 1; x <= wx + 1; x++)
                                for (int y = 0; y <= 3; y++)
                                    for (int dz = -1; dz <= 1; dz++)
                                        level.setBlock(r.abs(new BlockPos(x, y, z + dz)), Blocks.GLASS.defaultBlockState(), 3);
                            for (int y = 1; y <= 2; y++)
                                level.setBlock(r.abs(new BlockPos(wx, y, z)), Blocks.WATER.defaultBlockState(), 3);
                        } else {
                            for (int y = 1; y <= 3; y++)
                                for (int dz = -1; dz <= 1; dz++)
                                    level.setBlock(r.abs(new BlockPos(wx, y, z + dz)), wall(MATERIALS[i]), 3);
                        }
                    }
                    RadioMedium medium = RadioMediumHooks.medium();
                    if (medium == null) return;
                    String dim = level.dimension().location().toString();
                    List<RadioEndpoint[]> radios = new ArrayList<>();
                    for (int i = 0; i < MATERIALS.length; i++) {
                        RadioEndpoint[] pair = {new LaneRadio(dim, r.abs(new BlockPos(0, 1, laneZ(i)))),
                                new LaneRadio(dim, r.abs(new BlockPos(LENGTH, 1, laneZ(i))))};
                        medium.register(pair[0]);
                        medium.register(pair[1]);
                        medium.pathGainDb(pair[0], pair[1], Channel.wifi24(6).centerHz());   // ask for the trace now
                        radios.add(pair);
                    }
                    RADIOS.put(r, radios);
                }

                @Override
                public void clear(ScenarioRun r) {
                    unregister(r);
                }
            };
        }

        static void unregister(ScenarioRun r) {
            List<RadioEndpoint[]> radios = RADIOS.remove(r);
            RadioMedium medium = RadioMediumHooks.medium();
            if (radios == null || medium == null) return;
            for (RadioEndpoint[] pair : radios) {
                medium.unregister(pair[0]);
                medium.unregister(pair[1]);
            }
        }

        static void measure(ScenarioRun r) {
            RadioMedium medium = RadioMediumHooks.medium();
            List<RadioEndpoint[]> radios = RADIOS.get(r);
            if (medium == null || radios == null) {
                r.fail("no radio medium or lanes");
                return;
            }
            Channel ch = Channel.wifi24(6);
            StringBuilder table = new StringBuilder("wifi_walls (2.4 GHz, 14 blocks, 20 dBm):");
            double open = Double.NaN;
            for (int i = 0; i < MATERIALS.length; i++) {
                RadioEndpoint[] pair = radios.get(i);
                double g = medium.pathGainDb(pair[0], pair[1], ch.centerHz());
                if (Double.isNaN(g)) {
                    r.fail("lane " + MATERIALS[i] + " not traced yet");
                    return;
                }
                LaneRadio rx = (LaneRadio) pair[1];
                int before = rx.got.get();
                medium.transmit(pair[0], Emission.frame(ch, 20, medium.nowMicros(), 300, "DSSS-1", 1e6, new byte[] {1, 2, 3, (byte) i}));
                boolean got = rx.got.get() > before;
                table.append(String.format(Locale.ROOT, "%n  %-7s path gain %7.1f dB, frame %s", MATERIALS[i], g, got ? "delivered" : "lost"));
                if (got != DELIVERS[i]) {
                    r.fail("lane " + MATERIALS[i] + ": frame " + (got ? "delivered" : "lost") + " at " + g + " dB");
                    return;
                }
                if (i == 0) open = g;
                else if (DELIVERS[i] && open - g > 25 || !DELIVERS[i] && open - g < 60) {
                    r.fail("lane " + MATERIALS[i] + ": " + (open - g) + " dB below open air");
                    return;
                }
            }
            EvansComputerMod.LOGGER.info("[wifi_walls] {}", table);
            for (var p : r.level().players()) p.sendSystemMessage(Component.literal(table.toString()));
        }

        static Scenario scenario() {
            var b = Scenario.builder("wifi_walls",
                            "Seven 2.4 GHz lanes through open air, glass, wood, leaves, stone, water and iron;"
                                    + " the medium's traced path gain and one frame per lane. Control: open air.")
                    .timeLimit(20_000);
            b.decor(lanes());
            b.note("Lanes along +x from the origin, 4 blocks apart: " + String.join(", ", MATERIALS)
                    + ". Each has a radio at x=0 and x=14 (y=1, channel 6, vertical dipoles).");
            b.note("Manual: swap a lane's wall block, then /ecm radio link <x y z> <x y z> 2437 between its radios"
                    + " to see free space, walls, ground and the total.");
            b.waitMs(1500, "the link cache traces each lane (a few ticks)");
            b.mutate(WifiWalls::measure, "measured every lane: open/glass/wood/leaves deliver, stone/water/iron do not");
            b.mutate(WifiWalls::unregister, "lane radios unregistered");
            return b.build();
        }
    }

    private RadioScenarios() {}
}
//?}
