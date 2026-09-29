package com.example.evanscomputermod.testing.scenario;

import com.example.evanscomputermod.testing.scenario.Scenario.Path;
import net.minecraft.core.BlockPos;

import java.util.LinkedHashMap;
import java.util.Map;

import static net.minecraft.core.Direction.DOWN;
import static net.minecraft.core.Direction.EAST;
import static net.minecraft.core.Direction.NORTH;
import static net.minecraft.core.Direction.SOUTH;
import static net.minecraft.core.Direction.UP;
import static net.minecraft.core.Direction.WEST;

/**
 * Switching scenarios, spawnable with {@code /ecm scenario spawn <name>} and
 * run as GameTests in namespace {@code ecm_switch}. They mirror the
 * simulator scenarios (rust/simulator/scenarios/03, 04, 05, 07, 14) on real
 * blocks.
 *
 * <p>Layout (relative; y=0 is the cable level). Every terminal's screen
 * faces north, so on each terminal eth0 = down, eth1 = up, eth2 = south,
 * eth3 = west, eth4 = east. Hosts always use eth0 (the cable under them).
 * <pre>
 *   one switch  : sw1 (0,2,0); hosts west (-4,1,0) on sw1 eth3, east (4,1,0)
 *                 on eth4, south (0,1,4) on eth2, north (0,1,-3) on eth0
 *   two switches: sw1 (0,2,0), sw2 (8,2,0); link A sw1 eth4 - sw2 eth3
 *                 (straight, y=2); link B sw1 eth1 - sw2 eth1 (over the top,
 *                 y=4); h1 west of sw1 (eth3), h2 east of sw2 (eth4)
 * </pre>
 */
