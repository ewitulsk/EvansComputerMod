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

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

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
        add(microwaveLink());
        add(wifiRoom());
        add(AntennaScenarios.hamDipole());
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

    private RadioScenarios() {}
}
//?}
