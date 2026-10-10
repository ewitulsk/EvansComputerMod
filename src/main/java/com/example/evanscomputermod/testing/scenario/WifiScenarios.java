package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Wi-Fi client scenarios (1.21.1), built and operated as a player would: the
 * computers, cables and Access Point are placed by right-clicking, the Wi-Fi
 * modules are clicked into the computers' bays, the AP is set up in its
 * screen, and everything else is typed ({@code iw}, {@code tcpdump},
 * {@code wpa_cli}, {@code wpa_supplicant}).
 *
 * <ul>
 *   <li>{@code wifi_monitor}: two computers 8 blocks apart, no access point.
 *       Control: {@code iw dev wlan0 scan} finds nothing. Then {@code a} goes
 *       into monitor mode on channel 1 and captures (to a radiotap pcap and on
 *       screen) the probe requests {@code b}'s scan sends: module, medium,
 *       kernel and program, end to end.</li>
 *   <li>{@code wifi_wpa2_ping}: a computer with a Wi-Fi module, and an Access
 *       Point (WPA2 "ecm-lab", channel 6) on a cable to a gateway computer
 *       (192.168.77.1). Control: no link yet, so a ping fails. Then
 *       {@code wpa_cli} configures WPA2, {@code wpa_supplicant} associates and
 *       runs the 4-way handshake, the AP screen lists the client AUTHORIZED,
 *       and pings go through the AP to the gateway.</li>
 * </ul>
 */
public final class WifiScenarios {
    public static final String PASSPHRASE = "correct horse battery";
    public static final String SSID = "ecm-lab";
    public static final String GATEWAY_IP = "192.168.77.1";
    /** A finished command: the shell prompt is back. */
    static final String PROMPT = PlayerKit.PROMPT;

    private WifiScenarios() {
    }

    // ------------------------------------------------------------ scenarios