public final class SwitchScenarios {
    // Declared before ALL: the static block below uses them.
    private static final BlockPos SW1 = new BlockPos(0, 2, 0);
    private static final BlockPos SW2 = new BlockPos(8, 2, 0);

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        add(basic());
        add(vlans());
        add(trunk());
        add(stp());
        add(lacp());
    }

    private static void add(Scenario s) {
        ALL.put(s.name, s);
    }

    // ------------------------------------------------------------ layouts

    private static void westHost(Scenario.Builder b, String sw, BlockPos swPos, String host, String ip) {
        b.host(host, swPos.offset(-4, -1, 0), ip);
        b.link(sw + "-" + host, sw, WEST, host, DOWN,
                Path.from(swPos.relative(WEST)).go(WEST, 1).go(DOWN, 2).go(WEST, 2));
    }

    private static void eastHost(Scenario.Builder b, String sw, BlockPos swPos, String host, String ip) {
        b.host(host, swPos.offset(4, -1, 0), ip);
        b.link(sw + "-" + host, sw, EAST, host, DOWN,
                Path.from(swPos.relative(EAST)).go(EAST, 1).go(DOWN, 2).go(EAST, 2));
    }

    private static void southHost(Scenario.Builder b, String sw, BlockPos swPos, String host, String ip) {
        b.host(host, swPos.offset(0, -1, 4), ip);
        b.link(sw + "-" + host, sw, SOUTH, host, DOWN,
                Path.from(swPos.relative(SOUTH)).go(SOUTH, 1).go(DOWN, 2).go(SOUTH, 2));
    }

    /** Uses the switch's DOWN face; the host sits north, in front of the switch. */
    private static void northHost(Scenario.Builder b, String sw, BlockPos swPos, String host, String ip) {
        b.host(host, swPos.offset(0, -1, -3), ip);
        b.link(sw + "-" + host, sw, DOWN, host, DOWN,
                Path.from(swPos.relative(DOWN)).go(DOWN, 1).go(NORTH, 3));
    }

    /** sw1 + sw2 joined by link A (east-west); {@code linkB} adds the second, overhead link. */
    private static void twoSwitches(Scenario.Builder b, boolean linkB) {
        b.switchNode("sw1", SW1).switchNode("sw2", SW2);
        b.link("A", "sw1", EAST, "sw2", WEST, Path.from(SW1.relative(EAST)).go(EAST, 6));
        if (linkB) {
            b.link("B", "sw1", UP, "sw2", UP, Path.from(SW1.relative(UP)).go(UP, 1).go(EAST, 8).go(DOWN, 1));
        }
    }

    // ------------------------------------------------------------ scenarios

    private static Scenario basic() {
        Scenario.Builder b = Scenario.builder("switch_basic",
                "One switch, three hosts in VLAN 1: forwarding through the switch and MAC learning.");
        b.switchNode("sw1", SW1);
        westHost(b, "sw1", SW1, "h1", "10.0.0.1/24");
        eastHost(b, "sw1", SW1, "h2", "10.0.0.2/24");
        southHost(b, "sw1", SW1, "h3", "10.0.0.3/24");
        b.configureHosts();

        b.note("Switch: start it, then make the three host ports L2 (access VLAN 1)");
        b.send("sw1", "switch on").expect("sw1", "Switch started", "switch running");
        b.send("sw1", "switch");
        for (String p : new String[] {b.eth("sw1", WEST), b.eth("sw1", EAST), b.eth("sw1", SOUTH)}) {
            b.send("sw1", "interface " + p, "no routing");
            b.expect("sw1", "converted to L2", p + " is an L2 port");
            b.send("sw1", "exit");
        }

        b.note("Hosts ping each other through the switch");
        b.ping("h1", "10.0.0.2", 2, 2, "h1 -> h2 through sw1");
        b.ping("h1", "10.0.0.3", 2, 2, "h1 -> h3 through sw1");
        b.ping("h2", "10.0.0.3", 2, 2, "h2 -> h3 through sw1");

        b.note("The switch learned all three hosts");
        b.send("sw1", "show mac-address-table");
        b.expect("sw1", "Number of MAC addresses : 3", "3 MACs learned in VLAN 1");
        b.send("sw1", "show interface brief");
        b.expect("sw1", "^Port\\s+Link", "port table shown");
        return b.build();
    }

    private static Scenario vlans() {
        Scenario.Builder b = Scenario.builder("switch_vlans",
                "One switch, four hosts on one subnet split into VLAN 10 and VLAN 20: only same-VLAN pings work.");
        b.switchNode("sw1", SW1);
        northHost(b, "sw1", SW1, "a1", "10.0.0.1/24");
        eastHost(b, "sw1", SW1, "a2", "10.0.0.2/24");
        southHost(b, "sw1", SW1, "b1", "10.0.0.3/24");
        westHost(b, "sw1", SW1, "b2", "10.0.0.4/24");
        b.configureHosts();

        String a1 = b.eth("sw1", DOWN), a2 = b.eth("sw1", EAST);
        String b1 = b.eth("sw1", SOUTH), b2 = b.eth("sw1", WEST);
        b.note("Switch: VLAN 10 = a1, a2; VLAN 20 = b1, b2");
        b.send("sw1", "switch",
                "vlan 10", "no shutdown", "exit",
                "vlan 20", "no shutdown", "exit",
                "interface " + a1, "no routing", "vlan access 10", "exit",
                "interface " + a2, "no routing", "vlan access 10", "exit",
                "interface " + b1, "no routing", "vlan access 20", "exit",
                "interface " + b2, "no routing", "vlan access 20", "exit",
                "on", "show vlan");
        b.expect("sw1", "^10\\s.*" + a1 + "," + a2, "VLAN 10 has " + a1 + "," + a2);
        b.expect("sw1", "^20\\s.*" + b1 + "," + b2, "VLAN 20 has " + b1 + "," + b2);
        b.send("sw1", "exit"); // leave the CLI; the switch keeps running

        b.note("Same VLAN: works");
        b.ping("a1", "10.0.0.2", 2, 2, "a1 -> a2 (VLAN 10)");
        b.ping("b1", "10.0.0.4", 2, 2, "b1 -> b2 (VLAN 20)");
        b.note("Across VLANs: nothing gets through, although it is the same subnet");
        b.send("a1", "ping 10.0.0.3 -n 1");
        b.send("b2", "ping 10.0.0.2 -n 1");
        b.expect("a1", "^1 packets sent, 0 received", "a1 (VLAN 10) cannot reach b1 (VLAN 20)");
        b.expect("b2", "^1 packets sent, 0 received", "b2 (VLAN 20) cannot reach a2 (VLAN 10)");
        return b.build();
    }

    private static Scenario trunk() {
        Scenario.Builder b = Scenario.builder("switch_trunk",
                "Two switches joined by an 802.1Q trunk carrying VLANs 10 and 20; a management SVI on sw2; LLDP.");
        twoSwitches(b, false);
        westHost(b, "sw1", SW1, "a10", "10.0.10.1/24");
        southHost(b, "sw1", SW1, "a20", "10.0.20.1/24");
        eastHost(b, "sw2", SW2, "b10", "10.0.10.2/24");
        b.configureHosts();

        String trunk1 = b.eth("sw1", EAST), trunk2 = b.eth("sw2", WEST);
        b.note("sw1: trunk on " + trunk1 + ", a10 in VLAN 10, a20 in VLAN 20");
        b.send("sw1", "switch", "lldp timer 5",
                "vlan 10", "no shutdown", "exit",
                "vlan 20", "no shutdown", "exit",
                "interface " + trunk1, "no routing", "vlan trunk native 1", "vlan trunk allowed 10,20", "exit",
                "interface " + b.eth("sw1", WEST), "no routing", "vlan access 10", "exit",
                "interface " + b.eth("sw1", SOUTH), "no routing", "vlan access 20", "exit",
                "on", "exit");
        b.note("sw2: trunk on " + trunk2 + ", b10 in VLAN 10, SVI 10.0.20.254 in VLAN 20");
        b.send("sw2", "switch", "lldp timer 5",
                "vlan 10", "no shutdown", "exit",
                "vlan 20", "no shutdown", "exit",
                "interface " + trunk2, "no routing", "vlan trunk native 1", "vlan trunk allowed 10,20", "exit",
                "interface " + b.eth("sw2", EAST), "no routing", "vlan access 10", "exit",
                "interface vlan 20", "ip address 10.0.20.254/24");
        b.expect("sw2", "address 10\\.0\\.20\\.254/24", "SVI configured");
        b.send("sw2", "exit", "on", "exit");

        b.note("VLAN 10 end to end across the trunk; VLAN 20 reaches sw2's SVI");
        b.ping("a10", "10.0.10.2", 2, 2, "a10 -> b10 over the tagged trunk");
        b.ping("a20", "10.0.20.254", 2, 2, "a20 -> sw2's VLAN 20 SVI over the trunk");
        b.ping("a10", "10.0.20.254", 1, 0, "VLAN 10 cannot reach the VLAN 20 SVI");

        b.note("LLDP: sw1 sees sw2 on the trunk port");
        b.send("sw1", "switch");
        b.until("sw1", "show lldp neighbor-info", "^" + trunk1 + "\\s+\\S+", "sw2 is sw1's LLDP neighbour on " + trunk1);
        return b.build();
    }

    private static Scenario stp() {
        Scenario.Builder b = Scenario.builder("switch_stp",
                "Two switches cabled twice (a loop). RSTP blocks one link; cut the other and traffic fails over.");
        twoSwitches(b, true);
        westHost(b, "sw1", SW1, "h1", "10.0.0.1/24");
        eastHost(b, "sw2", SW2, "h2", "10.0.0.2/24");
        b.configureHosts();

        // Both uplinks come in on sw1 eth1 (B) / eth4 (A); sw2 eth1 (B) / eth3 (A).
        String[][] ports = {{"sw1", b.eth("sw1", UP), b.eth("sw1", EAST), b.eth("sw1", WEST)},
                            {"sw2", b.eth("sw2", UP), b.eth("sw2", WEST), b.eth("sw2", EAST)}};
        for (String[] p : ports) {
            String sw = p[0];
            b.note(sw + ": RSTP on, uplinks " + p[1] + " + " + p[2] + ", host edge port " + p[3]
                    + (sw.equals("sw1") ? " (sw1 is root: priority 4096)" : ""));
            b.send(sw, "switch", "spanning-tree", "spanning-tree forward-delay 4");
            if (sw.equals("sw1")) b.send(sw, "spanning-tree priority 4096");
            b.send(sw, "interface " + p[1], "no routing", "exit",
                    "interface " + p[2], "no routing", "exit",
                    "interface " + p[3], "no routing", "spanning-tree admin-edge-port", "exit",
                    "on");
        }

        // Equal-cost uplinks: sw2 picks the one to sw1's lower port (eth1, link B) as
        // root port, and blocks link A.
        String rootB = ports[1][1], altA = ports[1][2];
        b.note("sw2 converges: root port " + rootB + " (link B), " + altA + " (link A) blocked");
        b.until("sw2", "show spanning-tree", "^" + altA + "\\s+Alternate\\s+Discarding", altA + " is Alternate/Discarding");
        b.expect("sw2", "Root port\\s*:\\s*" + rootB, "root port is " + rootB);
        b.until("sw2", "show spanning-tree", "^" + rootB + "\\s+Root\\s+Forwarding", rootB + " is Root/Forwarding");
        b.ping("h1", "10.0.0.2", 3, 3, "h1 -> h2 through the loop, no storm");

        b.note("Cut link B: sw2 stops hearing BPDUs on " + rootB + " and fails over to link A");
        b.cut("B", 5, "remove the middle of the overhead cable");
        // A mid-cable cut keeps carrier, so sw2 notices by BPDU timeout (3 hellos), and
        // the new root port then waits out the recent-root timer before forwarding.
        b.until("sw2", "show spanning-tree", "Root port\\s*:\\s*" + altA, "root port moved to " + altA);
        b.until("sw2", "show spanning-tree", "^" + altA + "\\s+Root\\s+Forwarding", altA + " is Root/Forwarding");
        b.ping("h1", "10.0.0.2", 3, 3, "h1 -> h2 over link A");
        return b.build();
    }

    private static Scenario lacp() {
        Scenario.Builder b = Scenario.builder("switch_lacp",
                "Two switches with both links bundled into LACP LAG 1; cut a member and traffic keeps flowing.");
        twoSwitches(b, true);
        westHost(b, "sw1", SW1, "h1", "10.0.0.1/24");
        eastHost(b, "sw2", SW2, "h2", "10.0.0.2/24");
        b.configureHosts();

        String[][] ports = {{"sw1", b.eth("sw1", UP), b.eth("sw1", EAST), b.eth("sw1", WEST)},
                            {"sw2", b.eth("sw2", UP), b.eth("sw2", WEST), b.eth("sw2", EAST)}};
        for (String[] p : ports) {
            String sw = p[0];
            b.note(sw + ": LAG 1 = " + p[1] + " + " + p[2] + " (LACP active, fast), host on " + p[3]);
            b.send(sw, "switch",
                    "interface lag 1", "no routing", "lacp mode active", "lacp rate fast", "exit",
                    "interface " + p[1], "lag 1");
            b.expect(sw, p[1] + " joined LAG 1", p[1] + " in LAG 1");
            b.send(sw, "exit", "interface " + p[2], "lag 1");
            b.expect(sw, p[2] + " joined LAG 1", p[2] + " in LAG 1");
            b.send(sw, "exit", "interface " + p[3], "no routing", "exit", "on");
        }

        String m1 = ports[0][1], m2 = ports[0][2];
        b.note("LACP negotiates; both members distribute");
        b.until("sw1", "show lacp aggregates",
                "^1\\s+" + m1 + "," + m2 + "\\s+active\\s+fast\\s+y\\s+" + m1 + "," + m2, "LAG 1 up on " + m1 + "," + m2);
        b.ping("h1", "10.0.0.2", 3, 3, "h1 -> h2 over the LAG");

        b.note("Cut link A (" + m2 + "): its LACP partner times out and the LAG runs on " + m1);
        b.cut("A", 3, "remove the middle of the straight cable");
        b.until("sw1", "show lacp aggregates",
                "^1\\s+" + m1 + "," + m2 + "\\s+active\\s+fast\\s+y\\s+" + m1 + "\\s*$", "only " + m1 + " distributing");
        b.ping("h1", "10.0.0.2", 3, 3, "h1 -> h2 over the remaining member");
        return b.build();
    }

    private SwitchScenarios() {}
}
