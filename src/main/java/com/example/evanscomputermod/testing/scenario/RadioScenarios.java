package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.computer.ComputerStorage;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi.ap.IpResponder;
import com.example.evanscomputermod.radio.wifi.ap.VirtualStation;
import com.example.evanscomputermod.radio.wifi.ap.VirtualStations;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.block.LeavesBlock;

import java.util.ArrayList;
import java.util.Locale;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicInteger;

import java.nio.file.Files;
import java.nio.file.Path;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerContent;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Radio &amp; Wireless scenarios (1.21.1), spawnable with
 * {@code /ecm scenario spawn <name>} and reused by {@code RadioTests}.
 * Each radio feature adds its scenarios here (one {@code add(...)} line each).
 */
public final class RadioScenarios {

    // (declared before ALL: the static block below builds scenarios that use them)
    /** Computer A (west) and B (east), 12 blocks apart, each with a Standard SDR on its east side; B has a Speaker on its west side. */
    private static final BlockPos A = new BlockPos(0, 1, 0), B = new BlockPos(12, 1, 0);

    /** Low transmit power keeps the receivers out of clipping at this short range. */
    private static final String TX_POWER = "--power 0";

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        // Features register their scenarios below, one line each.
        add(dhcpLan());
        add(microwaveLink());
        add(wifiRoom());
        add(AntennaScenarios.hamDipole());
        add(WifiWalls.scenario());
        add(sdrLab());
        add(radio0Lab());
        add(WifiScenarios.monitor());
        add(WifiScenarios.wpa2Ping());
    }

    // ------------------------------------------------------------ wifi_room (Access Point, lane 3B)

    /** Access Point position in {@code wifi_room}, relative to the scenario origin. */
    public static final BlockPos WIFI_AP = new BlockPos(0, 1, 5);
    public static final String WIFI_SSID = "ecm-lab", WIFI_PASS = "correct horse battery";
    public static final String WIFI_PHONE_IP = "10.0.5.20", WIFI_ROGUE_IP = "10.0.5.21";
    /** Virtual stations in {@code wifi_room}: the phone (right passphrase) and a rogue (wrong one). */
    public static final BlockPos WIFI_PHONE = new BlockPos(-3, 1, 12), WIFI_ROGUE = new BlockPos(3, 1, 12);

    /**
     * {@code wifi_room}: a computer ({@code pc}, 10.0.5.1 on eth0) with a cable from
     * its bottom face south to a Wi-Fi Access Point (SSID ecm-lab, WPA2-PSK,
     * channel 6). Seven blocks further south stand two virtual Wi-Fi phones (no
     * computer behind them: a station plus a tiny IPv4 host): 10.0.5.20 knows the
     * passphrase, 10.0.5.21 has a wrong one. The pc pings the phone through the
     * AP (ARP broadcast under the GTK, unicast under the phone's TK, bridged onto
     * the cable with the phone's MAC); the rogue never completes the 4-way
     * handshake, so its pings get no reply. Right-click the AP: the Status tab
     * lists the phone (RSSI, rate, handshake DONE) and the rogue's MIC failures.
     * <pre>
     *   pc (0,1,0) facing north; cable (0,0,0)..(0,0,5) under the floor; AP (0,1,5) on its end
     *   phone (-3,1,12) on a lime carpet, rogue (3,1,12) on a red one; floor y=0 x -5..5, z -1..14
     * </pre>
     */
    private static Scenario wifiRoom() {
        return Scenario.builder("wifi_room",
                        "a computer on a cable to a Wi-Fi access point pings a virtual phone over WPA2 (wrong-passphrase control)")
                .host("pc", new BlockPos(0, 1, 0), "10.0.5.1/24")
                .decor(new WifiRoom())
                .note("Access point on the cable from pc eth0: SSID " + WIFI_SSID + ", WPA2-PSK \"" + WIFI_PASS + "\", channel 6."
                        + " Phone " + WIFI_PHONE_IP + " (lime carpet) joins by itself; rogue " + WIFI_ROGUE_IP
                        + " (red carpet) has the wrong passphrase.")
                .configureHosts()
                .note("Ping the phone over Wi-Fi (the first ping may wait for the 4-way handshake and ARP)")
                .until("pc", "ping " + WIFI_PHONE_IP + " -n 1", "^1 packets sent, 1 received", "the phone answers through the AP")
                .ping("pc", WIFI_PHONE_IP, 3, 3, "three pings through the AP, AES-CCMP on the air")
                .note("Control: the rogue phone has the wrong passphrase, so it never gets on the network")
                .ping("pc", WIFI_ROGUE_IP, 2, 0, "no replies from the wrong-passphrase phone")
                .mutate(RadioScenarios::checkWifiRoom, "AP status: phone authorized (handshake DONE), rogue not")
                .note("Right-click the access point and open Status to see both clients")
                .timeLimit(60_000)
                .build();
    }

    /** Key of one of the room's virtual stations in {@link VirtualStations} (per origin, so test and spawned rooms don't clash). */
    public static String wifiKey(ScenarioRun run, String who) {
        return "wifi_room@" + run.origin().toShortString() + "/" + who;
    }

    private static void checkWifiRoom(ScenarioRun run) {
        if (!(run.level().getBlockEntity(run.abs(WIFI_AP)) instanceof AccessPointBlockEntity ap) || ap.core() == null) {
            run.fail("access point missing or its radio is off");
            return;
        }
        VirtualStation phone = VirtualStations.get(wifiKey(run, "phone")), rogue = VirtualStations.get(wifiKey(run, "rogue"));
        if (phone == null || rogue == null) {
            run.fail("virtual stations not running");
            return;
        }
        ClientStatus p = ap.core().client(phone.mac());
        if (p == null || p.state() != ClientStatus.State.AUTHORIZED || p.handshake() != ClientStatus.Handshake.DONE) {
            run.fail("phone not authorized at the AP: " + p);
            return;
        }
        ClientStatus r = ap.core().client(rogue.mac());
        if (rogue.everConnected() || (r != null && r.state() == ClientStatus.State.AUTHORIZED))
            run.fail("control: the wrong-passphrase phone got authorized");
    }

    /** Floor, the cable under it, the AP on the cable's end, carpets for the phones, and the two virtual stations. */
    private static final class WifiRoom implements Scenario.Decor {
        private final List<BlockPos> floor = new ArrayList<>();
        private final List<BlockPos> cable = new ArrayList<>();

        WifiRoom() {
            for (int z = 0; z <= 5; z++) cable.add(new BlockPos(0, 0, z));
            for (int x = -5; x <= 5; x++)
                for (int z = -1; z <= 14; z++) {
                    BlockPos p = new BlockPos(x, 0, z);
                    if (!cable.contains(p)) floor.add(p);
                }
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(floor);
            all.addAll(cable);
            all.add(WIFI_AP);
            all.add(WIFI_PHONE);
            all.add(WIFI_ROGUE);
            return all;
        }

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            for (BlockPos p : floor) level.setBlock(run.abs(p), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
            for (BlockPos p : cable) level.setBlock(run.abs(p), ModBlocks.NETWORK_CABLE.get().defaultBlockState(), 3);
            level.setBlock(run.abs(WIFI_AP), AccessPointContent.ACCESS_POINT.get().defaultBlockState(), 3);
            for (BlockPos rel : cable) {   // setBlock skips getStateForPlacement: connect the arms
                BlockPos p = run.abs(rel);
                BlockState s = level.getBlockState(p);
                for (Direction d : Direction.values()) {
                    BlockPos q = p.relative(d);
                    s = s.setValue(NetworkCableBlock.getPropertyForDirection(d),
                            NetworkCableBlock.canConnectToFace(level.getBlockState(q), d.getOpposite(), level, q));
                }
                level.setBlock(p, s, 3);
            }
            level.setBlock(run.abs(WIFI_PHONE), Blocks.LIME_CARPET.defaultBlockState(), 3);
            level.setBlock(run.abs(WIFI_ROGUE), Blocks.RED_CARPET.defaultBlockState(), 3);
            if (!(level.getBlockEntity(run.abs(WIFI_AP)) instanceof AccessPointBlockEntity ap))
                throw new IllegalStateException("no access point at " + run.abs(WIFI_AP));
            String err = ap.applySettings(new ApSettings(WIFI_SSID, false, Security.WPA2_PSK, 6, 20, false, null, null), WIFI_PASS);
            if (err != null) throw new IllegalStateException("AP settings rejected: " + err);
            station(run, "phone", WIFI_PHONE, WIFI_PASS, WIFI_PHONE_IP, 0x20);
            station(run, "rogue", WIFI_ROGUE, "not the passphrase", WIFI_ROGUE_IP, 0x21);
        }

        private static void station(ScenarioRun run, String who, BlockPos rel, String pass, String ip, int tail) {
            BlockPos p = run.abs(rel);
            String dim = run.level().dimension().location().toString();
            long salt = p.asLong();
            MacAddress mac = new MacAddress(0x025A_0000_0000L | ((salt & 0xFFFF) << 8) | tail);
            VirtualStation sta = new VirtualStation(mac, Pose.at(dim, p.getX() + 0.5, p.getY() + 1.2, p.getZ() + 0.5),
                    6, WIFI_SSID, pass, salt).withIp(IpResponder.ip(ip));
            VirtualStations.start(wifiKey(run, who), sta);
        }

        @Override
        public void clear(ScenarioRun run) {
            VirtualStations.stop(wifiKey(run, "phone"));
            VirtualStations.stop(wifiKey(run, "rogue"));
        }
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

    // ------------------------------------------------------------ Phase 7: microwave link

    static final BlockPos MW_WEST_RADIO = new BlockPos(2, 0, 0), MW_EAST_RADIO = new BlockPos(26, 0, 0);
    static final BlockPos MW_WEST_DISH = new BlockPos(2, 1, 0), MW_EAST_DISH = new BlockPos(26, 1, 0);

    /** The two radios, their dishes and the short cable runs from each host's eth0 (down) face. */
    static Scenario.Decor microwaveHop() {
        var size = com.example.evanscomputermod.radio.microwave.dish.DishSize.MEDIUM;
        return new Scenario.Decor() {
            @Override
            public List<BlockPos> footprint() {
                List<BlockPos> f = new java.util.ArrayList<>(List.of(new BlockPos(0, 0, 0), new BlockPos(1, 0, 0), MW_WEST_RADIO,
                        new BlockPos(28, 0, 0), new BlockPos(27, 0, 0), MW_EAST_RADIO));
                for (int part = 0; part < size.parts(); part++) {
                    f.add(com.example.evanscomputermod.radio.microwave.dish.DishBlock.partPos(size, MW_WEST_DISH, Direction.EAST, part));
                    f.add(com.example.evanscomputermod.radio.microwave.dish.DishBlock.partPos(size, MW_EAST_DISH, Direction.WEST, part));
                }
                return f;
            }

            @Override
            public void build(ScenarioRun r) {
                var level = r.level();
                var cable = com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get().defaultBlockState();
                for (BlockPos p : List.of(new BlockPos(0, 0, 0), new BlockPos(1, 0, 0), new BlockPos(28, 0, 0), new BlockPos(27, 0, 0)))
                    level.setBlock(r.abs(p), cable, 3);
                var radio = com.example.evanscomputermod.radio.microwave.MicrowaveContent.MICROWAVE_RADIO.get().defaultBlockState();
                level.setBlock(r.abs(MW_WEST_RADIO), radio, 3);
                level.setBlock(r.abs(MW_EAST_RADIO), radio, 3);
                var dish = com.example.evanscomputermod.radio.microwave.MicrowaveContent.dish(size);
                if (!dish.place(level, r.abs(MW_WEST_DISH), Direction.EAST) || !dish.place(level, r.abs(MW_EAST_DISH), Direction.WEST))
                    throw new IllegalStateException("microwave_link: no room for the dishes");
                var west = mwDish(r, MW_WEST_DISH);
                var east = mwDish(r, MW_EAST_DISH);
                var wp = west.worldPose();
                var ep = east.worldPose();
                west.aimAt(ep.x(), ep.y(), ep.z());
                east.aimAt(wp.x(), wp.y(), wp.z());
                for (BlockPos p : List.of(MW_WEST_RADIO, MW_EAST_RADIO))
                    ((com.example.evanscomputermod.radio.microwave.MicrowaveRadioBlockEntity) level.getBlockEntity(r.abs(p)))
                            .link().setTxPowerDbm(-40);
            }

            @Override
            public void clear(ScenarioRun r) {
                // Remove the dish controllers first so their parts go without dropping items.
                for (BlockPos p : List.of(MW_WEST_DISH, MW_EAST_DISH)) {
                    BlockPos a = r.abs(p);
                    if (r.level().getBlockState(a).getBlock() instanceof com.example.evanscomputermod.radio.microwave.dish.DishBlock)
                        r.level().setBlock(a, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), 3);
                }
            }
        };
    }

    static com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity mwDish(ScenarioRun r, BlockPos rel) {
        return (com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity) r.level().getBlockEntity(r.abs(rel));
    }

    /**
     * Phase 7 gate: two wired LANs joined by a microwave link. Each host is
     * cabled to a microwave radio feeding a 1.2 m dish; the dishes, 24 blocks
     * apart, are aimed at each other on 24 GHz channel 0. The radios run at
     * -40 dBm (automatic transmit power control), which leaves the short hop
     * a realistic ~50 dB fade margin. Ping crosses the bridge; control: turn
     * one dish 30 degrees and the pings die; {@code align} finds the far
     * radio again and they come back.
     */
    static Scenario microwaveLink() {
        var b = Scenario.builder("microwave_link",
                        "Two LANs bridged by a 24 GHz microwave link between 1.2 m dishes; control: a dish turned 30 degrees loses"
                                + " the link, and align() restores it.")
                .timeLimit(55_000);
        b.host("west", new BlockPos(0, 1, 0), "10.60.0.1/24");
        b.host("east", new BlockPos(28, 1, 0), "10.60.0.2/24");
        b.decor(microwaveHop());
        b.note("Each host's eth0 (down) is cabled to a microwave radio; the radios feed dishes aimed at each other"
                + " (24 GHz ch 0, 56 MHz, -40 dBm). Check with the radio's info() or a right click.");
        b.configureHosts();
        b.note("Across the link");
        b.ping("west", "10.60.0.2", 3, 3, "ping crosses the microwave bridge");
        b.expect("west", "/ >", "shell");
        b.note("Control: misaim the east dish by 30 degrees");
        b.mutate(r -> mwDish(r, MW_EAST_DISH).nudge(30, 0), "east dish turned 30 degrees");
        b.waitMs(500, "the radio picks up the new aim");
        b.ping("west", "10.60.0.2", 2, 0, "a misaimed dish loses the link");
        b.expect("west", "/ >", "shell");
        b.note("Re-align: dish.align() scans for the far radio");
        b.mutate(r -> mwDish(r, MW_EAST_DISH).align(40), "east dish aligned on the west radio");
        b.waitMs(500, "the radio picks up the new aim");
        b.ping("west", "10.60.0.2", 2, 2, "link restored after align");
        return b.build();
    }

    // ------------------------------------------------------------ SDR programs

    /**
     * {@code sdr_lab}: A transmits, B receives, through the SDR programs and
     * the server's radio medium. B listens to A's FM test tone through its
     * Speaker ({@code rx_fm}, which reports the strongest audio tone), finds
     * A's carrier with {@code scan}, and decodes an AX.25 packet A sends with
     * {@code afsk1200}. Control: B listening on another frequency decodes
     * nothing while A sends.
     */
    static Scenario sdrLab() {
        return Scenario.builder("sdr_lab",
                        "two computers with SDR blocks: FM tone (rx_fm), band scan, and an AFSK1200 packet from A to B")
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(new SdrBench(true))
                .note("B listens to 146.52 MHz NBFM for 8 s (rx_fm, on its Speaker) while A sends a 1 kHz FM test tone")
                .send("B", "rx_fm 146.52M --seconds 8")
                .send("A", "tx_tone 146.52M --fm 1000 --seconds 5 " + TX_POWER)
                .expect("A", "^tx_tone: done", "A finished transmitting")
                .expectOrFail("B", "^fm: strongest audio tone (9[6-9]\\d|10[0-4]\\d) Hz, ([2-9]\\d|1\\d\\d) dB",
                        "B heard the 1 kHz tone (>= 20 dB over the noise)", "^fm: (strongest audio tone|no audio)")
                .note("A keys a carrier 5 kHz above 146.52 MHz; B scans 146.45-146.60 MHz")
                .send("A", "tx_tone 146.52M --offset 5k --seconds 6 " + TX_POWER)
                .send("B", "scan 146.45M 146.6M --dwell 300")
                .expectOrFail("B", "^scan: [1-9]\\d* active", "scan finished with activity", "^scan: 0 active")
                .expect("B", "146\\.52[45]\\d MHz", "scan logged A's carrier at 146.525 MHz")
                .expect("A", "^tx_tone: done", "A finished transmitting")
                .note("Packet: A sends an AX.25 UI frame with afsk1200; B decodes it")
                .send("B", "afsk1200 recv 144.39M --count 1 --seconds 15")
                .send("A", "afsk1200 send 144.39M N0CALL-1 APRS hello from A " + TX_POWER)
                .expectOrFail("B", "^N0CALL-1>APRS:hello from A", "B decoded A's packet", "^afsk1200: 0 frames")
                .expect("A", "^afsk1200: sent", "A sent it")
                .note("Control: B tuned to 145.00 MHz hears nothing of A's 144.39 MHz packet")
                .send("B", "afsk1200 recv 145.00M --seconds 4")
                .send("A", "afsk1200 send 144.39M N0CALL-1 APRS not for you " + TX_POWER)
                .expectOrFail("B", "^afsk1200: 0 frames decoded", "B decoded nothing off-frequency", "^N0CALL-1>APRS")
                .timeLimit(75_000)
                .build();
    }

    /**
     * {@code radio0_lab}: both computers run {@code radiod} (KISS-TNC style
     * bridge), giving each a {@code radio0} interface on 144.39 MHz packet;
     * A pings B over the air. Control: a ping to an address no station has
     * gets no reply.
     */
    static Scenario radio0Lab() {
        return Scenario.builder("radio0_lab",
                        "IP over AFSK1200 packet: radiod makes radio0 on two computers, and ping crosses the air")
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(new SdrBench(false))
                .note("Both computers bring up radio0 on 144.39 MHz (radiod in the background)")
                .send("A", "radiod radio0 up sdr_0 144.39M --call N0CALL-1 --ip 10.44.0.1/24 --seconds 50 --gain 10 --txdelay 100 -v " + TX_POWER + " &")
                .expect("A", "radio0 up on sdr_0", "A's radio0 is up")
                .send("B", "radiod radio0 up sdr_0 144.39M --call N0CALL-2 --ip 10.44.0.2/24 --seconds 50 --gain 10 --txdelay 100 -v " + TX_POWER + " &")
                .expect("B", "radio0 up on sdr_0", "B's radio0 is up")
                .note("A pings B over the radio (ARP, then echo, each an AX.25 frame; the first may time out while ARP resolves)")
                .send("A", "ping 10.44.0.2 -n 3")
                .expectOrFail("A", "^3 packets sent, [123] received", "B answered over radio0", "^3 packets sent, 0 received")
                .note("Control: nobody has 10.44.0.9")
                .send("A", "ping 10.44.0.9 -n 1")
                .expectOrFail("A", "^1 packets sent, 0 received", "no reply from a missing station", "^1 packets sent, 1 received")
                .timeLimit(75_000)
                .build();
    }

    /** A floor, a Standard SDR east of each computer, and (optionally) a Speaker west of B. */
    private static final class SdrBench implements Scenario.Decor {
        private final boolean speaker;

        SdrBench(boolean speaker) {
            this.speaker = speaker;
        }

        private static BlockPos sdrOf(BlockPos pc) {
            return pc.east();
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>();
            for (int x = -1; x <= 14; x++)
                for (int z = -1; z <= 2; z++)
                    all.add(new BlockPos(x, 0, z));
            all.add(sdrOf(A));
            all.add(sdrOf(B));
            if (speaker) all.add(B.west());
            return all;
        }

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            for (int x = -1; x <= 14; x++)
                for (int z = -1; z <= 2; z++)
                    level.setBlock(run.abs(new BlockPos(x, 0, z)), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
            var sdr = RadioSdrContent.SDR_STANDARD.get().defaultBlockState().setValue(SdrBlock.FACING, Direction.SOUTH);
            level.setBlock(run.abs(sdrOf(A)), sdr, 3);
            level.setBlock(run.abs(sdrOf(B)), sdr, 3);
            if (speaker) {
                level.setBlock(run.abs(B.west()), SpeakerContent.SPEAKER.get().defaultBlockState()
                        .setValue(SpeakerBlock.FACING, Direction.SOUTH), 3);
            }
        }
    }

    private RadioScenarios() {}
}
//?}