    public static Scenario monitor() {
        BlockPos a = new BlockPos(0, 1, 0), b = new BlockPos(8, 1, 0);
        List<BlockPos> floor = PlayerKit.box(-1, 0, -1, 9, 0, 1);
        return Scenario.builder("wifi_monitor",
                        "two computers with Wi-Fi modules: an empty scan (control), then monitor mode captures the other's probe requests")
                .asPlayer()
                .host("a", a, "-")
                .host("b", b, "-")
                .decor(PlayerKit.decor(floor, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()),
                        r -> PlayerKit.wifiModules(r, "a", "b"), null))
                .note("Setup (done for you): two computers 8 blocks apart; each got a Module Expansion Card and a Wi-Fi Module"
                        + " clicked onto its left side. Open each screen to boot it.")
                .send("a", "iw dev")
                .expect("a", "^\\s+Interface wlan0$", "a has wlan0")
                .send("b", "iw dev")
                .expect("b", "^\\s+Interface wlan0$", "b has wlan0")
                .note("Control: there is no access point, so a scan finds nothing")
                .send("a", "iw dev wlan0 scan")
                .expect("a", "\\A(?![\\s\\S]*^BSS )[\\s\\S]*" + PROMPT, "the scan finished with no BSS listed")
                .note("Monitor mode on a, channel 1")
                .send("a", "iw dev wlan0 set type monitor")
                .expect("a", PROMPT, "a is in monitor mode", "command failed")
                .send("a", "iw dev wlan0 set channel 1")
                .expect("a", PROMPT, "a listens on channel 1", "command failed")
                .send("a", "iw dev wlan0 info")
                .expect("a", "^\\s+type monitor$", "iw shows type monitor")
                .note("a captures to a radiotap pcap while b scans")
                .send("a", "tcpdump -i wlan0 -c 3 -w probes.pcap")
                .expect("a", "link-type IEEE802_11_RADIO", "tcpdump is listening on wlan0", "No such device|not in monitor mode")
                .send("b", "iw dev wlan0 scan")
                .expect("b", PROMPT, "b's scan finished")
                .expect("a", "^3 packets captured$", "a captured 3 frames from the air")
                .mutate(WifiScenarios::checkProbePcap, "probes.pcap on a holds b's probe requests (radiotap, link type 127)")
                .note("The same capture, printed")
                .send("a", "tcpdump -i wlan0 -c 2")
                .expect("a", "link-type IEEE802_11_RADIO", "tcpdump is listening")
                .send("b", "iw dev wlan0 scan")
                .expect("a", "Probe Request \\(\\) SA:[0-9a-f]{2}(:[0-9a-f]{2}){5} DA:ff:ff:ff:ff:ff:ff", "a printed a probe request from b")
                .timeLimit(60_000)
                .build();
    }

    /** {@code wifi_wpa2_ping}: the client computer, the gateway computer, the AP and the gateway's cable. */
    static final BlockPos WPA_PC = new BlockPos(0, 1, 0), WPA_GW = new BlockPos(6, 1, 5), WPA_AP = new BlockPos(6, 1, 2);
    static final List<BlockPos> WPA_CABLE = List.of(new BlockPos(6, 0, 5), new BlockPos(6, 0, 4), new BlockPos(6, 0, 3),
            new BlockPos(6, 0, 2));

    public static Scenario wpa2Ping() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-1, 0, -1, 8, 0, 6));
        floor.removeAll(WPA_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(WPA_CABLE);
        foot.add(WPA_AP);
        var b = Scenario.builder("wifi_wpa2_ping",
                        "a computer joins a WPA2 Access Point (cabled to a gateway computer) with wpa_supplicant and pings the gateway through it")
                .asPlayer()
                .realTime()
                .host("pc", WPA_PC, "-")
                .host("gw", WPA_GW, GATEWAY_IP + "/24")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : WPA_CABLE) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, WPA_AP, Direction.WEST, SSID, PASSPHRASE, 6, 20);
                    PlayerKit.wifiModules(r, "pc");
                }, null))
                .note("Setup (done for you): an Access Point on a cable from the gateway computer's bottom face, set up in its"
                        + " screen as SSID " + SSID + ", WPA2-PSK \"" + PASSPHRASE + "\", channel 6; the pc got an expansion card"
                        + " and a Wi-Fi Module.")
                .send("gw", "ifconfig eth0 " + GATEWAY_IP + "/24")
                .expect("gw", "eth0: inet " + GATEWAY_IP.replace(".", "\\.") + "/24", "the gateway has " + GATEWAY_IP)
                .note("Control: no Wi-Fi link yet, so the gateway can't be reached");
        b.send("pc", "ifconfig wlan0 192.168.77.2/24")
                .expect("pc", "wlan0: inet 192.168.77.2/24", "wlan0 has 192.168.77.2/24")
                .ping("pc", GATEWAY_IP, 1, 0, "no reply without an association")
                .note("The access point is visible to a scan")
                .await(PlayerKit.scan("pc", "\\s" + SSID + "$", null, 0),
                        "iw dev wlan0 scan, then wpa_cli scan_results, until it lists " + SSID, 20_000)
                .note("Configure WPA2 with wpa_cli and start wpa_supplicant")
                .send("pc", "wpa_cli add_network")
                .expect("pc", "^0$", "network 0 added")
                .send("pc", "wpa_cli set_network 0 ssid " + SSID)
                .expect("pc", "^OK$", "ssid set", "^FAIL")
                .send("pc", "wpa_cli set_network 0 psk \"" + PASSPHRASE + "\"")
                .expect("pc", "^OK$", "passphrase set", "^FAIL")
                .send("pc", "wpa_cli enable_network 0")
                .expect("pc", "^OK$", "network 0 enabled", "^FAIL")
                .send("pc", "wpa_supplicant -B -D packet -i wlan0 -c /etc/wpa_supplicant.conf")
                .expect("pc", PROMPT, "wpa_supplicant runs in the background", "wpa_supplicant:")
                .until("pc", "wpa_cli status", "^wpa_state=COMPLETED$", "the 4-way handshake completed")
                .await(WifiScenarios::apShowsAuthorized, "the AP screen's Status tab lists the pc AUTHORIZED, handshake DONE", 10_000)
                .note("Encrypted traffic both ways through the access point")
                .ping("pc", GATEWAY_IP, 3, 3, "3 of 3 pings answered through the AP")
                .send("pc", "iw dev wlan0 link")
                .expect("pc", "^\\s+tx bitrate: \\d+\\.\\d MBit/s", "iw shows the link's bitrate")
                .send("pc", "wpa_cli list_networks")
                .expect("pc", "^0\\s+" + SSID + "\\s+any\\s+\\[CURRENT\\]$", "network 0 is CURRENT")
                .timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ wifi_connect

    public static final String CAFE_SSID = "ecm-cafe", CAFE_PASS = "letmein123";
    static final BlockPos CAFE_ROUTER = new BlockPos(0, 1, 0), CAFE_AP = new BlockPos(0, 1, 4), CAFE_LAPTOP = new BlockPos(5, 1, 9);
    static final List<BlockPos> CAFE_CABLE = List.of(new BlockPos(0, 0, 0), new BlockPos(0, 0, 1), new BlockPos(0, 0, 2),
            new BlockPos(0, 0, 3), new BlockPos(0, 0, 4));
    static final String CAFE_POOL = "pool eth0 192.168.60.10 192.168.60.100 router 192.168.60.1 dns 1.1.1.1 lease 3600";

    /**
     * {@code wifi_connect}: the one-command {@code wifi} program, with every
     * fix-it message a player can run into on the way. A "router" computer
     * (192.168.60.1) is cabled to an Access Point (set up in its screen:
     * SSID ecm-cafe, WPA2 "letmein123", channel 6) and has /etc/dhcpd.conf
     * ready but no dhcpd running. A "laptop" computer 9 blocks away starts
     * without a Wi-Fi Module. Then: no module (the expansion-card fix);
     * the player clicks a card and a Wi-Fi Module in; a wrong password; no
     * password; the right password but no DHCP server; {@code dhcpd &} on the
     * router and {@code wifi connect} again → "Connected. Address ...", ping
     * the router, {@code wifi} shows the connection, and pings still work a
     * few seconds after the program has exited (the kernel keeps the link).
     */
    public static Scenario wifiConnect() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -1, 7, 0, 11));
        floor.removeAll(CAFE_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(CAFE_CABLE);
        foot.add(CAFE_AP);
        String cmd = "wifi connect " + CAFE_SSID;
        var b = Scenario.builder("wifi_connect",
                        "the wifi program joins a WPA2 Access Point and gets a DHCP address, with its fix-it messages for no module,"
                                + " a wrong or missing password and no DHCP server")
                .asPlayer()
                .realTime()
                .host("router", CAFE_ROUTER, "192.168.60.1/24")
                .host("laptop", CAFE_LAPTOP, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : CAFE_CABLE) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, CAFE_AP, Direction.NORTH, CAFE_SSID, CAFE_PASS, 6, 20);
                }, null))
                .timeLimit(60_000)
                .note("Setup (done for you): router computer cabled (bottom face) to an Access Point set up in its screen as SSID "
                        + CAFE_SSID + ", WPA2 \"" + CAFE_PASS + "\", channel 6. The laptop has no Wi-Fi Module yet.")
                .send("router", "ifconfig eth0 192.168.60.1/24")
                .expect("router", "eth0: inet 192\\.168\\.60\\.1/24", "router has 192.168.60.1");
        PlayerKit.typeFile(b, "router", "/etc/dhcpd.conf", CAFE_POOL);
        b.note("Control: the laptop has no Wi-Fi Module")
                .send("laptop", cmd + " " + CAFE_PASS)
                .expect("laptop", "^wifi: this computer has no Wi-Fi Module", "wifi says there is no Wi-Fi Module", "^Connected")
                .expect("laptop", "Module Expansion Card", "...and how to fit one (Module Expansion Card first)")
                .note("Right-click the laptop's side with a Module Expansion Card, then with a Wi-Fi Module")
                .mutate(r -> PlayerKit.wifiModules(r, "laptop"), "card and Wi-Fi Module clicked into the laptop")
                .until("laptop", "iw dev", "^\\s+Interface wlan0$", "wlan0 appears once the module is in")
                .note("Control: a wrong password")
                .send("laptop", cmd + " wrongpass1")
                .expect("laptop", "^wifi: the password for '" + CAFE_SSID + "' is wrong", "wifi says the password is wrong",
                        "^Connected|^wifi: (?!the password)")
                .note("Control: no password")
                .send("laptop", cmd)
                .expect("laptop", "^wifi: '" + CAFE_SSID + "' needs a password", "wifi says it needs a password", "^Connected")
                .note("Control: the right password, but nothing serves DHCP yet")
                .send("laptop", cmd + " " + CAFE_PASS)
                .expect("laptop", "^wifi: joined '" + CAFE_SSID + "', but nothing gave this computer an address",
                        "wifi joined but got no address, and says to run a DHCP server", "^Connected|^wifi: (?!joined)")
                .expect("laptop", "dhcpd &", "...the advice names dhcpd")
                .note("Start the DHCP server on the router, then connect again")
                .send("router", "dhcpd &")
                .expect("router", "serving eth0 192\\.168\\.60\\.10-192\\.168\\.60\\.100/24 as 192\\.168\\.60\\.1", "dhcpd serving the pool")
                .send("laptop", cmd + " " + CAFE_PASS)
                .expect("laptop", "^Connected\\. Address 192\\.168\\.60\\.\\d+/24, router 192\\.168\\.60\\.1", "Connected, with a DHCP address",
                        "^wifi: ")
                .until("laptop", "ping 192.168.60.1 -n 1", "^1 packets sent, 1 received", "the laptop reaches the router over Wi-Fi")
                .send("laptop", "wifi")
                .expect("laptop", "^wlan0: connected to '" + CAFE_SSID + "'", "wifi shows the connection")
                .waitMs(4_000, "the wifi program has exited; the kernel keeps the association")
                .ping("laptop", "192.168.60.1", 2, 2, "pings still answered a few seconds later");
        return b.build();
    }

    // ------------------------------------------------------------ checks

    /** probes.pcap on "a": pcap header with link type 127, three radiotap records, each a probe request from b. */
    static void checkProbePcap(ScenarioRun run) {
        TerminalBlockEntity a = run.terminal("a");
        byte[] bMac = PlayerKit.moduleOf(run, "b").mac().mac();
        byte[] f;
        try {
            f = Files.readAllBytes(com.example.evanscomputermod.computer.ComputerStorage.path(a).resolve("probes.pcap"));
        } catch (java.io.IOException e) {
            throw new IllegalStateException("probes.pcap missing on a: " + e);
        }
        java.nio.ByteBuffer bb = java.nio.ByteBuffer.wrap(f).order(java.nio.ByteOrder.LITTLE_ENDIAN);
        if (f.length < 24 || bb.getInt(0) != 0xa1b2c3d4 || bb.getInt(20) != 127)
            throw new IllegalStateException("probes.pcap: bad header (" + f.length + " bytes)");
        int off = 24, n = 0;
        while (off + 16 <= f.length) {
            int len = bb.getInt(off + 8);
            int rec = off + 16;
            int rtLen = (f[rec + 2] & 0xff) | (f[rec + 3] & 0xff) << 8;
            int fc = f[rec + rtLen] & 0xff;
            byte[] sa = Arrays.copyOfRange(f, rec + rtLen + 10, rec + rtLen + 16);
            if (f[rec] != 0 || fc != 0x40)
                throw new IllegalStateException("record " + n + ": not a radiotap probe request (fc " + Integer.toHexString(fc) + ")");
            if (!Arrays.equals(sa, bMac))
                throw new IllegalStateException("record " + n + ": SA " + MacAddress.of(sa) + " is not b (" + MacAddress.of(bMac) + ")");
            off = rec + len;
            n++;
        }
        if (n != 3) throw new IllegalStateException("probes.pcap has " + n + " records, expected 3");
    }

    /** Null once the AP screen lists the pc AUTHORIZED with the handshake DONE. */
    static String apShowsAuthorized(ScenarioRun run) {
        ApPackets.ClientRow c = PlayerKit.apClient(run, WPA_AP, PlayerKit.moduleOf(run, "pc").mac().mac());
        if (c == null) return "the AP screen doesn't list the pc";
        if (!c.state().equals("AUTHORIZED") || !c.handshake().equals("DONE"))
            return "AP screen: " + c.state() + " / " + c.handshake() + " (" + c.lastError() + ")";
        return null;
    }
}
//?}
