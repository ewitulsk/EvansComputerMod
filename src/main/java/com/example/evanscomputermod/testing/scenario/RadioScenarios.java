package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.radio.microwave.MicrowaveContent;
import com.example.evanscomputermod.radio.microwave.dish.DishBlock;
import com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity;
import com.example.evanscomputermod.radio.microwave.dish.DishSize;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Radio &amp; Wireless scenarios (1.21.1), spawnable with
 * {@code /ecm scenario spawn <name>} and reused by the {@code ecm_radio}
 * GameTests. Every one is built and operated the way a player would do it
 * ({@link Scenario.Builder#asPlayer}): mod blocks placed by right-clicks,
 * modules clicked into bays, Access Points set up in their screen, analyzers
 * and wrenches used on blocks, and everything else typed on the computers or
 * as chat commands. Only vanilla terrain (floors, walls, water, posts, chests)
 * is set directly, as /fill or a creative build would.
 */
public final class RadioScenarios {

    /** Computer A (west) and B (east), 12 blocks apart, each with a Standard SDR on its east side; B has a Speaker on its west side. */
    private static final BlockPos A = new BlockPos(0, 1, 0), B = new BlockPos(12, 1, 0);

    /** Low transmit power keeps the receivers out of clipping at this short range. */
    private static final String TX_POWER = "--power 0";
    static final String PROMPT = PlayerKit.PROMPT;

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();


    static void add(Scenario s) {
        ALL.put(s.name, s);
    }

    // ------------------------------------------------------------ wifi_room

    /** Access Point position in {@code wifi_room}, relative to the scenario origin. */
    public static final BlockPos WIFI_AP = new BlockPos(0, 1, 5);
    public static final String WIFI_SSID = "ecm-room", WIFI_PASS = "correct horse battery";
    public static final String WIFI_PHONE_IP = "10.0.5.20", WIFI_ROGUE_IP = "10.0.5.21";
    /** The phone (right passphrase) and the rogue (wrong one): computers with Wi-Fi modules. */
    public static final BlockPos WIFI_PHONE = new BlockPos(-3, 1, 12), WIFI_ROGUE = new BlockPos(3, 1, 12);
    static final List<BlockPos> WIFI_CABLE = List.of(new BlockPos(0, 0, 0), new BlockPos(0, 0, 1), new BlockPos(0, 0, 2),
            new BlockPos(0, 0, 3), new BlockPos(0, 0, 4), new BlockPos(0, 0, 5));

    /**
     * {@code wifi_room}: a computer ({@code pc}, 10.0.5.1 on eth0) with a cable
     * from its bottom face south (under the floor) to a Wi-Fi Access Point
     * (SSID ecm-room, WPA2-PSK, channel 6, set up in its screen). Seven blocks
     * further south stand two more computers with Wi-Fi modules: the phone
     * (10.0.5.20 on wlan0) knows the passphrase, the rogue (10.0.5.21) has a
     * wrong one. Both run wpa_supplicant. The pc pings the phone through the
     * AP (ARP broadcast under the GTK, unicast under the phone's key, bridged
     * onto the cable); the rogue never completes the 4-way handshake, so its
     * pings get no reply. The AP's Status tab lists the phone authorized.
     * <pre>
     *   pc (0,1,0) facing north; cable (0,0,0)..(0,0,5) under the floor; AP (0,1,5) on its end
     *   phone (-3,1,12), rogue (3,1,12); floor y=0 x -5..5, z -1..14
     * </pre>
     */
    private static Scenario wifiRoom() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-5, 0, -1, 5, 0, 14));
        floor.removeAll(WIFI_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(WIFI_CABLE);
        foot.add(WIFI_AP);
        var b = Scenario.builder("wifi_room",
                        "a computer cabled to a WPA2 Access Point pings a phone computer over Wi-Fi; a rogue computer with the wrong"
                                + " passphrase gets nothing")
                .asPlayer()
                .host("pc", new BlockPos(0, 1, 0), "10.0.5.1/24")
                .host("phone", WIFI_PHONE, "-")
                .host("rogue", WIFI_ROGUE, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : WIFI_CABLE) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, WIFI_AP, Direction.NORTH, WIFI_SSID, WIFI_PASS, 6, 20);
                    PlayerKit.wifiModules(r, "phone", "rogue");
                }, null))
                .note("Setup (done for you): a cable from pc's bottom face under the floor to an Access Point, set up in its screen"
                        + " as SSID " + WIFI_SSID + ", WPA2-PSK \"" + WIFI_PASS + "\", channel 6. phone and rogue each got an"
                        + " expansion card and a Wi-Fi Module.")
                .send("pc", "ifconfig eth0 10.0.5.1/24")
                .expect("pc", "eth0: inet 10\\.0\\.5\\.1/24", "pc has 10.0.5.1")
                .note("The phone joins with the right passphrase");
        PlayerKit.joinWpa2(b, "phone", WIFI_PHONE_IP + "/24", WIFI_SSID, WIFI_PASS);
        b.note("The rogue tries with a wrong passphrase");
        PlayerKit.joinWpa2(b, "rogue", WIFI_ROGUE_IP + "/24", WIFI_SSID, "not the passphrase");
        b.until("phone", "wpa_cli status", "^wpa_state=COMPLETED$", "the phone completed the 4-way handshake")
                .note("Ping the phone over Wi-Fi (the first ping may wait for ARP)")
                .until("pc", "ping " + WIFI_PHONE_IP + " -n 1", "^1 packets sent, 1 received", "the phone answers through the AP")
                .ping("pc", WIFI_PHONE_IP, 3, 3, "three pings through the AP, AES-CCMP on the air")
                .note("Control: the rogue has the wrong passphrase, so it never gets on the network")
                .send("rogue", "wpa_cli status")
                .expect("rogue", "^wpa_state=(?!COMPLETED)\\S+$", "the rogue is not COMPLETED", "^wpa_state=COMPLETED$")
                .ping("pc", WIFI_ROGUE_IP, 2, 0, "no replies from the wrong-passphrase computer")
                .await(RadioScenarios::checkWifiRoom, "AP Status tab: phone AUTHORIZED (handshake DONE), rogue not", 10_000)
                .note("Right-click the access point and open Status to see both clients")
                .timeLimit(60_000);
        return b.build();
    }

    private static String checkWifiRoom(ScenarioRun run) {
        ApPackets.ClientRow p = PlayerKit.apClient(run, WIFI_AP, PlayerKit.moduleOf(run, "phone").mac().mac());
        ApPackets.ClientRow r = PlayerKit.apClient(run, WIFI_AP, PlayerKit.moduleOf(run, "rogue").mac().mac());
        if (p == null || !p.state().equals("AUTHORIZED") || !p.handshake().equals("DONE")) return "phone on the AP screen: " + p;
        if (r != null && r.state().equals("AUTHORIZED")) {
            run.fail("control: the wrong-passphrase computer is AUTHORIZED on the AP screen");
            return "failed";
        }
        run.say("§7  AP screen: phone " + p.state() + "/" + p.handshake() + " at " + p.rssiDbm() + " dBm; rogue "
                + (r == null ? "not listed" : r.state() + "/" + r.handshake() + " (" + r.lastError() + ")"));
        return null;
    }

    // ------------------------------------------------------------ dhcp_lan

    static final String DHCPD_POOL = "pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600";

    /**
     * Phase 1 gate: a DHCP server is software a player runs. Computer A gets
     * /etc/dhcpd.conf (typed with echo) and runs {@code dhcpd} on eth0;
     * computer B, cabled to it, gets a lease with {@code dhclient eth0}.
     * Control first: with no server running, dhclient gets no lease.
     */
    static Scenario dhcpLan() {
        var b = Scenario.builder("dhcp_lan",
                        "dhcpd on one computer leases an address to dhclient on another over a cable;"
                                + " control: no server, no lease.")
                .asPlayer()
                .timeLimit(55_000);
        b.host("server", new BlockPos(0, 1, 0), "192.168.50.1/24");
        b.host("client", new BlockPos(4, 1, 0), null);
        b.link("lan", "server", Direction.DOWN, "client", Direction.DOWN,
                Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): two computers, a cable between their bottom faces.");

        b.note("Control: nothing serves DHCP yet");
        b.send("client", "dhclient -t 4 eth0");
        b.expect("client", "no lease on eth0 after 4s", "no lease without a server");
        b.expect("client", "/ >", "dhclient returned to shell");
        b.send("client", "dhclient -x eth0");
        b.expect("client", "DHCP client stopped", "client stopped for a fresh start");
        b.expect("client", "/ >", "shell");

        b.note("Server: address, /etc/dhcpd.conf (typed with echo; edit /etc/dhcpd.conf works too), dhcpd");
        b.send("server", "ifconfig eth0 192.168.50.1/24");
        b.expect("server", "eth0: inet 192\\.168\\.50\\.1/24", "server address");
        b.expect("server", "/ >", "shell");
        PlayerKit.typeFile(b, "server", "/etc/dhcpd.conf", "# LAN pool served by dhcpd on eth0", DHCPD_POOL);
        b.send("server", "cat /etc/dhcpd.conf");
        b.expect("server", "^pool eth0 192\\.168\\.50\\.10 192\\.168\\.50\\.100 router 192\\.168\\.50\\.1 dns 1\\.1\\.1\\.1 lease 3600",
                "the pool is in /etc/dhcpd.conf");
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

    // ------------------------------------------------------------ wifi_walls

    /**
     * wifi_walls: a computer with a Wi-Fi module in the middle of a stone
     * floor and seven Access Points around it, each about 14 blocks away
     * behind one material: open air (east), glass (west), oak planks (south),
     * leaves (north) one block thick and 3x3; stone (north-east), water in a
     * glass tank (south-east) and iron (south-west) as 3x3x3 cubes on the
     * diagonals. Every AP has its own SSID ({@code walls-<material>}),
     * channel 11, 20 dBm. {@code iw dev wlan0 scan} and then
     * {@code wpa_cli scan_results} on the computer list the open, glass, wood
     * and leaves APs and never the stone, water or iron ones. The player also runs
     * {@code /ecm radio link} from the computer to each AP, which prints the
     * traced path loss (free space, walls, total): stone, water and iron sit
     * 60+ dB below open air, the others within 25 dB. Swap any wall and scan
     * again.
     */
    static final class WifiWalls {
        static final String[] MATERIALS = {"air", "glass", "wood", "leaves", "stone", "water", "iron"};
        static final boolean[] DELIVERS = {true, true, true, true, false, false, false};
        /** Where each material's AP stands, relative to the computer at the origin's (0,1,0). */
        static final BlockPos[] AP = {new BlockPos(14, 1, 0), new BlockPos(-14, 1, 0), new BlockPos(0, 1, 14),
                new BlockPos(0, 1, -14), new BlockPos(10, 1, -10), new BlockPos(10, 1, 10), new BlockPos(-10, 1, 10)};
        static final BlockPos PC = new BlockPos(0, 1, 0);
        static final int R = 15;
        /** Channel 11 (2462 MHz): the Access Point screen offers 1, 6 and 11 on 2.4 GHz. */
        static final int CHANNEL = 11, MHZ = 2462;

        static String ssid(int i) {
            return "walls-" + MATERIALS[i];
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

        /** The wall blocks of lane {@code i} (relative). */
        static List<BlockPos> wallBlocks(int i) {
            BlockPos ap = AP[i];
            int mx = ap.getX() / 2, mz = ap.getZ() / 2;
            if (ap.getX() != 0 && ap.getZ() != 0) return PlayerKit.box(mx - 1, 1, mz - 1, mx + 1, 3, mz + 1);   // diagonal: a cube
            if (ap.getX() != 0) return PlayerKit.box(mx, 1, -1, mx, 3, 1);
            return PlayerKit.box(-1, 1, mz, 1, 3, mz);
        }

        static void terrain(ScenarioRun r) {
            PlayerKit.fill(r, PlayerKit.box(-R, 0, -R, R, 0, R), Blocks.STONE.defaultBlockState());
            for (int i = 0; i < MATERIALS.length; i++) {
                if (MATERIALS[i].equals("water")) {
                    PlayerKit.fill(r, wallBlocks(i), Blocks.GLASS.defaultBlockState());
                    BlockPos c = new BlockPos(AP[i].getX() / 2, 1, AP[i].getZ() / 2);
                    PlayerKit.fill(r, List.of(c, c.above()), Blocks.WATER.defaultBlockState());
                } else {
                    PlayerKit.fill(r, wallBlocks(i), wall(MATERIALS[i]));
                }
            }
        }

        static void build(ScenarioRun r) {
            PlayerKit.wifiModules(r, "pc");
            for (int i = 0; i < MATERIALS.length; i++)
                PlayerKit.accessPoint(r, AP[i], Direction.UP, ssid(i), null, CHANNEL, 20);
        }

        static String centre(ScenarioRun r, BlockPos rel) {
            BlockPos p = r.abs(rel);
            return String.format(Locale.ROOT, "%.1f %.1f %.1f", p.getX() + 0.5, p.getY() + 0.5, p.getZ() + 0.5);
        }

        static final Pattern TOTAL = Pattern.compile("total (-?\\d+\\.\\d) dB.*?walls (-?\\d+\\.\\d)", Pattern.DOTALL);

        /** The player runs /ecm radio link from the computer to every AP and reads the totals. */
        static void linkTable(ScenarioRun r) {
            StringBuilder table = new StringBuilder("wifi_walls, /ecm radio link at " + MHZ + " MHz (channel " + CHANNEL + "):");
            double open = Double.NaN;
            for (int i = 0; i < MATERIALS.length; i++) {
                String cmd = "/ecm radio link " + centre(r, PC) + " " + centre(r, AP[i]) + " " + MHZ;
                String said = String.join("\n", r.player().command(cmd));
                Matcher m = TOTAL.matcher(said);
                if (!m.find()) {
                    r.fail("lane " + MATERIALS[i] + ": /ecm radio link said: " + said);
                    return;
                }
                double total = Double.parseDouble(m.group(1)), walls = Double.parseDouble(m.group(2));
                // The command reports the total as a gain (negative) or a loss (positive): compare magnitudes.
                double loss = Math.abs(total);
                table.append(String.format(Locale.ROOT, "%n  %-7s loss %6.1f dB (walls %.1f)", MATERIALS[i], loss, Math.abs(walls)));
                if (i == 0) open = loss;
                else if (DELIVERS[i] && loss - open > 25 || !DELIVERS[i] && loss - open < 60) {
                    r.fail("lane " + MATERIALS[i] + ": " + String.format(Locale.ROOT, "%.1f", loss - open) + " dB more than open air");
                    return;
                }
            }
            EvansComputerMod.LOGGER.info("[wifi_walls] {}", table);
            r.say("§f" + table);
        }

        static Scenario scenario() {
            List<BlockPos> foot = new ArrayList<>(PlayerKit.box(-R, 0, -R, R, 0, R));
            for (int i = 0; i < MATERIALS.length; i++) {
                foot.addAll(wallBlocks(i));
                foot.add(AP[i]);
            }
            StringBuilder seen = new StringBuilder("\\A");
            for (int i = 0; i < MATERIALS.length; i++)
                if (DELIVERS[i]) seen.append("(?=[\\s\\S]*\\s").append(ssid(i)).append("$)");
            String blocked = "\\swalls-(stone|water|iron)$";
            var b = Scenario.builder("wifi_walls",
                            "a computer with a Wi-Fi module scans 7 Access Points 14 blocks away behind open air, glass, wood, leaves,"
                                    + " stone, water and iron: the first four are found, the last three never; /ecm radio link shows the losses")
                    .asPlayer()
                    .host("pc", PC, "-")
                    .decor(PlayerKit.decor(foot, WifiWalls::terrain, WifiWalls::build, null))
                    .timeLimit(60_000);
            b.note("Setup (done for you): the computer in the middle got a Wi-Fi module; 7 Access Points around it, each set up in"
                    + " its screen as open, channel " + CHANNEL + ", SSID walls-<material>: air east, glass west, wood south, leaves north,"
                    + " stone north-east, water south-east, iron south-west.");
            b.await(PlayerKit.scan("pc", seen.toString(), blocked, 0),
                    "iw dev wlan0 scan, then wpa_cli scan_results (one line per network), until one list has walls-air, walls-glass,"
                            + " walls-wood and walls-leaves; never walls-stone, walls-water or walls-iron", 40_000);
            b.note("Control: scan twice more; the blocked Access Points never show up");
            b.await(PlayerKit.scan("pc", null, blocked, 2), "two more scans without walls-stone, walls-water or walls-iron", 30_000);
            b.note("Path loss of each lane: /ecm radio link <computer x y z> <AP x y z> " + MHZ + " (chat command)");
            b.mutate(WifiWalls::linkTable, "stone, water and iron lose 60+ dB more than open air; glass, wood and leaves under 25 dB more");
            return b.build();
        }
    }

    // ------------------------------------------------------------ microwave_link

    static final DishSize MW_SIZE = DishSize.MEDIUM;
    static final BlockPos MW_WEST = new BlockPos(0, 2, 0), MW_EAST = new BlockPos(28, 2, 0);
    static final BlockPos MW_WEST_RADIO = MW_WEST.above(), MW_EAST_RADIO = MW_EAST.above();
    static final BlockPos MW_WEST_DISH = MW_WEST.east(), MW_EAST_DISH = MW_EAST.west();
    static final List<BlockPos> MW_WEST_CABLE = List.of(new BlockPos(0, 1, 0), new BlockPos(-1, 1, 0), new BlockPos(-2, 1, 0),
            new BlockPos(-2, 2, 0), new BlockPos(-2, 3, 0), new BlockPos(-1, 3, 0));
    static final List<BlockPos> MW_EAST_CABLE = List.of(new BlockPos(28, 1, 0), new BlockPos(29, 1, 0), new BlockPos(30, 1, 0),
            new BlockPos(30, 2, 0), new BlockPos(30, 3, 0), new BlockPos(29, 3, 0));

    static DishBlockEntity mwDish(ScenarioRun r, BlockPos rel) {
        return (DishBlockEntity) r.level().getBlockEntity(r.abs(rel));
    }

    /** The far dish's centre, as a player reads it off the other dish's info() (or F3): "x, y, z". */
    static String farDish(ScenarioRun r, BlockPos rel) {
        var p = mwDish(r, rel).worldPose();
        return String.format(Locale.ROOT, "%.2f, %.2f, %.2f", p.x(), p.y(), p.z());
    }

    /**
     * Phase 7 gate: two wired LANs joined by a microwave link. Each host has a
     * microwave radio on top of it (cabled round to the host's eth0) and a
     * 1.2 m dish beside it that the radio also touches; the dishes, 26
     * blocks apart, face each other on 24 GHz channel 0. From Python on each
     * host the player sets the radio to -40 dBm (which leaves the short hop a
     * realistic fade margin) and aims the dish at the far one with
     * {@code aim_at}. Ping crosses the bridge; control: {@code nudge(30, 0)}
     * the east dish and the pings die; {@code align(40)} finds the far radio
     * again and they come back.
     */
    static Scenario microwaveLink() {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -2, 31, 0, 2);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(MW_WEST_CABLE);
        foot.addAll(MW_EAST_CABLE);
        foot.add(MW_WEST_RADIO);
        foot.add(MW_EAST_RADIO);
        for (int part = 0; part < MW_SIZE.parts(); part++) {
            foot.add(DishBlock.partPos(MW_SIZE, MW_WEST_DISH, Direction.EAST, part));
            foot.add(DishBlock.partPos(MW_SIZE, MW_EAST_DISH, Direction.WEST, part));
        }
        var b = Scenario.builder("microwave_link",
                        "Two LANs bridged by a 24 GHz microwave link between 1.2 m dishes, aimed from Python; control: a dish nudged"
                                + " 30 degrees loses the link, and align() restores it.")
                .asPlayer()
                .timeLimit(60_000);
        b.host("west", MW_WEST, "10.60.0.1/24");
        b.host("east", MW_EAST, "10.60.0.2/24");
        b.decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
            for (List<BlockPos> run : List.of(MW_WEST_CABLE, MW_EAST_CABLE))
                for (BlockPos p : run) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.NORTH);
            r.player().place(MicrowaveContent.MICROWAVE_RADIO.get(), r.abs(MW_WEST_RADIO), Direction.NORTH);
            r.player().place(MicrowaveContent.MICROWAVE_RADIO.get(), r.abs(MW_EAST_RADIO), Direction.NORTH);
            // A dish faces the way the player looks: stand west of the west dish, east of the east one.
            r.player().place(MicrowaveContent.dish(MW_SIZE), r.abs(MW_WEST_DISH), Direction.WEST,
                    s -> s.getValue(DishBlock.FACING) == Direction.EAST);
            r.player().place(MicrowaveContent.dish(MW_SIZE), r.abs(MW_EAST_DISH), Direction.EAST,
                    s -> s.getValue(DishBlock.FACING) == Direction.WEST);
        }, null));
        b.note("Setup (done for you): each host has a Microwave Radio on top (cabled round to its eth0 = bottom face) and a"
                + " 1.2 m dish beside it, facing the other side. 24 GHz channel 0, 56 MHz.");
        b.configureHosts();
        for (String side : List.of("west", "east")) {
            BlockPos far = side.equals("west") ? MW_EAST_DISH : MW_WEST_DISH;
            b.note(side + ": in Python, set the radio to -40 dBm and aim the dish at the far dish (its x, y, z from the far dish's info())");
            PlayerKit.pythonStart(b, side);
            PlayerKit.py(b, side, "import peripheral", "import peripheral");
            PlayerKit.py(b, side, "r = peripheral.find(\"microwave_radio\")", side + " found its microwave radio");
            PlayerKit.py(b, side, "d = peripheral.find(\"dish\")", side + " found its dish");
            PlayerKit.py(b, side, "r.set_tx_power(-40)", side + " radio at -40 dBm");
            PlayerKit.pyPrintFn(b, side, r -> "d.aim_at(" + farDish(r, far) + ")", "d.aim_at(<far dish x>, <y>, <z>)",
                    "\\{'yaw'", side + " dish aimed at the far dish");
            PlayerKit.pythonEnd(b, side);
        }
        b.note("Across the link");
        b.until("west", "ping 10.60.0.2 -n 1", "^1 packets sent, 1 received", "the link is up");
        b.ping("west", "10.60.0.2", 3, 3, "ping crosses the microwave bridge");
        b.expect("west", "/ >", "shell");
        b.note("Control: nudge the east dish 30 degrees (Python on east)");
        PlayerKit.pythonStart(b, "east");
        PlayerKit.py(b, "east", "import peripheral", "import peripheral");
        PlayerKit.py(b, "east", "d = peripheral.find(\"dish\")", "east found its dish");
        PlayerKit.py(b, "east", "d.nudge(30, 0)", "east dish turned 30 degrees");
        b.waitMs(500, "the radio picks up the new aim");
        b.ping("west", "10.60.0.2", 2, 0, "a misaimed dish loses the link");
        b.expect("west", "/ >", "shell");
        b.note("Re-align: dish.align(40) scans for the far radio");
        PlayerKit.pyPrint(b, "east", "d.align(40)", "\\{'found': True", "east dish found the west radio");
        PlayerKit.pythonEnd(b, "east");
        b.waitMs(500, "the radio picks up the new aim");
        b.until("west", "ping 10.60.0.2 -n 1", "^1 packets sent, 1 received", "link back after align");
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
                .asPlayer()
                .realTime()
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(sdrBench(true))
                .note("Setup (done for you): a Standard SDR east of each computer, a Speaker west of B.")
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
                .asPlayer()
                .realTime()
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(sdrBench(false))
                .note("Setup (done for you): a Standard SDR east of each computer.")
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

    /** A floor; the player puts a Standard SDR east of each computer and (optionally) a Speaker west of B, all facing south. */
    private static Scenario.Decor sdrBench(boolean speaker) {
        List<BlockPos> floor = PlayerKit.box(-1, 0, -1, 14, 0, 2);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.add(A.east());
        foot.add(B.east());
        if (speaker) foot.add(B.west());
        return PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
            for (BlockPos pc : List.of(A, B))
                r.player().place(RadioSdrContent.SDR_STANDARD.get(), r.abs(pc.east()), Direction.SOUTH,
                        s -> s.getValue(SdrBlock.FACING) == Direction.SOUTH);
            if (speaker)
                r.player().place(SpeakerContent.SPEAKER.get(), r.abs(B.west()), Direction.SOUTH,
                        s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
        }, null);
    }

    // (last: the scenarios use the constants declared above)
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
        add(AntennaScenarios.antennaTools());
        add(AirshipScenarios.airshipRadio());
        add(PowerScenarios.hamStation());
        add(StationScenarios.radioStation());
    }

    private RadioScenarios() {}
}
//?}
