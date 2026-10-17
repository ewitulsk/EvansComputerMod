package com.example.evanscomputermod.testing.scenario;

import java.util.*;
import net.minecraft.core.BlockPos;

/**
 * DNS scenarios (docs/DNS.md), spawnable with {@code /ecm scenario spawn <name>} and run as
 * GameTests in namespace {@code ecm_dns} ({@code testing/v1211/DnsTests}).
 *
 * <p>Real player procedures (AGENTS.md): the computers, the switch and the cables are placed
 * by the scenario player, zone files are written in {@code edit} or typed from Python, and
 * everything else is typed at the computers' keyboards: {@code named}, {@code dig},
 * {@code curl}. Nothing is injected.
 */
public final class DnsScenarios {
  public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

  //? if <=1.21.1 {

  private static final net.minecraft.core.Direction DOWN = net.minecraft.core.Direction.DOWN,
      UP = net.minecraft.core.Direction.UP, NORTH = net.minecraft.core.Direction.NORTH,
      SOUTH = net.minecraft.core.Direction.SOUTH, EAST = net.minecraft.core.Direction.EAST,
      WEST = net.minecraft.core.Direction.WEST;

  private static void add(Scenario s) {
    ALL.put(s.name, s);
  }

  private static String q(String s) {
    return java.util.regex.Pattern.quote(s);
  }

  // ------------------------------------------------------------ typing helpers

  /** ESC [ B: the Down arrow as the console sends it. */
  private static final String DOWN_ARROW = "\u001b[B";
  private static final int HID_DOWN = 0x51;

  /**
   * Write a new file with the text editor, as a player does: {@code edit FILE}, type each
   * line and Enter, then Ctrl+R (save and exit).
   */
  static void editNew(Scenario.Builder b, String node, String path, String... lines) {
    b.send(node, "edit " + path);
    b.expect(node, q(path) + " - L\\d+/\\d+", node + ": the editor is open on " + path);
    b.mutate(r -> {
      for (String l : lines) {
        if (!l.isEmpty()) r.typeText(node, l);
        r.pressKey(node, ScenarioPlayer.HID_ENTER, 0, "\n");
      }
    }, node + ": type the " + lines.length + " lines of " + path);
    b.mutate(r -> ctrl(r, node, 'r'), node + ": Ctrl+R saves " + path + " and leaves the editor");
    b.expect(node, PlayerKit.PROMPT, node + ": back at the shell");
  }

  /** Open {@code path} in the editor, go down {@code down} lines and delete {@code count} lines with Ctrl+D, then save. */
  static void editDeleteLines(Scenario.Builder b, String node, String path, int down, int count, String what) {
    b.send(node, "edit " + path);
    b.expect(node, q(path) + " - L\\d+/\\d+", node + ": the editor is open on " + path);
    b.mutate(r -> {
      for (int i = 0; i < down; i++) r.pressKey(node, HID_DOWN, 0, DOWN_ARROW);
      for (int i = 0; i < count; i++) ctrl(r, node, 'd');
      ctrl(r, node, 'r');
    }, node + ": " + what + " (Down x" + down + ", Ctrl+D x" + count + ", Ctrl+R)");
    b.expect(node, PlayerKit.PROMPT, node + ": back at the shell");
  }

  private static void ctrl(ScenarioRun r, String node, char c) {
    r.pressKey(node, ScenarioPlayer.hidOf(c),
        com.example.evanscomputermod.computer.ui.proto.UiProto.Mods.CTRL,
        String.valueOf((char) (c & 0x1f)));
  }

  /** {@code named &} and wait for its zones. */
  static void startNamed(Scenario.Builder b, String node, String expect, String what) {
    b.send(node, "named &");
    b.expect(node, expect, what);
  }

  // ------------------------------------------------------------ dns_lab

  /** Around one switch (as switch_vlans): root north (eth0 of the switch: down), tld east, auth south, client west. */
  private static final BlockPos SW = new BlockPos(0, 2, 0);

