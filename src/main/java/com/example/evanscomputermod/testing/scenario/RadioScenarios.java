package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ComputerStorage;

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

    private RadioScenarios() {}
}
//?}
