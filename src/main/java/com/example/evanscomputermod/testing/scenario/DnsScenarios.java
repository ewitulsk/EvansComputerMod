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

  // ------------------------------------------------------------ the ring (a house PC in a Tech Village)

  /** What a ring scenario works out as it runs: the PC's village, the village it looks up, TTLs seen. */
  static final class Ring {
    int village;
    int target;
    int ttl = -1;
    final List<int[]> cuts = new ArrayList<>();
    final Map<String, Long> typed = new HashMap<>();
  }

  private static final Map<ScenarioRun, Ring> RINGS = Collections.synchronizedMap(new WeakHashMap<>());

  static Ring ring(ScenarioRun r) {
    return RINGS.computeIfAbsent(r, k -> {
      Ring g = new Ring();
      var d = com.example.evanscomputermod.computer.WorldNetwork.get(r.level());
      d.plan(r.level());
      g.village = d.nearest(r.abs(BlockPos.ZERO)).number();
      g.target = next(g.village, g.village == 6 ? 2 : 1);
      return g;
    });
  }

  static int next(int v, int k) {
    return Math.floorMod(v - 1 + k, 10) + 1;
  }

  /** An infrastructure computer's screen (its block's display when attached, else the headless one): for checks and reports. */
  public static String infraScreen(net.minecraft.server.level.ServerLevel level, int village, String role) {
    var d = com.example.evanscomputermod.computer.WorldNetwork.get(level);
    var host = com.example.evanscomputermod.computer.ComputerHost.get(level.getServer(), d.identity(level, village, role));
    if (host.attachment() instanceof com.example.evanscomputermod.block.TerminalBlockEntity be && be.getDisplay() != null)
      return ScenarioRun.screen(be.getDisplay());
    return (host.instance() == null ? "(not running)\n" : "") + ScenarioRun.screen(host.headlessDisplay());
  }

  static com.example.evanscomputermod.computer.DnsPlan plan(ScenarioRun r) {
    return com.example.evanscomputermod.computer.WorldNetwork.get(r.level()).dns(r.level());
  }

  /**
   * The house PC nearest to {@code me} in the nearest Tech Village (its world position): where
   * the ring scenarios start. Thrown when that village was not generated.
   */
  public static BlockPos housePc(net.minecraft.server.level.ServerLevel level, BlockPos me) {
    var d = com.example.evanscomputermod.computer.WorldNetwork.get(level);
    d.plan(level);
    int v = d.nearest(me).number();
    var visit = com.example.evanscomputermod.worldgen.TechVillageLocator.visit(level, v);
    if (visit.network() == null) throw new IllegalStateException("Tech Village " + v + " has no houses on its network");
    return visit.network().terminals().stream()
        .filter(t -> t.role().endsWith(".pc"))
        .map(com.example.evanscomputermod.worldgen.TechNetworkPiece.Terminal::pos)
        .min(Comparator.comparingDouble(p -> p.distSqr(me)))
        .orElseThrow(() -> new IllegalStateException("no house PC in Tech Village " + v));
  }

  /**
   * Type {@code line} at {@code node} (again every {@code everyMs} while the answer is not
   * there) until the output after it matches {@code re}; both are worked out when the step
   * runs. Fails the run at once if the output matches {@code never}.
   */
  static void untilFn(Scenario.Builder b, String node, java.util.function.Function<ScenarioRun, String> line,
                      java.util.function.Function<ScenarioRun, String> re, String never, String what, int everyMs, int timeoutMs) {
    String key = what;
    b.await(r -> {
      Ring g = ring(r);
      long now = System.currentTimeMillis();
      Long at = g.typed.get(key);
      String out = r.latestOutput(node);
      if (at != null && out != null) {
        if (java.util.regex.Pattern.compile(re.apply(r), java.util.regex.Pattern.MULTILINE).matcher(out).find()) {
          // Done once the program is back at the prompt (the next line typed would be lost otherwise).
          if (!promptLast(out)) return "matched; waiting for the prompt";
          g.typed.remove(key);
          return null;
        }
        if (never != null && java.util.regex.Pattern.compile(never, java.util.regex.Pattern.MULTILINE).matcher(out).find()) {
          r.fail(what + ": " + node + " said what it must not (" + never + "):\n" + out.strip());
          return null;
        }
      }
      // Type only at the prompt: a line typed while a program is still exiting is lost.
      if (at == null && out != null && !promptLast(out)) return "waiting for the prompt";
      if (at == null || now - at > everyMs && out != null && promptLast(out)) {
        r.typeLine(node, line.apply(r));
        g.typed.put(key, now);
        return "typed " + line.apply(r);
      }
      return "waiting for " + node + ":\n" + (out == null ? "" : out.strip());
    }, what, timeoutMs);
  }

  /**
   * Is the shell back at its prompt after the last command? (A background job's output may
   * follow the prompt on its line: {@code / > [12:00:01] named: ...}.)
   */
  static boolean promptLast(String out) {
    return java.util.regex.Pattern.compile("^/\\S* >( |$)", java.util.regex.Pattern.MULTILINE).matcher(out).find();
  }

  /** {@link #untilFn} typed once (a long {@code everyMs}). */
  static void typeExpect(Scenario.Builder b, String node, java.util.function.Function<ScenarioRun, String> line,
                         java.util.function.Function<ScenarioRun, String> re, String what, int timeoutMs) {
    untilFn(b, node, line, re, null, what, Integer.MAX_VALUE, timeoutMs);
  }

  private static String oct(int v) {
    return String.valueOf(64 + v);
  }

  /** The start of every ring scenario: the PC has its lease and the village resolver answers for the ring. */
  static void ringReady(Scenario.Builder b) {
    b.note("Stand by a house PC in a Tech Village (a world created with DNS: its home router hands out the village resolver)");
    untilFn(b, "pc", r -> "cat /etc/resolv.conf",
        r -> "^nameserver " + q(plan(r).resolver(ring(r).village)) + "$[\\s\\S]*^search v" + ring(r).village + "\\.ring$", null,
        "the PC's /etc/resolv.conf (from DHCP): the village resolver and the vN.ring search domain", 3_000, 40_000);
    untilFn(b, "pc", r -> "dig web.v" + ring(r).target + ".ring",
        r -> "^web\\.v" + ring(r).target + "\\.ring\\.\\s+\\d+ IN A\\s+100\\." + oct(ring(r).target) + "\\.0\\.10$", null,
        "the village resolver answers for another village's name (the hierarchy is up)", 3_000, 40_000);
  }

  private static Scenario techVillage() {
    var b = Scenario.builder("dns_tech_village",
            "At a house PC in a Tech Village: resolv.conf from DHCP, dig +trace from the root through ring. to another"
                + " village's zone, curl by name and by the search domain, NXDOMAIN at once, an answer from the resolver's cache.")
        .asPlayer().timeLimit(58_000)
        .locate(DnsScenarios::housePc);
    b.host("pc", BlockPos.ZERO, null);
    ringReady(b);
    b.note("dig +trace: the root servers, then ring., then the other village's own server");
    typeExpect(b, "pc", r -> "dig +trace web.v" + ring(r).target + ".ring",
        r -> "^ring\\.\\s+\\d+ IN NS\\s+ns[12]\\.ring\\.ring\\.$[\\s\\S]*Received \\d+ bytes from [\\d.]+#53\\([a-e]\\.root-servers\\.ring\\)"
            + "[\\s\\S]*^v" + ring(r).target + "\\.ring\\.\\s+\\d+ IN NS\\s+ns\\.v" + ring(r).target + "\\.ring\\.$[\\s\\S]*Received \\d+ bytes from "
            + "[\\d.]+#53\\(ns[12]\\.ring\\.ring\\)[\\s\\S]*^web\\.v" + ring(r).target + "\\.ring\\.\\s+300 IN A\\s+100\\." + oct(ring(r).target)
            + "\\.0\\.10$[\\s\\S]*Received \\d+ bytes from 100\\." + oct(ring(r).target) + "\\.0\\.10#53\\(ns\\.v" + ring(r).target + "\\.ring\\)",
        "root -> ring. -> vN.ring.: each referral and who gave it", 20_000);
    b.note("Names in use: another village's web site, then this village's through the search domain");
    typeExpect(b, "pc", r -> "curl http://web.v" + ring(r).target + ".ring/",
        r -> "<h1>Tech Village " + ring(r).target + "</h1>", "curl http://web.vN.ring/ (another village)", 20_000);
    typeExpect(b, "pc", r -> "curl http://web/",
        r -> "<h1>Tech Village " + ring(r).village + "</h1>", "curl http://web/ is this village's site (search domain)", 20_000);
    b.note("Control: a name that does not exist fails at once (NXDOMAIN), not after a timeout");
    typeExpect(b, "pc", r -> "dig nosuch.ring", r -> "status: NXDOMAIN[\\s\\S]*Query time: \\d{1,3} msec", "dig nosuch.ring: NXDOMAIN", 10_000);
    typeExpect(b, "pc", r -> "curl http://nosuch.ring/", r -> "Could not resolve host: nosuch\\.ring \\(name not found\\)",
        "curl says the name does not exist", 10_000);
    b.note("The resolver's cache: the same question twice, the TTL counts down");
    typeExpect(b, "pc", r -> "dig isp.v" + ring(r).target + ".ring", r -> "^isp\\.v" + ring(r).target + "\\.ring\\.\\s+\\d+ IN A", "first answer", 10_000);
    b.await(r -> {
      var m = java.util.regex.Pattern.compile("^isp\\.v\\d+\\.ring\\.\\s+(\\d+) IN A", java.util.regex.Pattern.MULTILINE)
          .matcher(r.latestOutput("pc"));
      if (!m.find()) return "no TTL";
      ring(r).ttl = Integer.parseInt(m.group(1));
      r.say("§7  TTL " + ring(r).ttl);
      return null;
    }, "its TTL", 2_000);
    b.waitMs(2_500, "let 2.5 s pass");
    typeExpect(b, "pc", r -> "dig isp.v" + ring(r).target + ".ring", r -> "^isp\\.v" + ring(r).target + "\\.ring\\.\\s+\\d+ IN A", "second answer", 10_000);
    b.await(r -> {
      var m = java.util.regex.Pattern.compile("^isp\\.v\\d+\\.ring\\.\\s+(\\d+) IN A", java.util.regex.Pattern.MULTILINE)
          .matcher(r.latestOutput("pc"));
      if (!m.find()) return "no TTL";
      int ttl = Integer.parseInt(m.group(1));
      if (ttl >= ring(r).ttl) {
        r.fail("the second answer's TTL " + ttl + " is not lower than the first's " + ring(r).ttl + ": not from the cache");
        return null;
      }
      r.say("§7  TTL " + ttl + " (was " + ring(r).ttl + "): from the resolver's cache");
      return null;
    }, "the second answer comes from the cache: a lower TTL", 2_000);
    return b.build();
  }

  /** {@code /ecm net cut|repair A B} typed by the player (an operator). */
  static void net(Scenario.Builder b, boolean cut, java.util.function.Function<ScenarioRun, int[]> link, String what) {
    b.mutate(r -> {
      int[] l = link.apply(r);
      String cmd = "/ecm net " + (cut ? "cut " : "repair ") + l[0] + " " + l[1];
      var said = r.player().command(cmd);
      r.say("§7  " + cmd + (said.isEmpty() ? "" : ": " + String.join(" / ", said)));
      if (cut) ring(r).cuts.add(l);
      else ring(r).cuts.removeIf(c -> c[0] == l[0] && c[1] == l[1]);
    }, what);
  }

  private static Scenario resilience() {
    var b = Scenario.builder("dns_resilience",
            "At a house PC in a Tech Village: a root server's village cut off the ring, names still resolve through the other"
                + " root letters; the ring cut in two, each half still resolves its .ring names; a name in the other half fails"
                + " (control).")
        .asPlayer().timeLimit(58_000)
        .locate(DnsScenarios::housePc);
    b.host("pc", BlockPos.ZERO, null);
    ringReady(b);
    b.note("Cut a root server's village off the ring (both its fibers): the other letters answer");
    java.util.function.Function<ScenarioRun, Integer> rootVillage = r -> {
      var p = plan(r);
      for (char l : com.example.evanscomputermod.computer.DnsPlan.LETTERS.toCharArray()) {
        int v = p.letterVillage(l);
        if (v != ring(r).village && v != ring(r).target) return v;
      }
      throw new IllegalStateException("no root letter away from the PC");
    };
    net(b, true, r -> new int[] {next(rootVillage.apply(r), -1), rootVillage.apply(r)}, "cut the fiber before the root server's village");
    net(b, true, r -> new int[] {rootVillage.apply(r), next(rootVillage.apply(r), 1)}, "cut the fiber after it");
    untilFn(b, "pc", r -> "dig +trace isp.v" + ring(r).target + ".ring",
        r -> "^isp\\.v" + ring(r).target + "\\.ring\\.\\s+300 IN A\\s+100\\." + oct(ring(r).target) + "\\.0\\.1$", ";; SERVFAIL",
        "dig +trace still finds the name: the root servers still reachable answer", 6_000, 25_000);
    net(b, false, r -> new int[] {next(rootVillage.apply(r), -1), rootVillage.apply(r)}, "repair the first fiber");
    net(b, false, r -> new int[] {rootVillage.apply(r), next(rootVillage.apply(r), 1)}, "repair the second");
    b.note("Cut the ring in two: this half still resolves its own .ring names");
    net(b, true, r -> new int[] {next(ring(r).village, 2), next(ring(r).village, 3)}, "cut the fiber two villages ahead");
    net(b, true, r -> new int[] {next(ring(r).village, 7), next(ring(r).village, 8)}, "cut the fiber three villages behind");
    untilFn(b, "pc", r -> "dig +trace web.v" + next(ring(r).village, 1) + ".ring",
        r -> "^web\\.v" + next(ring(r).village, 1) + "\\.ring\\.\\s+300 IN A\\s+100\\." + oct(next(ring(r).village, 1)) + "\\.0\\.10$", null,
        "a village in this half: its name resolves from the root down within the half", 6_000, 30_000);
    b.note("Control: a village in the other half cannot be reached, so its name cannot be found");
    typeExpect(b, "pc", r -> "dig +trace web.v" + next(ring(r).village, 5) + ".ring",
        r -> ";; SERVFAIL: no nameserver of v" + next(ring(r).village, 5) + "\\.ring\\. answered", "the other half's zone server does not answer: SERVFAIL",
        30_000);
    net(b, false, r -> new int[] {next(ring(r).village, 2), next(ring(r).village, 3)}, "repair the ring");
    net(b, false, r -> new int[] {next(ring(r).village, 7), next(ring(r).village, 8)}, "and the other cut");
    return b.build();
  }

  // ------------------------------------------------------------ registering a name (a rack computer + the house PC)

  /** The Data Center of the PC's village as placed: a rack position (template x 11, y 1, z) in the world. */
  static BlockPos rack(ScenarioRun r, int z) {
    var visit = com.example.evanscomputermod.worldgen.TechVillageLocator.visit(r.level(), ring(r).village);
    if (visit.datacenter() == null) throw new IllegalStateException("Tech Village " + ring(r).village + " has no Data Center");
    return com.example.evanscomputermod.worldgen.TechVillageLocator.world(visit.datacenter(), new BlockPos(11, 1, z));
  }

  /** The way every rack computer of that Data Center faces (the web server's, rack z = 3). */
  static net.minecraft.core.Direction rackFacing(ScenarioRun r) {
    return r.level().getBlockState(rack(r, 3)).getValue(com.example.evanscomputermod.block.TerminalBlock.FACING);
  }

  /** The rack slot (template z) that holds DNS role {@code role} in the PC's village, or -1. */
  static int dnsRackZ(ScenarioRun r, String role) {
    var dns = plan(r);
    String[] slots = {com.example.evanscomputermod.worldgen.ProvisioningProcessor.DNS_SLOT,
        com.example.evanscomputermod.worldgen.ProvisioningProcessor.DNS_SLOT + ".1",
        com.example.evanscomputermod.worldgen.ProvisioningProcessor.DNS_SLOT + ".2"};
    for (int k = 0; k < 3; k++)
      if (role.equals(com.example.evanscomputermod.worldgen.ProvisioningProcessor.dnsSlot(slots[k], dns, ring(r).village))) return 7 + k;
    return -1;
  }

  private static final String MYBASE = "mybase.ecm", RACK_PAGE = "<h1>mybase.ecm, served from a Data Center rack</h1>";

  private static Scenario register() {
    var b = Scenario.builder("dns_register",
            "Register a .ecm name: a computer in a free rack of the village Data Center makes an account, registers"
                + " mybase.ecm and points www at itself; the house PC reaches http://www.mybase.ecm/ and whois shows the"
                + " registrant; the .ecm secondary has it (NOTIFY); with the registry's primary cut off the secondary still"
                + " answers. Controls: the same name again, a wrong password, a reserved name, a change while the registry"
                + " is unreachable.")
        .asPlayer().timeLimit(58_000)
        .locate(DnsScenarios::housePc);
    b.host("pc", BlockPos.ZERO, null);
    b.hostLater("rack", new BlockPos(0, 64, 0));
    ringReady(b);
    b.note("Put a computer into a free rack of this village's Data Center: its top face (eth1) is on the server LAN");
    b.mutate(r -> {
      var facing = rackFacing(r);
      for (int z = 4; z <= 9; z++) {
        BlockPos at = rack(r, z);
        if (!r.level().getBlockState(at).isAir()) continue;
        r.player().place(com.example.evanscomputermod.block.ModBlocks.TERMINAL_BLOCK.get(), at, facing,
            s -> s.getValue(com.example.evanscomputermod.block.TerminalBlock.FACING) == facing);
        r.relocate("rack", at);
        r.player().openScreen(at, facing);
        r.say("§7  a computer in rack slot " + (z - 2) + " at " + at.toShortString());
        return;
      }
      throw new IllegalStateException("no free rack in the Data Center of village " + ring(r).village);
    }, "place a computer in the first free rack and open its screen (it boots)");
    b.await(r -> r.screen("rack").contains("Welcome to Terminal OS") ? null : "booting", "the rack computer has booted", 20_000);
    typeExpect(b, "rack", r -> "dhclient eth1", r -> "bound to 100\\.\\d+\\.0\\.\\d+/24", "a lease on the server LAN (DHCP .100-.199)", 20_000);
    b.await(r -> {
      var m = java.util.regex.Pattern.compile("bound to (100\\.\\d+\\.0\\.\\d+)/24").matcher(r.latestOutput("rack"));
      if (!m.find()) return "no address";
      ring(r).typed.put("ip:" + m.group(1), 0L);
      r.say("§7  the rack computer is " + m.group(1));
      return null;
    }, "its address", 2_000);
    PlayerKit.typeFile(b, "rack", "/index.html", RACK_PAGE);
    typeExpect(b, "rack", r -> "httpd 80 &", r -> "HTTP server listening on port 80", "its web server", 10_000);
    b.note("Register at the registrar (http://nic.ring/) with the domain program");
    typeExpect(b, "rack", r -> "domain account create carol pw1234", r -> "logged in as carol", "an account at nic.ring", 20_000);
    typeExpect(b, "rack", r -> "domain register " + MYBASE, r -> "registered mybase\\.ecm to carol", "domain register mybase.ecm", 20_000);
    typeExpect(b, "rack", r -> "domain set " + MYBASE + " www A " + rackIp(r),
        r -> "www\\.mybase\\.ecm\\. A " + q(rackIp(r)), "www.mybase.ecm points at the rack computer", 20_000);
    typeExpect(b, "rack", r -> "domain set " + MYBASE + " api A " + rackIp(r),
        r -> "api\\.mybase\\.ecm\\. A " + q(rackIp(r)), "and api.mybase.ecm (looked up later)", 20_000);
    b.note("From the house PC: the name works, whois knows the registrant");
    untilFn(b, "pc", r -> "curl http://www." + MYBASE + "/", r -> q(RACK_PAGE), null,
        "curl http://www.mybase.ecm/ reaches the rack computer", 4_000, 25_000);
    typeExpect(b, "pc", r -> "whois " + MYBASE, r -> "Domain Name: MYBASE\\.ECM[\\s\\S]*Registrant: carol", "whois mybase.ecm (whois.nic.ring)", 15_000);
    b.note("Controls at the registrar");
    typeExpect(b, "rack", r -> "domain register " + MYBASE, r -> "mybase\\.ecm is already registered", "the same name again is refused", 15_000);
    typeExpect(b, "rack", r -> "domain --password wrong set " + MYBASE + " www A 1.2.3.4", r -> "wrong handle or password",
        "an update with the wrong password is refused", 15_000);
    typeExpect(b, "rack", r -> "domain register nic.ecm", r -> "'nic' is reserved", "a reserved name is refused", 15_000);
    b.note("The registry's secondary picked the registration up (NOTIFY, then a zone transfer)");
    untilFn(b, "pc", r -> "dig @" + plan(r).ecmSecondaryAddress() + " www." + MYBASE + " +norecurse",
        r -> "flags: qr aa;[\\s\\S]*^www\\.mybase\\.ecm\\.\\s+300 IN A\\s+" + q(rackIp(r)), null,
        "the .ecm secondary answers for www.mybase.ecm itself", 3_000, 15_000);
    b.note("Cut the .ecm registry's primary off: the secondary still answers");
    b.mutate(r -> {
      int primary = plan(r).ecmPrimary();
      if (primary != ring(r).village) {
        for (int[] l : new int[][] {{next(primary, -1), primary}, {primary, next(primary, 1)}}) {
          r.player().command("/ecm net cut " + l[0] + " " + l[1]);
          ring(r).cuts.add(l);
        }
        r.say("§7  /ecm net cut both fibers of village " + primary + " (the .ecm primary)");
      } else {
        // The primary is in this village: unplug it, breaking the rack cable over its slot.
        BlockPos cable = rack(r, dnsRackZ(r, com.example.evanscomputermod.computer.DnsPlan.REGISTRY)).above();
        r.player().breakBlock(cable, rackFacing(r));
        ring(r).typed.put("cable", cable.asLong());
        r.say("§7  broke the rack cable over the .ecm primary (this village's) at " + cable.toShortString());
      }
    }, "cut the .ecm primary off the ring (or off its LAN)");
    typeExpect(b, "rack", r -> "domain set " + MYBASE + " mail A " + rackIp(r), r -> "the registry is unreachable",
        "control: a change cannot reach the registry now", 20_000);
    untilFn(b, "pc", r -> "dig api." + MYBASE, r -> "^api\\.mybase\\.ecm\\.\\s+\\d+ IN A\\s+" + q(rackIp(r)), null,
        "a name never asked before still resolves: the secondary answers", 6_000, 25_000);
    b.mutate(r -> {
      Long cable = ring(r).typed.remove("cable");
      if (cable != null) {
        r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), BlockPos.of(cable), net.minecraft.core.Direction.UP);
      }
      for (int[] l : new ArrayList<>(ring(r).cuts)) r.player().command("/ecm net repair " + l[0] + " " + l[1]);
      ring(r).cuts.clear();
    }, "put the ring (or the cable) back");
    return b.build();
  }

  static String rackIp(ScenarioRun r) {
    return ring(r).typed.keySet().stream().filter(k -> k.startsWith("ip:")).map(k -> k.substring(3)).findFirst().orElse("0.0.0.0");
  }

  // ------------------------------------------------------------ the fiber-tap lab: a player AS with its own nameservers

  /**
   * The tap lab of {@link RouterScenarios#tapLab} between village {@code a} and the next,
   * then the PC's resolver set to village {@code a}'s and the router back at the shell.
   */
  static Scenario.Builder tapDns(int a, String name, String description, String resolver) {
    var b = RouterScenarios.tapLab(a, name, description, 58_000, false, List.of(), List.of());
    b.send("tap", "exit");
    b.expect("tap", PlayerKit.PROMPT, "tap: back at the shell");
    return b;
  }

  /** The resolver a tap scenario's PC uses: village {@code a}'s (from the world's DNS plan when it has one). */
  static String tapResolver(net.minecraft.server.level.ServerLevel level, int a) {
    return level == null ? com.example.evanscomputermod.computer.DnsPlan.lan(a, com.example.evanscomputermod.computer.DnsPlan.RESOLVER_HOST)
        : com.example.evanscomputermod.computer.WorldNetwork.get(level).dns(level).resolver(a);
  }

  /** Where a tap scenario goes: on top of a span of the chord nearest to {@code me}. */
  static int[] nearestTap(net.minecraft.server.level.ServerLevel level, BlockPos me) {
    var d = com.example.evanscomputermod.computer.WorldNetwork.get(level);
    var ring = d.ring(level);
    int best = -1, bestIndex = -1;
    double dist = Double.MAX_VALUE;
    for (int c = 0; c < 10; c++) {
      int i = RouterScenarios.tapIndex(ring, c, (x, z) -> Math.hypot(x - me.getX(), z - me.getZ()) < 64, at -> true);
      if (i < 0) continue;
      int[] p = ring.path(c)[i];
      double dd = Math.hypot(p[0] - me.getX(), p[2] - me.getZ());
      if (dd < dist) {
        dist = dd;
        best = c;
        bestIndex = i;
      }
    }
    if (best < 0) throw new IllegalStateException("stand on the Tech Village fiber ring (a span within 64 blocks) to tap it");
    int[] p = ring.path(best)[bestIndex];
    return new int[] {best + 1, p[0], p[1] + 1, p[2]};
  }

  static Scenario.Builder tapped(Scenario.Builder b) {
    return b.builtAt((level, me) -> {
      int[] t = nearestTap(level, me);
      return new BlockPos(t[1], t[2], t[3]);
    });
  }

  public static Scenario ownNameserver(int a, String resolver) {
    String lan = "10.200." + a + ".";
    String base = "tapbase.ecm";
    var b = tapDns(a, "dns_own_nameserver",
        "A player AS on the fiber-tap lab runs its own named for tapbase.ecm, registers it with a delegation and glue:"
            + " dig +trace shows the fourth hop to the player's server; a record changed there is served from the resolver's"
            + " cache until its TTL runs out. Control: the player's named stopped, the name fails (lame delegation) while"
            + " everything else keeps resolving.", resolver);
    tapped(b);
    b.note("The PC's resolver (village " + a + "'s), its named.conf and the zone, typed in");
    PlayerKit.typeFiles(b, Map.of("pc", new TreeMap<>(Map.of(
        "/etc/resolv.conf", "nameserver " + resolver,
        "/etc/named.conf", "zone " + base + " primary /etc/named/" + base + ".zone",
        "/etc/named/" + base + ".zone", String.join("\n", "$ORIGIN " + base + ".", "$TTL 15", "@ SOA ns1 admin 1 60 30 600 15",
            "@ NS ns1", "ns1 A " + lan + "10", "www A " + lan + "10", "mail A " + lan + "10")))));
    startNamed(b, "pc", "zone tapbase\\.ecm\\.: loaded serial 1", "the player's named serves tapbase.ecm");
    b.note("Register it, delegated to this named (glue: its address)");
    typeExpect(b, "pc", r -> "domain account create dave pw1234", r -> "logged in as dave", "an account at nic.ring", 20_000);
    typeExpect(b, "pc", r -> "domain register " + base, r -> "registered tapbase\\.ecm to dave", "domain register tapbase.ecm", 20_000);
    typeExpect(b, "pc", r -> "domain ns " + base + " ns1." + base + "=" + lan + "10",
        r -> "tapbase\\.ecm delegated to ns1\\.tapbase\\.ecm\\. \\(" + q(lan + "10") + "\\)", "domain ns: delegation with glue", 20_000);
    untilFn(b, "pc", r -> "dig +trace www." + base,
        r -> "^ecm\\.\\s+\\d+ IN NS\\s+ns[12]\\.ecm\\.ring\\.$[\\s\\S]*^tapbase\\.ecm\\.\\s+\\d+ IN NS\\s+ns1\\.tapbase\\.ecm\\.$[\\s\\S]*"
            + "Received \\d+ bytes from [\\d.]+#53\\(ns[12]\\.ecm\\.ring\\)[\\s\\S]*^www\\.tapbase\\.ecm\\.\\s+15 IN A\\s+" + q(lan + "10")
            + "$[\\s\\S]*Received \\d+ bytes from " + q(lan + "10") + "#53\\(ns1\\.tapbase\\.ecm\\)", ";; SERVFAIL",
        "dig +trace: root -> ecm. -> tapbase.ecm. (the player's server)", 4_000, 20_000);
    b.note("Change a record: the resolver keeps the old answer until its TTL (15 s) runs out");
    typeExpect(b, "pc", r -> "dig www." + base, r -> "^www\\.tapbase\\.ecm\\.\\s+\\d+ IN A\\s+" + q(lan + "10"), "the resolver caches www", 15_000);
    PlayerKit.typeFile(b, "pc", "/etc/named/" + base + ".zone", "$ORIGIN " + base + ".", "$TTL 15", "@ SOA ns1 admin 2 60 30 600 15",
        "@ NS ns1", "ns1 A " + lan + "10", "www A " + lan + "20", "mail A " + lan + "10");
    typeExpect(b, "pc", r -> "named reload", r -> "zone tapbase\\.ecm\\.: loaded serial 2", "named reload: www is " + lan + "20 now", 10_000);
    typeExpect(b, "pc", r -> "dig www." + base, r -> "^www\\.tapbase\\.ecm\\.\\s+([0-9]|1[0-4]) IN A\\s+" + q(lan + "10"),
        "still the old answer, from the resolver's cache (TTL counting down)", 10_000);
    untilFn(b, "pc", r -> "dig www." + base, r -> "^www\\.tapbase\\.ecm\\.\\s+\\d+ IN A\\s+" + q(lan + "20"), null,
        "once the TTL ran out: the new answer", 2_000, 20_000);
    b.note("Control: stop the player's named");
    typeExpect(b, "pc", r -> "named stop", r -> "stopping", "named stopped", 10_000);
    typeExpect(b, "pc", r -> "dig nosuch." + base, r -> "status: SERVFAIL", "a name in tapbase.ecm fails: SERVFAIL (lame delegation)", 20_000);
    typeExpect(b, "pc", r -> "dig web.v" + a + ".ring", r -> "^web\\.v" + a + "\\.ring\\.\\s+\\d+ IN A", "everything else still resolves", 15_000);
    return b.build();
  }

  public static Scenario tld(int a, String resolver, String otherResolver) {
    String lan = "10.200." + a + ".";
    var b = tapDns(a, "dns_tld",
        "A new top-level domain: the fiber-tap lab runs .base on two nameservers (the tap router and the PC, which also"
            + " serves shop.base), applies to IANA, and the root letters delegate it: dig +trace shows root -> base. ->"
            + " shop.base., another village's resolver finds www.shop.base. Controls: .test is refused, an application"
            + " with a lame nameserver is refused, before the application the root says .base does not exist.", resolver);
    tapped(b);
    String baseZone = String.join("\n", "$ORIGIN base.", "$TTL 60", "@ SOA ns1 admin 1 60 30 600 30", "@ NS ns1", "@ NS ns2",
        "ns1 A " + lan + "1", "ns2 A " + lan + "10", "shop NS ns.shop", "ns.shop A " + lan + "10");
    b.note("Both nameservers' files, typed in: .base on the tap router (ns1) and the PC (ns2); shop.base on the PC");
    PlayerKit.typeFiles(b, new LinkedHashMap<>(Map.of(
        "tap", new TreeMap<>(Map.of("/etc/named.conf", "zone base primary /etc/named/base.zone", "/etc/named/base.zone", baseZone)),
        "pc", new TreeMap<>(Map.of(
            "/etc/resolv.conf", "nameserver " + resolver,
            "/etc/named.conf", "zone base primary /etc/named/base.zone\nzone shop.base primary /etc/named/shop.zone",
            "/etc/named/base.zone", baseZone,
            "/etc/named/shop.zone", String.join("\n", "$ORIGIN shop.base.", "$TTL 60", "@ SOA ns admin 1 60 30 600 30", "@ NS ns",
                "ns A " + lan + "10", "www A " + lan + "10"))))));
    startNamed(b, "pc", "zone shop\\.base\\.: loaded serial 1", "the PC serves base. and shop.base.");
    b.note("Before: the root says .base does not exist");
    typeExpect(b, "pc", r -> "dig +trace www.shop.base", r -> "www\\.shop\\.base\\. does not exist \\(NXDOMAIN\\)", "NXDOMAIN at the root", 20_000);
    b.note("Apply to IANA (http://iana.ring/)");
    typeExpect(b, "pc", r -> "domain --handle erin --password pw1234 tld apply test ns1.base=" + lan + "1",
        r -> "\\.test is reserved and never delegated", "control: .test is refused", 20_000);
    typeExpect(b, "pc", r -> "domain --handle erin --password pw1234 tld apply base ns1.base=" + lan + "1 ns2.base=" + lan + "10",
        r -> "ns1\\.base\\. \\(" + q(lan + "1") + "\\) did not answer .*lame", "control: ns1 (the tap router) runs no named yet: lame, refused",
        20_000);
    startNamed(b, "tap", "zone base\\.: loaded serial 1", "the tap router serves base. too");
    typeExpect(b, "pc", r -> "domain --handle erin --password pw1234 tld apply base ns1.base=" + lan + "1 ns2.base=" + lan + "10",
        r -> "\\.base is delegated to ns1\\.base\\. ns2\\.base\\.", "IANA puts .base in the root zone", 20_000);
    b.note("After: the root letters delegate base. (they got the new root zone by NOTIFY + AXFR)");
    untilFn(b, "pc", r -> "dig +trace www.shop.base",
        r -> "^base\\.\\s+3600 IN NS\\s+ns[12]\\.base\\.$[\\s\\S]*Received \\d+ bytes from [\\d.]+#53\\([a-e]\\.root-servers\\.ring\\)"
            + "[\\s\\S]*^shop\\.base\\.\\s+60 IN NS\\s+ns\\.shop\\.base\\.$[\\s\\S]*^www\\.shop\\.base\\.\\s+60 IN A\\s+" + q(lan + "10"), null,
        "dig +trace: root -> base. -> shop.base.", 3_000, 20_000);
    typeExpect(b, "pc", r -> "dig @" + otherResolver + " www.shop.base", r -> "^www\\.shop\\.base\\.\\s+\\d+ IN A\\s+" + q(lan + "10"),
        "another village's resolver (" + otherResolver + ") finds www.shop.base", 20_000);
    typeExpect(b, "pc", r -> "domain tld show base", r -> "TLD \\.base: delegated .* to erin", "domain tld show base", 15_000);
    typeExpect(b, "pc", r -> "domain --handle erin --password pw1234 tld revoke base", r -> "\\.base removed from the root zone",
        "(revoke it again: the lab leaves the root as it found it)", 15_000);
    return b.build();
  }

  /** The spawnable entry of a tap scenario: shown for chord 1, specialised to the chord nearest the player. */
  private static Scenario tapEntry(java.util.function.BiFunction<Integer, net.minecraft.server.level.ServerLevel, Scenario> make) {
    Scenario s = make.apply(1, null);
    s.specialize = (level, me) -> make.apply(nearestTap(level, me)[0], level);
    return s;
  }

  static {
    add(lab());
    add(techVillage());
    add(resilience());
    add(register());
    add(tapEntry((a, level) -> ownNameserver(a, tapResolver(level, a))));
    add(tapEntry((a, level) -> tld(a, tapResolver(level, a), tapResolver(level, next(a, 5)))));
  }

  //?}
}