  static final String LAB_ROOT_ZONE = String.join("\n",
      "$TTL 300",
      "@                    SOA a.root-servers.test. admin.lab.test. 1 60 30 600 30",
      "@                    NS  a.root-servers.test.",
      "a.root-servers.test. A   10.53.0.1",
      "test.                NS  ns.test.",
      "ns.test.             A   10.53.0.2");
  static final String LAB_TEST_ZONE = String.join("\n",
      "$ORIGIN test.",
      "$TTL 300",
      "@        SOA ns admin.lab 1 60 30 600 30",
      "@        NS  ns",
      "ns       A   10.53.0.2",
      "lab      NS  ns1.lab",
      "ns1.lab  A   10.53.0.3");
  static final String[] LAB_ZONE = {
      "$ORIGIN lab.test.",
      "$TTL 60",
      "@     SOA ns1 admin 1 60 30 600 30",
      "@     NS  ns1",
      "ns1   A   10.53.0.3",
      "www   A   10.53.0.3",
      "web   CNAME www"};

  private static Scenario lab() {
    var b = Scenario.builder("dns_lab",
            "A DNS hierarchy on four computers around a switch: root, the test. TLD, the lab.test. zone and a"
                + " client with its own resolver; dig +trace, the stub resolver, then a stopped server (SERVFAIL)"
                + " and a removed delegation (NXDOMAIN).")
        .timeLimit(58_000).asPlayer();
    b.switchNode("sw", SW);
    b.host("root", SW.offset(0, -1, -3), "10.53.0.1/24");
    b.link("sw-root", "sw", DOWN, "root", DOWN, Scenario.Path.from(SW.relative(DOWN)).go(DOWN, 1).go(NORTH, 3));
    b.host("tld", SW.offset(4, -1, 0), "10.53.0.2/24");
    b.link("sw-tld", "sw", EAST, "tld", DOWN, Scenario.Path.from(SW.relative(EAST)).go(EAST, 1).go(DOWN, 2).go(EAST, 2));
    b.host("auth", SW.offset(0, -1, 4), "10.53.0.3/24");
    b.link("sw-auth", "sw", SOUTH, "auth", DOWN, Scenario.Path.from(SW.relative(SOUTH)).go(SOUTH, 1).go(DOWN, 2).go(SOUTH, 2));
    b.host("client", SW.offset(-4, -1, 0), "10.53.0.4/24");
    b.link("sw-client", "sw", WEST, "client", DOWN, Scenario.Path.from(SW.relative(WEST)).go(WEST, 1).go(DOWN, 2).go(WEST, 2));

    b.note("The switch: four L2 ports in VLAN 1");
    b.send("sw", "switch on").expect("sw", "Switch started", "switch running");
    b.send("sw", "switch");
    for (var face : new net.minecraft.core.Direction[] {DOWN, EAST, SOUTH, WEST}) {
      String p = b.eth("sw", face);
      b.send("sw", "interface " + p, "no routing");
      b.expect("sw", "converted to L2", p + " is an L2 port");
      b.send("sw", "exit");
    }
    b.configureHosts();

    b.note("Configuration files: named.conf on every server, the root and test. zones, the client's resolver");
    Map<String, Map<String, String>> files = new LinkedHashMap<>();
    files.put("root", Map.of("/etc/named.conf", "zone . primary /etc/named/root.zone", "/etc/named/root.zone", LAB_ROOT_ZONE));
    files.put("tld", Map.of("/etc/named.conf", "zone test primary /etc/named/test.zone", "/etc/named/test.zone", LAB_TEST_ZONE));
    files.put("auth", Map.of("/etc/named.conf", "zone lab.test primary /etc/named/lab.test.zone",
        "/index.html", "<h1>hello from www.lab.test</h1>"));
    files.put("client", new TreeMap<>(Map.of(
        "/etc/named.conf", "recursion yes\nroot-hints /etc/named/root.hints",
        "/etc/named/root.hints", ". NS a.root-servers.test.\na.root-servers.test. A 10.53.0.1",
        "/etc/resolv.conf", "nameserver 127.0.0.1\nsearch lab.test")));
    PlayerKit.typeFiles(b, files);

    b.note("auth writes its zone in the editor");
    b.send("auth", "mkdir /etc/named");
    b.expect("auth", PlayerKit.PROMPT, "auth: /etc/named");
    editNew(b, "auth", "/etc/named/lab.test.zone", LAB_ZONE);
    b.send("auth", "named check");
    b.expect("auth", "zone lab\\.test\\.: /etc/named/lab\\.test\\.zone OK, serial 1, 5 records", "the zone file is valid");

    b.note("Start the servers");
    startNamed(b, "root", "zone \\.: loaded serial 1", "root serves .");
    startNamed(b, "tld", "zone test\\.: loaded serial 1", "tld serves test.");
    startNamed(b, "auth", "zone lab\\.test\\.: loaded serial 1", "auth serves lab.test.");
    b.send("auth", "httpd 80 &").waitMs(300, "auth: httpd started");
    startNamed(b, "client", "recursion yes", "the client's resolver runs");

    b.note("dig +trace: root -> test. -> lab.test.");
    b.send("client", "dig +trace www.lab.test");
    b.expect("client", "^test\\.\\s+300 IN NS\\s+ns\\.test\\.", "the root refers test. to ns.test");
    b.expect("client", "Received \\d+ bytes from 10\\.53\\.0\\.1#53\\(a\\.root-servers\\.test\\)", "... said by the root");
    b.expect("client", "^lab\\.test\\.\\s+300 IN NS\\s+ns1\\.lab\\.test\\.", "test. refers lab.test. to ns1.lab.test");
    b.expect("client", "Received \\d+ bytes from 10\\.53\\.0\\.2#53\\(ns\\.test\\)", "... said by the TLD server");
    b.expect("client", "^www\\.lab\\.test\\.\\s+60 IN A\\s+10\\.53\\.0\\.3", "www.lab.test is 10.53.0.3");
    b.expect("client", "Received \\d+ bytes from 10\\.53\\.0\\.3#53\\(ns1\\.lab\\.test\\)", "... said by the zone's own server");
    b.expect("client", PlayerKit.PROMPT, "dig done");

    b.note("The stub resolver: search domain, CNAME, NXDOMAIN");
    b.send("client", "dig web.lab.test");
    b.expect("client", "status: NOERROR", "the resolver answers");
    b.expect("client", "^web\\.lab\\.test\\.\\s+60 IN CNAME www\\.lab\\.test\\.", "web is a CNAME for www");
    b.expect("client", PlayerKit.PROMPT, "dig done");
    b.send("client", "curl http://www/");
    b.expect("client", "hello from www\\.lab\\.test", "curl http://www/ works through the search domain");
    b.expect("client", PlayerKit.PROMPT, "curl done");
    b.send("client", "curl http://nosuch/");
    b.expect("client", "Could not resolve host: nosuch \\(name not found\\)", "a name that does not exist fails at once (NXDOMAIN)");
    b.expect("client", PlayerKit.PROMPT, "curl done");

    b.note("Control: stop the zone's server -> SERVFAIL (lame delegation)");
    b.send("auth", "named stop");
    b.expect("auth", "stopping", "auth's named stopped");
    b.send("client", "named flush");
    b.expect("client", "cache flushed", "the client's cache is empty");
    b.send("client", "dig www.lab.test");
    b.expect("client", "status: SERVFAIL", "nobody answers for lab.test.: SERVFAIL");
    b.expect("client", PlayerKit.PROMPT, "dig done");

    b.note("Control: remove the delegation from the test. zone -> NXDOMAIN");
    editDeleteLines(b, "tld", "/etc/named/test.zone", 5, 2, "delete the lab NS and ns1.lab A lines");
    b.send("tld", "named reload");
    b.expect("tld", "zone test\\.: loaded serial 1 \\(3 records\\)", "the TLD reloaded without lab");
    b.send("client", "named flush");
    b.expect("client", "cache flushed", "the client's cache is empty");
    b.send("client", "dig +trace www.lab.test");
    b.expect("client", "www\\.lab\\.test\\. does not exist \\(NXDOMAIN\\)", "the TLD says lab.test. does not exist");
    b.expect("client", PlayerKit.PROMPT, "dig done");
    return b.build();
  }

  static {
    add(lab());
  }

  //?}
}
