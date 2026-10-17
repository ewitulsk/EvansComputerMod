package com.example.evanscomputermod.testing.scenario;

import com.example.evanscomputermod.computer.*;
import java.util.*;
import net.minecraft.core.BlockPos;

/**
 * Playable router laboratories, shared with the {@code ecm_router_scenarios} GameTests.
 *
 * <p>Built and operated by the scenario player (AGENTS.md: scenarios are real player
 * procedures, docs/SCENARIO_AUDIT.md): the computers and every cable between their ports
 * are placed by hand, and each computer's configuration is typed in the way a player sets
 * one up: addresses and routes with {@code ifconfig} / {@code ip route add} (the kernel
 * saves them to network.cfg), {@code router on} and the router configuration pasted into the
 * router CLI and saved with {@code write memory} (router.cfg), services started by typing
 * their command lines, and other files written from Python. A lab's link is cut by
 * breaking a block of its cable and repaired by placing one back.
 *
 * <p>Ports: every computer's screen faces north, so eth0 = DOWN, eth1 = UP, eth2 = SOUTH (its back).
 * Computers stand 4 blocks apart on x; "down-down" runs along y=0 under two neighbours,
 * "up-down" runs from one computer's top over to the next one's bottom.
 */
public final class RouterScenarios {
  public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

  //? if <=1.21.1 {

  private static final net.minecraft.core.Direction DOWN = net.minecraft.core.Direction.DOWN,
      UP = net.minecraft.core.Direction.UP, NORTH = net.minecraft.core.Direction.NORTH,
      SOUTH = net.minecraft.core.Direction.SOUTH, EAST = net.minecraft.core.Direction.EAST,
      WEST = net.minecraft.core.Direction.WEST;

  private static void add(Scenario s) {
    ALL.put(s.name, s);
  }

  /** Computers named {@code names}, 4 blocks apart on x, built and operated by the player. */
  private static Scenario.Builder nodes(String name, String description, String... names) {
    var b = Scenario.builder(name, description).timeLimit(55_000).asPlayer();
    for (int i = 0; i < names.length; i++) b.host(names[i], new BlockPos(i * 4, 1, 0), null);
    return b;
  }

  private static Map<String, String> router(String body) {
    return Map.of(
        "services.cfg",
        "router on\n",
        "router.cfg",
        "configure terminal\nip routing\n" + body + "\nend\n");
  }

  private static String address(int port, String ip) {
    return "interface eth" + port + "\nip address " + ip + "\nexit\n";
  }

  private static Map<String, String> host(String ip, String gateway) {
    return Map.of(
        "network.cfg",
        "iface eth0 "
            + ip
            + "\n"
            + (gateway == null ? "" : "route default via " + gateway + " dev eth0\n"));
  }

  /** A cable under computers {@code a} and {@code b} (at x {@code xa} and {@code xa + 4}): a's eth0 to b's eth0. */
  private static void downDown(Scenario.Builder b, String link, String a, String b2, int xa) {
    b.link(link, a, DOWN, b2, DOWN, Scenario.Path.from(new BlockPos(xa, 0, 0)).go(EAST, 4));
  }

  /** A cable from the top of {@code a} (eth1, at x {@code xa}) to the bottom of {@code b} (eth0, at x {@code xa + 4}). */
  private static void upDown(Scenario.Builder b, String link, String a, String b2, int xa) {
    b.link(link, a, UP, b2, DOWN,
        Scenario.Path.from(new BlockPos(xa, 2, 0)).go(EAST, 2).go(DOWN, 2).go(EAST, 2));
  }

  private static List<String> lines(String text) {
    List<String> out = new ArrayList<>();
    for (String l : text.split("\n")) if (!l.isBlank()) out.add(l.strip());
    return out;
  }

  private static final Set<String> STARTUP = Set.of("network.cfg", "services.cfg", "router.cfg");

  /**
   * Type {@code node}'s configuration in, as a player sets the computer up: other files
   * from Python, then the network.cfg lines as {@code ifconfig} / {@code ip route add}, the
   * services.cfg lines as commands, {@code router on} with router.cfg pasted into the router
   * CLI and saved, and last the DHCP clients ({@code dhclient}), which need their server
   * up. The kernel and the router write network.cfg and router.cfg back themselves.
   */
  private static void provision(Scenario.Builder b, String node, Map<String, String> files) {
    typeOtherFiles(b, Map.of(node, files));
    provisionStartup(b, node, files);
  }

  /** The files that aren't startup configuration (web pages, /etc/chat.conf), typed from Python on every computer at once. */
  private static void typeOtherFiles(Scenario.Builder b, Map<String, Map<String, String>> files) {
    Map<String, Map<String, String>> typed = new LinkedHashMap<>();
    for (var n : files.entrySet())
      for (var e : new TreeMap<>(n.getValue()).entrySet())
        if (!STARTUP.contains(e.getKey()))
          typed.computeIfAbsent(n.getKey(), k -> new LinkedHashMap<>()).put("/" + e.getKey(), e.getValue().strip());
    if (!typed.isEmpty()) PlayerKit.typeFiles(b, typed);
  }

  private static void provisionStartup(Scenario.Builder b, String node, Map<String, String> files) {
    List<String> dhcp = new ArrayList<>();
    for (String l : lines(files.getOrDefault("network.cfg", ""))) {
      String[] w = l.split("\\s+");
      if (w.length >= 3 && w[0].equals("iface") && w[2].equals("dhcp")) {
        dhcp.add(w[1]);
      } else if (w.length >= 3 && w[0].equals("iface")) {
        b.send(node, "ifconfig " + w[1] + " " + w[2]);
        b.expect(node, w[1] + ": inet " + java.util.regex.Pattern.quote(w[2]), node + " has " + w[2] + " on " + w[1]);
        b.expect(node, PlayerKit.PROMPT, node + ": shell");
      } else if (w.length >= 6 && w[0].equals("route") && w[1].equals("default")) {
        b.send(node, "ip route add default via " + w[3] + " dev " + w[5]);
        b.expect(node, "Default route added", node + ": default route via " + w[3]);
        b.expect(node, PlayerKit.PROMPT, node + ": shell");
      }
      // "dns" lines: nothing to type (the labs reach servers by address)
    }
    List<String> services = lines(files.getOrDefault("services.cfg", ""));
    for (String l : services) {
      if (l.equals("router on")) continue;
      b.send(node, l);
      // A background job's first line can land on the prompt's row, so only foreground lines wait for the prompt.
      if (!l.endsWith("&")) b.expect(node, PlayerKit.PROMPT, node + ": " + l);
      else b.waitMs(300, node + ": " + l + " started");
    }
    String cfg = files.get("router.cfg");
    if (cfg != null || services.contains("router on")) {
      b.send(node, "router on");
      b.expect(node, "Router started", node + ": router service");
      if (cfg != null) {
        b.send(node, "router");
        b.expect(node, "router#", node + ": router CLI");
        List<String> paste = new ArrayList<>(lines(cfg));
        if (!paste.get(paste.size() - 1).equals("end")) paste.add("end");
        paste.add("write memory");
        b.paste(node, paste);
        b.expect(node, "Configuration saved", node + ": configuration pasted and saved (router.cfg)");
        b.send(node, "exit");
        b.expect(node, PlayerKit.PROMPT, node + ": shell");
      }
    }
    for (String nic : dhcp) {
      b.send(node, "dhclient " + nic);
      b.expect(node, "bound to", node + " leased an address on " + nic);
      b.expect(node, PlayerKit.PROMPT, node + ": shell");
    }
  }

  /** {@link #provision} for each computer, in the given order (DHCP servers before their clients). */
  private static void provisionAll(Scenario.Builder b, LinkedHashMap<String, Map<String, String>> files) {
    b.note("Set each computer up (typed, as a player configures it)");
    typeOtherFiles(b, files);
    for (var e : files.entrySet()) provisionStartup(b, e.getKey(), e.getValue());
  }

  @SafeVarargs
  private static <K, V> LinkedHashMap<K, V> ordered(Map.Entry<K, V>... entries) {
    LinkedHashMap<K, V> m = new LinkedHashMap<>();
    for (var e : entries) m.put(e.getKey(), e.getValue());
    return m;
  }

  private static void ping(
      Scenario.Builder b, String node, String address, int count, int replies, String purpose) {
    b.ping(node, address, count, replies, purpose);
    b.expect(node, "/ >", "ping returned to shell");
  }

  private static Scenario home() {
    var b =
        nodes(
            "router_home",
            "DHCP LAN, WAN NAT, route/ARP/lease displays, traceroute, and forwarding-off control.",
            "client",
            "router",
            "server");
    downDown(b, "lan", "client", "router", 0);
    upDown(b, "wan", "router", "server", 4);
    provisionAll(
        b,
        ordered(
            Map.entry("server", host("10.90.0.3/24", null)),
            Map.entry(
                "router",
                router(
                    "interface eth0\n"
                        + "ip address 192.168.90.1/24\n"
                        + "ip nat inside\n"
                        + "exit\n"
                        + "interface eth1\n"
                        + "ip address 10.90.0.2/24\n"
                        + "ip nat outside\n"
                        + "exit\n"
                        + "dhcp-server vrf default\n"
                        + "pool lan\n"
                        + "range 192.168.90.10 192.168.90.20\n"
                        + "default-router 192.168.90.1\n"
                        + "dns-server 1.1.1.1\n"
                        + "lease 3600\n"
                        + "enable\n")),
            Map.entry("client", Map.of("network.cfg", "iface eth0 dhcp\n"))));
    b.note("Cables: client eth0 (bottom) to router eth0 (bottom); router eth1 (top) to server eth0 (bottom).");
    b.send("client", "ifconfig");
    b.expect("client", "192.168.90.10", "DHCP lease");
    b.expect("client", "/ >", "ifconfig returned to shell");
    ping(b, "client", "10.90.0.3", 2, 2, "NAT replies without a private return route");
    b.send("client", "traceroute 10.90.0.3 4");
    b.expect("client", "^2\\s+10\\.90\\.0\\.3\\s*$", "traceroute reaches server at hop two");
    b.expect("client", "/ >", "traceroute returned to shell");
    b.send("router", "router");
    b.expect("router", "router#", "router CLI");
    for (String command :
        new String[] {
          "show ip route",
          "show arp",
          "show dhcp-server leases",
          "show ip nat translations",
          "show running-config"
        }) b.send("router", command);
    b.send(
        "router",
        "configure terminal",
        "ip route 203.0.113.0/24 10.90.0.3 eth1",
        "end",
        "write memory");
    b.send("router", "exit");
    b.expect("router", "/ >", "shell");
    b.send("router", "router off");
    b.expect("router", "Router stopped", "off control");
    ping(b, "client", "10.90.0.3", 1, 0, "no forwarding while off");
    b.send("router", "router on");
    b.expect("router", "Router started", "restart reads saved config");
    b.send("router", "router", "show running-config");
    b.expect("router", "ip route 203.0.113.0/24 10.90.0.3 eth1", "saved edit survives restart");
    b.send("router", "exit");
    b.expect("router", "/ >", "shell restored");
    ping(b, "client", "10.90.0.3", 1, 1, "forwarding restored");
    return b.build();
  }

  private static Scenario portForward() {
    var b =
        nodes(
            "router_port_forward",
            "HTTP through a static NAT port forward; LAN hairpin access and a closed-port control.",
            "lan",
            "router",
            "wan");
    downDown(b, "lan", "lan", "router", 0);
    upDown(b, "wan", "router", "wan", 4);
    var lan = new HashMap<>(host("192.168.91.10/24", "192.168.91.1"));
    lan.put("services.cfg", "httpd 80 &\n");
    lan.put("index.html", "<h1>router-forward-ok</h1>\n");
    provisionAll(
        b,
        ordered(
            Map.entry("lan", lan),
            Map.entry("wan", host("10.91.0.3/24", null)),
            Map.entry(
                "router",
                router(
                    "interface eth0\n"
                        + "ip address 192.168.91.1/24\n"
                        + "ip nat inside\n"
                        + "exit\n"
                        + "interface eth1\n"
                        + "ip address 10.91.0.2/24\n"
                        + "ip nat outside\n"
                        + "exit\n"
                        + "ip nat inside source static tcp 192.168.91.10 80 8080\n"))));
    b.send("wan", "curl http://10.91.0.2:8080/index.html");
    b.expect("wan", "router-forward-ok", "WAN static forward");
    b.send("lan", "curl http://10.91.0.2:8080/index.html");
    b.expect("lan", "router-forward-ok", "hairpin reply");
    b.send("wan", "curl http://10.91.0.2:8081/index.html");
    b.expect("wan", "Failed to connect", "unforwarded-port control");
    return b.build();
  }

  private static Scenario staticRoutes() {
    var b =
        nodes(
            "router_static",
            "Static routes, forwarding, TTL/traceroute, and routing-disabled control.",
            "client",
            "r1",
            "r2",
            "server");
    downDown(b, "lan1", "client", "r1", 0);
    upDown(b, "transit", "r1", "r2", 4);
    upDown(b, "lan2", "r2", "server", 8);
    provisionAll(
        b,
        ordered(
            Map.entry("client", host("10.92.1.10/24", "10.92.1.1")),
            Map.entry(
                "r1",
                router(
                    address(0, "10.92.1.1/24")
                        + address(1, "172.30.92.1/30")
                        + "ip route 10.92.2.0/24 172.30.92.2\n")),
            Map.entry(
                "r2",
                router(
                    address(0, "172.30.92.2/30")
                        + address(1, "10.92.2.1/24")
                        + "ip route 10.92.1.0/24 172.30.92.1\n")),
            Map.entry("server", host("10.92.2.10/24", "10.92.2.1"))));
    ping(b, "client", "10.92.2.10", 2, 2, "two-router static path");
    b.send("client", "traceroute 10.92.2.10 5");
    b.expect("client", "^3\\s+10\\.92\\.2\\.10\\s*$", "TTL probes reach destination at hop three");
    b.expect("client", "/ >", "traceroute returned to shell");
    b.send("r1", "router", "show ip route", "configure terminal", "no ip routing", "end", "exit");
    b.expect("r1", "/ >", "shell");
    ping(b, "client", "10.92.2.10", 1, 0, "routing disabled control");
    return b.build();
  }

  /**
   * BGP labs: {@code count} routers in a ring (r_i eth1 on top to r_i+1 eth0 underneath; the
   * last one's eth1 back over the north side to r1's eth0), a client on r1's eth2 (south face)
   * and a server on the last router's eth2.
   */
  private static Scenario bgp(int count, boolean policies) {
    String name =
        policies ? "router_bgp_policy" : count == 2 ? "router_bgp_pair" : "router_bgp_ring";
    String[] names = new String[count + 2];
    names[0] = "client";
    for (int i = 1; i <= count; i++) names[i] = "r" + i;
    names[count + 1] = "server";
    var b =
        nodes(
            name,
            "BGP "
                + count
                + "-AS lab: source-bound TCP 179, IPv4 routes, "
                + (policies
                    ? "prefix filters, local preference, MED and communities with a denied-prefix"
                        + " control."
                    : "route diagnostics and withdrawals; the ring cuts, isolates, then repairs."),
            names);
    int last = 4 * count;
    for (int i = 1; i < count; i++) upDown(b, "ring" + i, "r" + i, "r" + (i + 1), 4 * i);
    // The ring's last link: r_count's top, back west along the north side, up into r1's bottom.
    b.link("ring" + count, "r" + count, UP, "r1", DOWN,
        Scenario.Path.from(new BlockPos(last, 2, 0)).go(UP, 1).go(NORTH, 2).go(WEST, last - 4)
            .go(DOWN, 4).go(SOUTH, 2).go(UP, 1));
    b.link("client", "client", DOWN, "r1", SOUTH,
        Scenario.Path.from(new BlockPos(0, 0, 0)).go(SOUTH, 2).go(EAST, 4).go(UP, 1).go(NORTH, 1));
    b.link("server", "r" + count, SOUTH, "server", DOWN,
        Scenario.Path.from(new BlockPos(last, 1, 1)).go(SOUTH, 1).go(EAST, 4).go(DOWN, 1).go(NORTH, 2));
    LinkedHashMap<String, Map<String, String>> files = new LinkedHashMap<>();
    for (int i = 1; i <= count; i++) {
      int prev = i == 1 ? count : i - 1, next = i == count ? 1 : i + 1;
      String body =
          address(0, "172.29." + prev + ".2/30")
              + address(1, "172.29." + i + ".1/30")
              + address(2, "100." + (90 + i) + ".0.1/24");
      if (policies && i == 1)
        body +=
            "ip prefix-list ACCEPT seq 10 permit 100.92.0.0/24\n"
                + "route-map INBOUND permit 10\n"
                + "match ip address prefix-list ACCEPT\n"
                + "set local-preference 150\n"
                + "set metric 20\n"
                + "set community 65101:10\n"
                + "exit\n";
      body +=
          "router bgp "
              + (4_200_000_000L + i)
              + "\nbgp router-id 100."
              + (90 + i)
              + ".0.1\ntimers bgp 1 3\nneighbor 172.29."
              + prev
              + ".1 remote-as "
              + (4_200_000_000L + prev)
              + "\nneighbor 172.29."
              + i
              + ".2 remote-as "
              + (4_200_000_000L + next)
              + "\naddress-family ipv4 unicast\nneighbor 172.29."
              + prev
              + ".1 activate\nneighbor 172.29."
              + i
              + ".2 activate\nnetwork 100."
              + (90 + i)
              + ".0.0/24\n";
      if (policies && i == 1)
        body +=
            "neighbor 172.29."
                + i
                + ".2 route-map INBOUND in\nneighbor 172.29."
                + prev
                + ".1 route-map INBOUND in\n";
      if (policies && i == 2) body += "redistribute connected\n";
      files.put("r" + i, router(body));
    }
    files.put("client", host("100.91.0.10/24", "100.91.0.1"));
    files.put("server", host("100." + (90 + count) + ".0.10/24", "100." + (90 + count) + ".0.1"));
    provisionAll(b, files);
    String target = "100." + (90 + count) + ".0.10";
    b.until("client", "ping " + target + " -n 1", "^1 packets sent, 1 received", "BGP converged: the client reaches the server");
    b.expect("client", "/ >", "ping returned to shell");
    ping(b, "client", target, 2, 2, "learned route and return path");
    b.send("r1", "router", "show bgp ipv4 unicast summary");
    b.expect("r1", "Established", "BGP established");
    b.send("r1", "show bgp ipv4 unicast", "show ip route");
    if (policies) {
      b.send("r1", "show bgp ipv4 unicast");
      b.expect("r1", "100.92.0.0/24", "allowed aggregate");
      b.expect("r1", "local-pref 150 MED 20 community 65101:10", "inbound attributes applied");
      b.expect("r1", "router#", "policy table completed");
      b.await(
          r -> {
            var output = r.latestOutput("r1");
            if (output != null
                && java.util.regex.Pattern.compile("(?m)^172\\.29\\.").matcher(output).find())
              r.fail("INBOUND leaked denied transit prefixes:\n" + output);
            return null;
          },
          "Transit 172.29 prefixes are redistributed by r2 but denied by INBOUND: none in show bgp ipv4 unicast",
          1_000);
    }
    b.send("r1", "exit");
    b.expect("r1", "/ >", "shell");
    if (count == 10) {
      b.cut("ring" + count, 6, "Break a block of the r" + count + "-r1 cable: the direct link is cut.");
      b.waitMs(3500, "withdrawal and nine-hop route");
      ping(b, "client", target, 1, 1, "alternate ring path");
      b.cut("ring1", 1, "Break a block of the r1-r2 cable as well: r1 is isolated.");
      b.waitMs(3500, "isolation withdrawal");
      ping(b, "client", target, 1, 0, "no route when isolated");
      b.mend("ring" + count, 6, "Put the r" + count + "-r1 cable back.");
      b.mend("ring1", 1, "Put the r1-r2 cable back.");
      b.waitMs(3500, "repair convergence");
      b.until("client", "ping " + target + " -n 1", "^1 packets sent, 1 received", "repaired connectivity");
    }
    return b.build();
  }

  private static Scenario wanDhcp() {
    var b =
        nodes(
            "router_wan_dhcp",
            "Unconfigured WAN DHCP learns its address/default/DNS; a second DHCP pool serves the"
                + " private LAN, with NAT across both.",
            "client",
            "router",
            "isp");
    downDown(b, "lan", "client", "router", 0);
    upDown(b, "wan", "router", "isp", 4);
    var isp =
        new HashMap<>(
            router(
                address(0, "10.93.0.1/24")
                    + "dhcp-server vrf default\n"
                    + "pool wan\n"
                    + "range 10.93.0.10 10.93.0.20\n"
                    + "default-router 10.93.0.1\n"
                    + "dns-server 1.1.1.1\n"
                    + "enable\n"));
    isp.put("services.cfg", "router on\nhttpd 80 &\n");
    isp.put("index.html", "wan-dhcp-ok\n");
    provisionAll(
        b,
        ordered(
            Map.entry("isp", isp),
            Map.entry(
                "router",
                router(
                    "interface eth0\n"
                        + "ip address 192.168.93.1/24\n"
                        + "ip nat inside\n"
                        + "exit\n"
                        + "interface eth1\n"
                        + "ip dhcp\n"
                        + "ip nat outside\n"
                        + "exit\n"
                        + "dhcp-server vrf default\n"
                        + "pool lan\n"
                        + "range 192.168.93.10 192.168.93.20\n"
                        + "default-router 192.168.93.1\n"
                        + "dns-server 1.1.1.1\n"
                        + "enable\n")),
            Map.entry("client", Map.of("network.cfg", "iface eth0 dhcp\n"))));
    b.until("router", "ifconfig", "10\\.93\\.0\\.10", "WAN lease");
    b.expect("router", "/ >", "ifconfig returned to shell");
    b.send("router", "ip route");
    b.expect("router", "10.93.0.1", "DHCP default gateway");
    b.expect("router", "/ >", "route display returned to shell");
    b.send("client", "curl http://10.93.0.1/index.html");
    b.expect("client", "wan-dhcp-ok", "HTTP through leased WAN NAT");
    return b.build();
  }

  /** Per run: the local web server standing in for "a website on the internet", and its URL. */
  private static final Map<ScenarioRun, java.net.ServerSocket> FIXTURES = new java.util.WeakHashMap<>();
  private static final Map<ScenarioRun, String> FIXTURE_URL = new java.util.WeakHashMap<>();

  private static Scenario internet() {
    var b =
        nodes(
            "router_internet",
            "The Internet Gateway block bridges real host TCP sockets: a client cabled to it, with a static"
                + " address (the gateway serves no DHCP), curls a web server on the host machine. An"
                + " isolated local HTTP fixture avoids reliance on a public website; manual mode can also"
                + " curl a public plain-HTTP URL.",
            "client");
    List<BlockPos> run = List.of(new BlockPos(0, 0, 0), new BlockPos(0, 0, 1));
    BlockPos gateway = new BlockPos(0, 0, 2);
    b.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            List<BlockPos> f = new ArrayList<>(run);
            f.add(gateway);
            return f;
          }

          public void build(ScenarioRun r) {
            // The player's Internet Gateway, cabled to the client's bottom face (eth0).
            for (BlockPos p : run) r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), r.abs(p), UP);
            r.player().place(com.example.evanscomputermod.block.ModBlocks.INTERNET_GATEWAY.get(), r.abs(gateway), SOUTH);
            // "The internet": a web server on the host machine (outside the game).
            try {
              var listener = new java.net.ServerSocket(0);
              FIXTURES.put(r, listener);
              var worker =
                  new Thread(
                      () -> {
                        while (!listener.isClosed())
                          try (var socket = listener.accept()) {
                            socket.setSoTimeout(3000);
                            var input =
                                new java.io.BufferedReader(
                                    new java.io.InputStreamReader(socket.getInputStream()));
                            while (true) {
                              var line = input.readLine();
                              if (line == null || line.isEmpty()) break;
                            }
                            socket
                                .getOutputStream()
                                .write(
                                    "HTTP/1.0 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nhost-socket-ok\n"
                                        .getBytes(java.nio.charset.StandardCharsets.US_ASCII));
                          } catch (java.io.IOException e) {
                            if (!listener.isClosed())
                              com.example.evanscomputermod.EvansComputerMod.LOGGER.warn(
                                  "Router lab HTTP fixture rejected a request", e);
                          }
                      },
                      "Router-lab-HTTP");
              worker.setDaemon(true);
              worker.start();
              String address = null;
              for (var network : Collections.list(java.net.NetworkInterface.getNetworkInterfaces()))
                if (network.isUp() && !network.isLoopback())
                  for (var ip : Collections.list(network.getInetAddresses()))
                    if (ip instanceof java.net.Inet4Address
                        && !ip.isLinkLocalAddress()
                        && ip.isSiteLocalAddress()) {
                      address = ip.getHostAddress();
                      break;
                    }
              if (address == null) throw new IllegalStateException("No host IPv4 interface");
              FIXTURE_URL.put(r, "http://" + address + ":" + listener.getLocalPort() + "/");
            } catch (java.io.IOException e) {
              throw new IllegalStateException(e);
            }
          }

          public void clear(ScenarioRun r) {
            var listener = FIXTURES.remove(r);
            FIXTURE_URL.remove(r);
            if (listener == null) return;
            try {
              listener.close();
            } catch (Exception ignored) {
            }
          }
        });
    b.note("Built for you: an Internet Gateway block cabled to the client's bottom face (eth0); a web server"
        + " runs on this machine (outside the game).");
    provision(b, "client", Map.of("network.cfg",
        "iface eth0 10.0.0.50/24\nroute default via 10.0.0.1 dev eth0\n"));
    b.send("client", "ifconfig");
    b.expect("client", "10.0.0.50", "static gateway-LAN address (the gateway serves no DHCP)");
    b.expect("client", "/ >", "ifconfig returned to shell");
    b.await(r -> {
          var m = CableNetworkManager.getInstance();
          byte[] mac = NetworkHub.deriveMac(r.terminal("client").getComputerId(), 0);
          return m != null && m.hasInternetAccess(mac) ? null : "the client's cable doesn't reach an Internet Gateway";
        }, "the client's cable reaches the Internet Gateway (its segment is online)", 5_000);
    b.sendFn("client", r -> "curl " + FIXTURE_URL.get(r),
        "curl http://<this machine's LAN address>:<the web server's port>/");
    b.expect("client", "host-socket-ok", "the real host socket proxy fetched the page (external ICMP and HTTPS are unsupported)");
    return b.build();
  }

  /**
   * The chat command against a Discord backend: discordd on "server", with alice cabled to its
   * eth0 (bottom) and bob to its eth1 (top, a second LAN); alice and bob read the server from
   * /etc/chat.conf and talk in the backend's home server #general. Control first: a port with no
   * backend gets the "refused" fix-it hint.
   */
  private static Scenario chat() {
    var b =
        nodes(
            "router_chat",
            "Instant messaging: discordd on 'server', 'alice' and 'bob' run chat (server from"
                + " /etc/chat.conf), talk in #general, /who and /quit. Control: a port without a"
                + " backend is refused with a fix-it hint.",
            "server",
            "alice",
            "bob");
    downDown(b, "lan-alice", "server", "alice", 0);
    // server eth1 (top) over alice to bob's bottom (eth0).
    b.link("lan-bob", "server", UP, "bob", DOWN,
        Scenario.Path.from(new BlockPos(0, 2, 0)).go(UP, 1).go(EAST, 6).go(DOWN, 3).go(EAST, 2));
    provisionAll(
        b,
        ordered(
            Map.entry("server", Map.of("network.cfg", "iface eth0 10.95.0.1/24\niface eth1 10.95.1.1/24\n",
                "services.cfg", "discordd 7777 &\n")),
            Map.entry("alice", Map.of("network.cfg", "iface eth0 10.95.0.2/24\n", "etc/chat.conf",
                "server 10.95.0.1\nport 7777\nnick alice\n")),
            Map.entry("bob", Map.of("network.cfg", "iface eth0 10.95.1.3/24\n", "etc/chat.conf",
                "server 10.95.1.1\nnick bob\n"))));
    b.note("server runs discordd 7777 & (a Discord backend) on both of its LANs; cat /etc/chat.conf on alice and bob.");
    b.send("alice", "chat 10.95.0.1:7778");
    b.expect("alice", "refused", "control: no Discord backend on TCP port 7778 (fix-it hint; 7778 is its voice port, UDP)");
    b.expect("alice", "/ >", "shell");
    b.send("alice", "chat");
    b.expect("alice", "\\*\\*\\* Joined .* as alice", "alice joined (server and nick from /etc/chat.conf)");
    b.send("bob", "chat");
    b.expect("bob", "\\*\\*\\* Joined .* as bob", "bob joined");
    b.send("alice", "hello bob");
    b.expect("bob", "<alice> hello bob", "bob sees alice's message with a timestamp");
    b.send("bob", "/who");
    b.expect("bob", "2 online: alice, bob", "/who lists both");
    b.send("bob", "/quit");
    b.expect("alice", "\\* bob left \\(quit\\)", "alice sees bob leave");
    b.send("alice", "/quit");
    b.expect("alice", "\\*\\*\\* bye", "alice quit");
    return b.build();
  }

  private static Scenario fiber() {
    var b =
        nodes(
            "router_fiber",
            "Physical six-way fiber joins, removal/repair, serialized states, a fiber riser onto"
                + " the patch panel, and Always-On Module model inspection.",
            "pc");
    BlockPos center = new BlockPos(5, 4, 5);
    var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
    b.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            return List.of(new BlockPos(0, 1, 3), new BlockPos(10, 5, 7));
          }

          public void build(ScenarioRun r) {
            var p = r.player();
            // The cross: the centre, then each arm clicked onto it.
            p.place(span, r.abs(center), SOUTH);
            for (var d : net.minecraft.core.Direction.values())
              p.placeAgainst(span, r.abs(center.relative(d)), d == SOUTH ? EAST : SOUTH, d.getOpposite(), null);
            // A free-floating riser on a patch panel: spans need no support and join the panel below.
            p.place(com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get(), r.abs(new BlockPos(9, 2, 5)), SOUTH);
            for (int y = 3; y <= 5; y++) p.placeAgainst(span, r.abs(new BlockPos(9, y, 5)), SOUTH, DOWN, null);
            p.installModule(r.where("pc"), com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE.get());
          }
        });
    b.note(
        "Inspect the labelled PC's installed Always-On Module, the rack patch panel with a"
            + " floating fiber riser on it, and the six-way fiber cross. Break the east arm, then"
            + " replace it with Fiber Span: only that arm must change.");
    b.mutate(
        r -> {
          var l = r.level();
          var s = l.getBlockState(r.abs(center));
          for (var d : net.minecraft.core.Direction.values())
            if (!s.getValue(
                com.example.evanscomputermod.block.NetworkCableBlock.getPropertyForDirection(d)))
              throw new IllegalStateException("Missing " + d + " join");
          var riser = l.getBlockState(r.abs(new BlockPos(9, 3, 5)));
          if (!riser.getValue(com.example.evanscomputermod.block.NetworkCableBlock.DOWN)
              || !riser.getValue(com.example.evanscomputermod.block.NetworkCableBlock.UP)
              || l.getBlockState(r.abs(new BlockPos(9, 5, 5)))
                  .getValue(com.example.evanscomputermod.block.NetworkCableBlock.UP))
            throw new IllegalStateException("Riser does not join the panel / top arm is open");
        },
        "All six center arms are connected; the floating riser joins the patch panel below it"
            + " and its top end has no open arm.");
    b.mutate(
        r -> {
          var p = r.abs(center.east());
          r.player().breakBlock(p, SOUTH);
          if (r.level()
              .getBlockState(p.west())
              .getValue(com.example.evanscomputermod.block.NetworkCableBlock.EAST))
            throw new IllegalStateException("Disconnected east arm remained");
        },
        "Break the east neighbor of the six-way center; its east arm must disappear.");
    b.mutate(
        r -> {
          var p = r.abs(center.east());
          r.player().placeAgainst(span, p, SOUTH, WEST, null);
          if (!r.level().getBlockState(p.west())
              .getValue(com.example.evanscomputermod.block.NetworkCableBlock.EAST))
            throw new IllegalStateException("Repaired east arm missing");
        },
        "Replace the east neighbor; the arm reconnects.");
    b.mutate(
        r -> {
          var p = r.abs(center);
          var state = r.level().getBlockState(p);
          var saved = net.minecraft.nbt.NbtUtils.writeBlockState(state);
          var loaded =
              net.minecraft.nbt.NbtUtils.readBlockState(
                  r.level().holderLookup(net.minecraft.core.registries.Registries.BLOCK), saved);
          if (!state.equals(loaded))
            throw new IllegalStateException("Fiber connection state failed save/reload");
        },
        "Save and reload the world: connection properties persist in block-state NBT.");
    return b.build();
  }

  /**
   * A Tech Village network in miniature, with village 5's configuration typed in and real
   * cables: ISP router DOWN (eth0, village access + DHCP) to a home router's DOWN (eth0, WAN
   * DHCP + NAT) through a buried run; the home router's UP (eth1, LAN) to the PC's UP over a
   * patch cable; ISP UP (eth1) to the server's UP.
   */
  private static Scenario village() {
    var b =
        Scenario.builder(
                "router_village",
                "Tech Village wiring: buried village cable from the ISP's DOWN face to a home"
                    + " router's WAN, patch cable over the router and PC (LAN), server on the"
                    + " ISP's UP face. Village 5's configuration, typed in. Control: cut the village cable.")
            .timeLimit(55_000)
            .asPlayer();
    b.host("isp", new BlockPos(0, 1, 0), null);
    b.host("server", new BlockPos(2, 1, 0), null);
    b.host("home", new BlockPos(6, 1, 0), null);
    b.host("pc", new BlockPos(7, 1, 0), null);
    b.link("server-lan", "isp", UP, "server", UP,
        Scenario.Path.from(new BlockPos(0, 2, 0)).go(EAST, 2));
    b.link("village-cable", "isp", DOWN, "home", DOWN,
        Scenario.Path.from(new BlockPos(0, 0, 0)).go(DOWN, 1).go(EAST, 6).go(UP, 1));
    b.link("home-lan", "home", UP, "pc", UP,
        Scenario.Path.from(new BlockPos(6, 2, 0)).go(EAST, 1));
    // The village's own files (chat.conf aside: this lab doesn't chat), set up by hand.
    LinkedHashMap<String, Map<String, String>> files = new LinkedHashMap<>();
    for (var e : List.of(Map.entry("isp", WorldNetwork.ISP_ROUTER), Map.entry("server", WorldNetwork.WEB),
        Map.entry("home", "house1.router"), Map.entry("pc", "house1.pc"))) {
      Map<String, String> f = new HashMap<>(WorldNetwork.configs(5, e.getValue(), 5, List.of()));
      f.remove("etc/chat.conf");
      files.put(e.getKey(), f);
    }
    provisionAll(b, files);
    b.note("DOWN is eth0 and UP is eth1 on every terminal; cat router.cfg on isp and home.");
    b.until("pc", "ifconfig eth1", "inet 192\\.168\\.1\\.\\d+", "the PC leased a home LAN address");
    b.expect("pc", "/ >", "shell");
    b.until("home", "ifconfig eth0", "inet 100\\.69\\.1\\.\\d+", "the home router's WAN leased from the ISP over the village cable");
    b.expect("home", "/ >", "shell");
    ping(b, "pc", "100.69.0.10", 2, 2, "PC reaches the village server through home NAT and the ISP");
    b.send("pc", "curl http://100.69.0.10/index.html");
    b.expect("pc", "Tech Village 5", "the village server's page");
    b.expect("pc", "/ >", "shell");
    b.cut("village-cable", 3, "Dig up the village cable under the street: the house loses the ISP.");
    b.waitMs(1500, "carrier loss");
    ping(b, "pc", "100.69.0.10", 1, 0, "control: no path without the village cable");
    return b.build();
  }

  /**
   * A player's own fiber between two buildings, built by hand: a Fiber Patch Panel on each
   * PC's UP face (eth1) and a run of Fiber Span between the panels. Fiber joins only fiber
   * and panels, so a copper cable touching the run (the "tap" PC) is not connected.
   */
  private static Scenario playerFiber() {
    var b =
        Scenario.builder(
                "router_player_fiber",
                "Your own fiber: panels on two PCs' top faces (eth1) joined by hand-placed Fiber"
                    + " Span. Ping across; break a span (no reply); replace it. Control: copper"
                    + " cable touching the fiber is not connected.")
            .timeLimit(55_000)
            .asPlayer();
    b.host("west", new BlockPos(0, 1, 0), "10.94.0.1/24");
    b.host("east", new BlockPos(8, 1, 0), "10.94.0.2/24");
    b.host("tap", new BlockPos(4, 1, 1), "10.94.0.3/24");
    var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
    var panel = com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get();
    var cable = com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get();
    BlockPos breakAt = new BlockPos(4, 3, 0);
    b.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            List<BlockPos> f = new ArrayList<>();
            f.add(new BlockPos(0, 2, 0));
            f.add(new BlockPos(8, 2, 0));
            for (int x = 0; x <= 8; x++) f.add(new BlockPos(x, 3, 0));
            f.add(new BlockPos(4, 2, 1));
            f.add(new BlockPos(4, 3, 1));
            return f;
          }

          public void build(ScenarioRun r) {
            var hands = r.player();
            hands.place(panel, r.abs(new BlockPos(0, 2, 0)), SOUTH);
            hands.place(panel, r.abs(new BlockPos(8, 2, 0)), SOUTH);
            for (int x = 0; x <= 8; x++) hands.place(span, r.abs(new BlockPos(x, 3, 0)), SOUTH);
            // The tap: copper from the third PC's UP face, touching the fiber's side.
            hands.place(cable, r.abs(new BlockPos(4, 2, 1)), SOUTH);
            hands.place(cable, r.abs(new BlockPos(4, 3, 1)), SOUTH);
          }
        });
    b.note("Panels sit on each PC's UP face (eth1); the fiber runs between their tops.");
    b.send("west", "ifconfig eth1 10.94.0.1/24");
    b.expect("west", "eth1: inet 10\\.94\\.0\\.1/24", "west has 10.94.0.1");
    b.send("east", "ifconfig eth1 10.94.0.2/24");
    b.expect("east", "eth1: inet 10\\.94\\.0\\.2/24", "east has 10.94.0.2");
    b.send("tap", "ifconfig eth1 10.94.0.3/24");
    b.expect("tap", "eth1: inet 10\\.94\\.0\\.3/24", "tap has 10.94.0.3");
    // A NIC has carrier only with a partner on the wire; kernels sample carrier every 250 ms.
    b.waitMs(500, "link up on both ends of the fiber");
    ping(b, "west", "10.94.0.2", 2, 2, "west reaches east over the player's fiber");
    ping(b, "tap", "10.94.0.2", 1, 0, "control: copper touching the fiber is not connected");
    b.mutate(
        r -> r.player().breakBlock(r.abs(breakAt), SOUTH),
        "Break the middle Fiber Span: the run is cut.");
    b.waitMs(500, "carrier loss");
    ping(b, "west", "10.94.0.2", 1, 0, "no path with a span missing");
    b.mutate(
        r -> r.player().place(span, r.abs(breakAt), SOUTH),
        "Place a Fiber Span back in the gap: the run carries traffic again.");
    b.waitMs(500, "link up again");
    ping(b, "west", "10.94.0.2", 2, 2, "west reaches east again");
    return b.build();
  }

  /**
   * Ports follow the computer's facing: eth0 bottom, eth1 top, eth2 back, eth3 left, eth4
   * right, whichever way the screen faces. The player places an east-facing PC (a facing
   * whose ports moved when numbering was by world direction: its back was eth4), cables its
   * back to a server's bottom and pings over eth2, then turns the PC to face each other way
   * and reads every face with the Interface Probe.
   */
  private static Scenario portsFollowFacing() {
    var b =
        Scenario.builder(
                "router_ports_follow_facing",
                "Ports are relative to the screen: bottom eth0, top eth1, back eth2, left eth3,"
                    + " right eth4. An east-facing PC's back (west face) is cabled to a server and"
                    + " pings on eth2; turned to face south, west and north, the Interface Probe"
                    + " reads the same names on the same relative faces. Control: the screen face"
                    + " has no port.")
            .timeLimit(55_000)
            .asPlayer();
    b.host("server", new BlockPos(0, 1, 0), "10.95.0.1/24");
    b.host("pc", new BlockPos(4, 1, 0), "10.95.0.2/24", EAST);
    // server DOWN (eth0) -> under the floor -> up into the PC's back (its west face).
    b.link("cable", "server", DOWN, "pc", WEST,
        Scenario.Path.from(new BlockPos(0, 0, 0)).go(EAST, 3).go(UP, 1));
    b.note("The PC faces east, so its back is the west face, where the cable arrives: that is eth2.");
    b.mutate(r -> probeFaces(r, "pc", EAST), "Probe the east-facing PC's faces: bottom eth0, top eth1, back eth2, left eth3, right eth4");
    b.send("server", "ifconfig eth0 10.95.0.1/24");
    b.expect("server", "eth0: inet 10\\.95\\.0\\.1/24", "server has 10.95.0.1 on its bottom face");
    b.send("pc", "ifconfig eth2 10.95.0.2/24");
    b.expect("pc", "eth2: inet 10\\.95\\.0\\.2/24", "PC has 10.95.0.2 on its back face");
    b.waitMs(500, "link up on both ends");
    ping(b, "pc", "10.95.0.1", 2, 2, "the PC reaches the server through its back port (eth2)");
    for (var facing : List.of(SOUTH, WEST, NORTH)) {
      String way = facing.getName();
      b.mutate(r -> r.player().breakForDrop(r.where("pc")), "Break the PC (it drops as an item)");
      b.await(r -> r.player().pickUp(r.where("pc"), PlayerKit.ModBlocksRef.terminalItem()), "pick the PC up", 5_000);
      b.mutate(r -> {
        BlockPos at = r.where("pc");
        r.player().placeFromInventory(PlayerKit.ModBlocksRef.terminalItem(), at, facing,
            s -> s.getValue(com.example.evanscomputermod.block.TerminalBlock.FACING) == facing);
        r.player().openScreen(at, facing);
      }, "Place the PC back facing " + way + " and open its screen");
      b.await(r -> r.terminal("pc") != null && r.terminal("pc").findInterfaceIndexByExitPos(r.where("pc").below()) == 0 ? null : "booting",
          "the PC's ports are up", 10_000);
      b.mutate(r -> probeFaces(r, "pc", facing), "Probe the " + way + "-facing PC: the same names on the same relative faces");
    }
    return b.build();
  }

  /** Sneak-right-click every face of {@code node} with the Interface Probe (unsneaking opens the screen) and check the names relative to {@code facing}. */
  private static void probeFaces(ScenarioRun r, String node, net.minecraft.core.Direction facing) {
    var probe = com.example.evanscomputermod.item.ModItems.INTERFACE_PROBE.get();
    BlockPos at = r.where(node);
    var expected = ordered(
        Map.entry(DOWN, "eth0"), Map.entry(UP, "eth1"), Map.entry(facing.getOpposite(), "eth2"),
        Map.entry(facing.getCounterClockWise(), "eth3"), Map.entry(facing.getClockWise(), "eth4"));
    String[] names = {"bottom", "top", "back", "left", "right"};
    int i = 0;
    for (var e : expected.entrySet()) {
      String heard = String.join(" ", r.player().use(probe, at, e.getKey(), true));
      String side = names[i++] + " (" + e.getKey().getName() + ")";
      if (!heard.contains(e.getValue() + " ")) {
        r.fail(node + " facing " + facing.getName() + ": the " + side + " face should be " + e.getValue() + ", probe said: " + heard);
        return;
      }
      r.say("§7  " + side + ": " + e.getValue());
    }
    String screen = String.join(" ", r.player().use(probe, at, facing, true));
    if (!screen.contains("no interface")) r.fail("control: the screen face should have no port, probe said: " + screen);
    else r.say("§7  screen (" + facing.getName() + "): no port");
  }

  /**
   * The router configuration a player types on a tap router: eth0 (DOWN, on the patch
   * panel) takes a free address in the link's /28, eth1 (UP) is the player's LAN, and the
   * router peers with both ends of the fiber. {@code extra} adds lines in global config
   * (before {@code router bgp}) and {@code family} in the address family.
   */
  static List<String> tapConfig(long asn, int lan, String address, String peer1, long as1, String peer2, long as2,
      List<String> extra, List<String> family) {
    List<String> l = new ArrayList<>(List.of(
        "configure terminal",
        "ip routing",
        "interface eth0",
        "ip address " + address,
        "exit",
        "interface eth1",
        "ip address 10.200." + lan + ".1/24",
        "exit"));
    l.addAll(extra);
    l.addAll(List.of(
        "router bgp " + asn,
        "bgp router-id 10.200." + lan + ".1",
        "timers bgp 3 9",
        "neighbor " + peer1 + " remote-as " + as1,
        "neighbor " + peer2 + " remote-as " + as2,
        "address-family ipv4 unicast",
        "neighbor " + peer1 + " activate",
        "neighbor " + peer2 + " activate",
        "network 10.200." + lan + ".0/24"));
    l.addAll(family);
    l.addAll(List.of("end", "write memory"));
    return l;
  }

  private static String q(String s) {
    return java.util.regex.Pattern.quote(s);
  }

  /** The tap PC's startup file: its UP face (eth1) on the tap router's LAN. */
  private static Map<String, String> tapPc(int lan) {
    return Map.of("network.cfg", "iface eth1 10.200." + lan + ".10/24\nroute default via 10.200." + lan + ".1 dev eth1\n");
  }

  /** Lab ISP: eth0 (DOWN, on the fiber's patch panel) and eth1 (UP, its server LAN). */
  private static Map<String, String> labIsp(int n) {
    int peer = 3 - n;
    String cfg =
        address(0, "172.30.50." + n + "/28")
            + address(1, "100." + (80 + n) + ".0.1/24")
            + "ip prefix-list TAP-IN seq 10 deny 100.64.0.0/10 le 32\n"
            + "ip prefix-list TAP-IN seq 20 deny 172.16.0.0/12 le 32\n"
            + "ip prefix-list TAP-IN seq 30 deny 0.0.0.0/0\n"
            + "ip prefix-list TAP-IN seq 40 permit 0.0.0.0/0 le 24\n"
            + "route-map TAP-IN permit 10\nmatch ip address prefix-list TAP-IN\nexit\n"
            + "router bgp " + (65100 + n) + "\nbgp router-id 100." + (80 + n) + ".0.1\ntimers bgp 3 9\n"
            + "neighbor 172.30.50." + peer + " remote-as " + (65100 + peer) + "\n"
            + "neighbor TAPS peer-group\nneighbor TAPS remote-as external\n"
            + "neighbor TAPS listen ip-range 172.30.50.0/28 limit 8\n"
            + "address-family ipv4 unicast\n"
            + "neighbor 172.30.50." + peer + " activate\n"
            + "neighbor TAPS activate\nneighbor TAPS route-map TAP-IN in\nneighbor TAPS maximum-prefix 20\n"
            + "network 100." + (80 + n) + ".0.0/24\n";
    return router(cfg);
  }

  private static Map<String, String> labServer(int n) {
    return Map.of(
        "network.cfg",
        "iface eth1 100." + (80 + n) + ".0.10/24\nroute default via 100." + (80 + n) + ".0.1 dev eth1\n");
  }

  /**
   * Tapping a fiber: two ISPs (AS 65101 and 65102, each with a server on its UP face)
   * joined by a 13-block run of Fiber Span between the patch panels under their DOWN
   * faces. Both run open peering on the link's /28, like every Tech Village ISP. The
   * player puts a Fiber Patch Panel on the middle span under their own router, finds the
   * subnet with tcpdump, takes 172.30.50.5 and peers with both ISPs; a hijack of ISP A's
   * prefix and a default route are filtered. Control: breaking the span between the tap
   * and ISP A drops that session but ISP B's side keeps working; replacing it repairs.
   */
  private static Scenario fiberTapLab() {
    var b =
        Scenario.builder(
                "router_fiber_tap",
                "Tap a fiber: a patch panel on a span between two open-peering ISPs, your router"
                    + " (AS 65200) takes a free address in the link's /28 and peers with both;"
                    + " hijacks are filtered. Control: break the span toward ISP A, B still works.")
            .timeLimit(55_000)
            .asPlayer();
    var up = net.minecraft.core.Direction.UP;
    b.host("ispA", new BlockPos(0, 3, 0), null);
    b.host("srvA", new BlockPos(1, 3, 0), null);
    b.host("tap", new BlockPos(6, 3, 0), null);
    b.host("pc", new BlockPos(7, 3, 0), null);
    b.host("srvB", new BlockPos(11, 3, 0), null);
    b.host("ispB", new BlockPos(12, 3, 0), null);
    b.link("lanA", "ispA", up, "srvA", up, Scenario.Path.from(new BlockPos(0, 4, 0)).go(net.minecraft.core.Direction.EAST, 1));
    b.link("lanTap", "tap", up, "pc", up, Scenario.Path.from(new BlockPos(6, 4, 0)).go(net.minecraft.core.Direction.EAST, 1));
    b.link("lanB", "ispB", up, "srvB", up, Scenario.Path.from(new BlockPos(12, 4, 0)).go(net.minecraft.core.Direction.WEST, 1));
    var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
    var panel = com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get();
    var south = net.minecraft.core.Direction.SOUTH;
    BlockPos tapPanel = new BlockPos(6, 2, 0), breakAt = new BlockPos(3, 1, 0);
    b.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            List<BlockPos> f = new ArrayList<>();
            for (int x = 0; x <= 12; x++) f.add(new BlockPos(x, 1, 0));
            f.add(new BlockPos(0, 2, 0));
            f.add(new BlockPos(12, 2, 0));
            f.add(tapPanel);
            return f;
          }

          public void build(ScenarioRun r) {
            var hands = r.player();
            for (int x = 0; x <= 12; x++) hands.place(span, r.abs(new BlockPos(x, 1, 0)), south);
            hands.place(panel, r.abs(new BlockPos(0, 2, 0)), south);
            hands.place(panel, r.abs(new BlockPos(12, 2, 0)), south);
          }
        });
    b.note("ISP A and ISP B sit on Fiber Patch Panels (their DOWN face, eth0) at the ends of a"
        + " fiber run; each has a server on its UP face. Set the ISPs up with open peering (group"
        + " TAPS), the servers and your PC.");
    provisionAll(
        b,
        ordered(
            Map.entry("ispA", labIsp(1)),
            Map.entry("ispB", labIsp(2)),
            Map.entry("srvA", labServer(1)),
            Map.entry("srvB", labServer(2)),
            Map.entry("pc", tapPc(0))));
    b.send("ispA", "router");
    b.until("ispA", "show bgp ipv4 unicast summary", "^ 172\\.30\\.50\\.2 +65102 .*Established", "the ISPs peer over the fiber");
    b.send("ispA", "exit");
    b.expect("ispA", "/ >", "shell");
    b.note("Tap the fiber");
    b.mutate(
        r -> r.player().place(panel, r.abs(tapPanel), south),
        "Place a Fiber Patch Panel on top of the middle span, right under your router's DOWN face (eth0).");
    b.send("tap", "tcpdump -i eth0 -c 2");
    b.expect("tap", "172\\.30\\.50\\.[12]", "the ISPs' BGP keepalives give away the link's addresses (.1 and .2 of a /28)");
    b.expect("tap", "/ >", "tcpdump done");
    b.note("Take a free address (.5) in 172.30.50.0/28 and peer with both ISPs");
    b.send("tap", "router on");
    b.expect("tap", "Router started", "router service");
    b.send("tap", "router");
    b.expect("tap", "router#", "router CLI");
    for (String line :
        tapConfig(65200, 0, "172.30.50.5/28", "172.30.50.1", 65101, "172.30.50.2", 65102,
            List.of("ip route 100.81.0.128/25 10.200.0.99"),
            List.of("network 100.81.0.128/25", "neighbor 172.30.50.1 default-originate",
                "neighbor 172.30.50.2 default-originate")))
      b.send("tap", line);
    b.expect("tap", "Configuration saved", "write memory");
    b.until("tap", "show bgp ipv4 unicast summary", "^ 172\\.30\\.50\\.1 +65101 .*Established", "peered with ISP A");
    b.until("tap", "show bgp ipv4 unicast summary", "^ 172\\.30\\.50\\.2 +65102 .*Established", "peered with ISP B");
    b.send("tap", "show bgp ipv4 unicast");
    b.expect("tap", "^100\\.82\\.0\\.0/24 via 172\\.30\\.50\\.2 AS_PATH \\[65102\\]", "learned ISP B's prefix");
    b.send("tap", "exit");
    b.expect("tap", "/ >", "shell");
    ping(b, "pc", "100.81.0.10", 2, 2, "your LAN reaches ISP A's server");
    ping(b, "pc", "100.82.0.10", 2, 2, "and ISP B's");
    b.note("On ISP B: the tap is a dynamic neighbor; its own /24 is in, its hijack and default are out");
    b.send("ispB", "router");
    b.send("ispB", "show bgp ipv4 unicast summary");
    b.expect("ispB", "^\\*172\\.30\\.50\\.5 +65200 .*Established +Up +1$", "dynamic neighbor, 1 prefix accepted");
    b.send("ispB", "show bgp ipv4 unicast");
    b.expect("ispB", "^100\\.81\\.0\\.0/24 via 172\\.30\\.50\\.1 ", "ISP A's prefix comes from ISP A");
    b.expect("ispB", "^10\\.200\\.0\\.0/24 via 172\\.30\\.50\\.5 AS_PATH \\[65200\\]", "the tap's LAN is accepted");
    b.mutate(
        r -> {
          String out = r.latestOutput("ispB");
          if (out == null || java.util.regex.Pattern.compile("(?m)^(0\\.0\\.0\\.0/0|100\\.81\\.0\\.128/25) ").matcher(out).find())
            throw new IllegalStateException("the tap's hijack or default route was accepted:\n" + out);
        },
        "No 100.81.0.128/25 (the tap's hijack of a piece of ISP A's network) and no 0.0.0.0/0 in ISP B's"
            + " table: the TAP-IN filter refused them.");
    b.send("ispB", "exit");
    b.expect("ispB", "/ >", "shell");
    b.note("Control: cut the fiber between the tap and ISP A");
    b.mutate(r -> r.player().breakBlock(r.abs(breakAt), south), "Break the span three blocks toward ISP A.");
    ping(b, "pc", "100.82.0.10", 2, 2, "ISP B's side still works");
    b.send("tap", "router");
    b.until("tap", "show bgp ipv4 unicast summary", "^ 172\\.30\\.50\\.1 +65101 .*(Active|Connect|Idle)",
        "the session to ISP A times out (hold 9 s)");
    b.send("tap", "show bgp ipv4 unicast summary");
    b.expect("tap", "^ 172\\.30\\.50\\.2 +65102 .*Established", "ISP B's session is untouched");
    b.mutate(r -> r.player().place(span, r.abs(breakAt), south), "Put a Fiber Span back in the gap.");
    b.until("tap", "show bgp ipv4 unicast summary", "^ 172\\.30\\.50\\.1 +65101 .*Established", "re-peered with ISP A");
    b.send("tap", "exit");
    b.expect("tap", "/ >", "shell");
    return b.build();
  }

  /**
   * A path index of chord {@code chord} where the {@link #ringTap} layout fits: its box
   * (above the path) holds no ring block and passes {@code free}, the control panel
   * touches no path block, and the tap and the break three blocks back satisfy
   * {@code where(x, z)}. Searches outward from the chord's middle; -1 if none.
   */
  public static int tapIndex(FiberChords ring, int chord, java.util.function.BiPredicate<Integer, Integer> where,
      java.util.function.LongPredicate free) {
    int[][] path = ring.path(chord);
    int mid = path.length / 2;
    for (int off = 0; off < path.length / 2; off++)
      for (int i : new int[] {mid + off, mid - off}) {
        if (i < 4 || i + 4 >= path.length) continue;
        int[] p = path[i], q = path[i - 3];
        if (!where.test(p[0], p[2]) || !where.test(q[0], q[2])) continue;
        boolean clear = true;
        for (int x = -2; x <= 4 && clear; x++)
          for (int y = 1; y <= 6 && clear; y++)
            for (int z = -3; z <= 2 && clear; z++) {
              long at = FiberChords.pack(p[0] + x, p[1] + y, p[2] + z);
              if (ring.chordsAt(at) != 0 || !free.test(at)) clear = false;
            }
        long ctl = FiberChords.pack(p[0] + 2, p[1] + 1, p[2]);
        if (clear && !RingPieces.touchesPath(ring, ctl)) return i;
      }
    return -1;
  }

  /** What a Tech Village ISP router shows for {@code command}, polled from its (headless) screen. */
  private static java.util.function.Function<ScenarioRun, String> ispShows(
      int village, String command, String must, String mustNot) {
    long[] sentAt = {0};
    String[] before = {null};
    var re = java.util.regex.Pattern.compile(must, java.util.regex.Pattern.MULTILINE);
    var bad = mustNot == null ? null : java.util.regex.Pattern.compile(mustNot, java.util.regex.Pattern.MULTILINE);
    return r -> {
      var level = r.level();
      var d = WorldNetwork.get(level);
      var host = ComputerHost.get(level.getServer(), d.identity(level, village, WorldNetwork.ISP_ROUTER));
      if (host.instance() == null) return "village " + village + " ISP router not running";
      String screen =
          host.attachment() instanceof com.example.evanscomputermod.block.TerminalBlockEntity be
              ? ScenarioRun.screen(be.getDisplay())
              : ScenarioRun.screen(host.headlessDisplay());
      long now = System.currentTimeMillis();
      if (before[0] == null || now - sentAt[0] > 3000) {
        before[0] = screen;
        host.instance().sendInput("router\n" + command + "\nexit\n");
        sentAt[0] = now;
        return "asked";
      }
      // Judge only a fresh, finished answer: the screen changed and is back at the shell.
      if (screen.equals(before[0]) || !screen.stripTrailing().endsWith("/ >")) return "waiting for the answer";
      String out = ScenarioRun.after(screen, command);
      if (!re.matcher(out).find()) return "village " + village + " '" + command + "' lacks /" + must + "/:\n" + out;
      if (bad != null && bad.matcher(out).find())
        return "village " + village + " '" + command + "' shows /" + mustNot + "/:\n" + out;
      return null;
    };
  }

  /**
   * The ring-tap lab up to BGP: the tap router on a Fiber Patch Panel on a span of the
   * generated ring between village {@code a} and the next (eth0, a free address .5 in the
   * link's /28, AS 65200+a), its LAN 10.200.a.0/24 on its UP face with a PC (.10), an
   * off-path control panel under "ctl", both sessions Established. {@code listen}: first
   * listen to the link with tcpdump; {@code extra}/{@code family}: more router lines (see
   * {@link #tapConfig}). Other scenarios (DNS) continue from here.
   */
  public static Scenario.Builder tapLab(int a, String name, String description, long timeLimit, boolean listen,
      List<String> extra, List<String> family) {
    int b = a == 10 ? 1 : a + 1;
    String sub = "172.31." + a + ".";
    var s = Scenario.builder(name, description).timeLimit(timeLimit).asPlayer();
    var up = net.minecraft.core.Direction.UP;
    var north = net.minecraft.core.Direction.NORTH;
    s.host("tap", new BlockPos(0, 2, 0), null);
    s.host("pc", new BlockPos(1, 2, 0), null);
    s.host("ctl", new BlockPos(2, 1, 0), null);
    s.link("lan", "tap", up, "pc", up, Scenario.Path.from(new BlockPos(0, 3, 0)).go(net.minecraft.core.Direction.EAST, 1));
    var panel = com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get();
    var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
    var cable = com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get();
    s.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            return List.of(new BlockPos(0, 0, 0), new BlockPos(0, 1, 0), new BlockPos(2, 0, 0));
          }

          public void build(ScenarioRun r) {
            r.player().place(cable, r.abs(new BlockPos(0, 1, 0)), north);
            // Control: a panel under "ctl", not touching the ring.
            r.player().place(panel, r.abs(new BlockPos(2, 0, 0)), north);
          }
        });
    java.util.function.Function<ScenarioRun, byte[]> tapNic =
        r -> NetworkHub.deriveMac(r.terminal("tap").getComputerId(), 0);
    java.util.function.Function<ScenarioRun, byte[]> ctlNic =
        r -> NetworkHub.deriveMac(r.terminal("ctl").getComputerId(), 0);
    java.util.function.BiFunction<ScenarioRun, Boolean, byte[]> ispNic =
        (r, next) -> NetworkHub.deriveMac(
            WorldNetwork.get(r.level()).identity(r.level(), next ? a : b, WorldNetwork.ISP_ROUTER),
            next ? WorldNetwork.FIBER_NEXT_PORT : WorldNetwork.FIBER_PREV_PORT);
    s.note("Your PC on the router's LAN: its address and default route (typed)");
    provision(s, "pc", tapPc(a));
    s.note("Tap the fiber");
    s.mutate(
        r -> r.player().place(panel, r.abs(new BlockPos(0, 0, 0)), north),
        "Place a Fiber Patch Panel on top of a span of the ring (under the cable to your router's DOWN face).");
    s.await(
        r -> {
          var m = CableNetworkManager.getInstance();
          if (!m.areOnSameNetwork(tapNic.apply(r), ispNic.apply(r, true))
              || !m.areOnSameNetwork(tapNic.apply(r), ispNic.apply(r, false)))
            return "the tap is not on the " + a + "-" + b + " fiber segment";
          if (m.areOnSameNetwork(ctlNic.apply(r), ispNic.apply(r, true)) || m.carrierOf(ctlNic.apply(r)))
            return "control: a panel two blocks from the path joined the fiber";
          return null;
        },
        "the tap shares the fiber with both ISPs; the off-path control panel does not",
        5_000);
    if (listen) {
      // The ring carries the carrier's cell traffic too: listen to BGP (TCP 179) only.
      s.send("tap", "tcpdump -i eth0 -c 1 tcp port 179");
      s.expect("tap", q(sub) + "[12]\\.\\d+ > " + q(sub) + "[12]\\.", "the ISPs' keepalives give away the link (" + sub + "1 and .2)");
      s.expect("tap", "/ >", "tcpdump done");
    }
    s.send("tap", "router on");
    s.expect("tap", "Router started", "router service");
    s.send("tap", "router");
    s.expect("tap", "router#", "router CLI");
    for (String line :
        tapConfig(65200 + a, a, sub + "5/28", sub + "1", 65000 + a, sub + "2", 65000 + b, extra, family))
      s.send("tap", line);
    s.expect("tap", "Configuration saved", "write memory");
    s.until("tap", "show bgp ipv4 unicast summary", "^ " + q(sub) + "1 +" + (65000 + a) + " .*Established", "peered with village " + a);
    s.until("tap", "show bgp ipv4 unicast summary", "^ " + q(sub) + "2 +" + (65000 + b) + " .*Established", "peered with village " + b);
    return s;
  }

  /**
   * A player taps the real generated ring between village {@code a} and the next one. The
   * layout's origin is the air block on top of a path span: the player puts a Fiber Patch
   * Panel there, a network cable on it and their router on the cable (DOWN face, eth0); a
   * PC shares the router's UP face (eth1, LAN 10.200.a.0/24). A control panel is placed
   * two blocks from the path under the "ctl" computer and must not join the fiber.
   *
   * @param cut also break the span at {@code breakAt} (toward village a) and repair it, and
   *     keep the tap router Always-On through an unload
   */
  public static Scenario ringTap(int a, boolean cut, BlockPos breakAt) {
    int b = a == 10 ? 1 : a + 1, far = (a + 3) % 10 + 1;
    String sub = "172.31." + a + ".";
    var s = tapLab(a, "router_ring_tap_" + a + (cut ? "_cut" : ""),
        "Tap the Tech Village ring between villages " + a + " and " + b + ": a patch panel"
            + " on a span, a router (AS " + (65200 + a) + ", LAN 10.200." + a + ".0/24) at " + sub
            + "5/28 peering with both ISPs.",
        55_000, !cut,
        cut ? List.of() : List.of("ip route 100." + (64 + far) + ".0.0/24 10.200." + a + ".99"),
        cut ? List.of() : List.of("network 100." + (64 + far) + ".0.0/24",
            "neighbor " + sub + "1 default-originate", "neighbor " + sub + "2 default-originate"));
    var north = net.minecraft.core.Direction.NORTH;
    var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
    java.util.function.Function<ScenarioRun, byte[]> tapNic =
        r -> NetworkHub.deriveMac(r.terminal("tap").getComputerId(), 0);
    java.util.function.BiFunction<ScenarioRun, Boolean, byte[]> ispNic =
        (r, next) -> NetworkHub.deriveMac(
            WorldNetwork.get(r.level()).identity(r.level(), next ? a : b, WorldNetwork.ISP_ROUTER),
            next ? WorldNetwork.FIBER_NEXT_PORT : WorldNetwork.FIBER_PREV_PORT);
    if (!cut) {
      s.until("tap", "show bgp ipv4 unicast", "^0\\.0\\.0\\.0/0 via ", "the default route from village 1");
      s.mutate(
          r -> {
            String out = r.latestOutput("tap");
            var m = java.util.regex.Pattern.compile("(?m)^100\\.(\\d+)\\.([01])\\.0/24 via ").matcher(out == null ? "" : out);
            Set<String> seen = new HashSet<>();
            while (m.find()) seen.add(m.group(1) + "." + m.group(2));
            for (int v = 1; v <= 10; v++)
              for (int k = 0; k <= 1; k++)
                if (!seen.contains((64 + v) + "." + k) && !(v == far && k == 0))
                  throw new IllegalStateException("missing 100." + (64 + v) + "." + k + ".0/24:\n" + out);
          },
          "The tap learned every village /24 (twenty, its own static for village " + far + " aside) and the default.");
      s.send("tap", "exit");
      s.expect("tap", "/ >", "shell");
      s.note("On village " + a + "'s ISP: the tap is a dynamic neighbor; its /24 is in, its hijack and default are out"
          + " [exception: the scenario types into the far village ISP router's console, where a player would walk or"
          + " ssh there; docs/SCENARIO_EXCEPTIONS.md]");
      s.await(ispShows(a, "show bgp ipv4 unicast summary", "^\\*" + q(sub) + "5 +" + (65200 + a) + " .*Established +Up +1$", null),
          "village " + a + " lists the tap as a dynamic neighbor with 1 accepted prefix", 20_000);
      s.await(ispShows(a, "show bgp ipv4 unicast", "^10\\.200\\." + a + "\\.0/24 via " + q(sub) + "5 AS_PATH \\[" + (65200 + a) + "\\]",
              "^(100\\." + (64 + far) + "\\.0\\.0/24|0\\.0\\.0\\.0/0) via " + q(sub) + "5 "),
          "village " + a + " accepts 10.200." + a + ".0/24 from the tap, not its hijack of 100." + (64 + far) + ".0.0/24 or its default", 20_000);
      s.await(ispShows(far, "show ip route", "^10\\.200\\." + a + "\\.0/24 via .* Bgp", null),
          "village " + far + ", across the ring, routes to the tap's LAN", 20_000);
      s.note("From the tap's LAN PC: the villages' web sites and the ring's chat");
      s.send("pc", "curl http://100." + (64 + b) + ".0.10/");
      s.expect("pc", "<h1>Tech Village " + b + "</h1>", "village " + b + "'s site");
      s.expect("pc", "/ >", "shell");
      s.send("pc", "curl http://100.65.0.10/");
      s.expect("pc", "<h1>Tech Village 1</h1>", "village 1's site, across the ring");
      s.expect("pc", "/ >", "shell");
      s.sendFn("pc", r -> "chat " + WorldNetwork.chatAddress(WorldNetwork.get(r.level()).chatVillage()) + " --nick tapper",
          "chat 100.(64+K).0.20 --nick tapper   (K: the chat village, /ecm techvillage info)");
      s.expect("pc", "\\*\\*\\* Joined", "joined the ring's chat");
      s.send("pc", "hello from the tap");
      s.await(
          r -> {
            var level = r.level();
            var d = WorldNetwork.get(level);
            var host = ComputerHost.get(level.getServer(), d.identity(level, d.chatVillage(), WorldNetwork.CHAT));
            String log = host.instance() == null ? "" : ScenarioRun.screen(host.headlessDisplay());
            return log.contains("tapper joined from 10.200." + a + ".10") ? null : "discordd log:\n" + log;
          },
          "the chat server saw tapper join from 10.200." + a + ".10 (no NAT: the tap's own prefix is routed)",
          15_000);
      s.send("pc", "/quit");
      s.expect("pc", "\\*\\*\\* bye", "left the chat");
      return s.build();
    }
    s.until("pc", "ping 100." + (64 + b) + ".0.10 -n 1", "^1 packets sent, 1 received",
        "the tap's LAN reaches village " + b + "'s server");
    s.expect("pc", "/ >", "shell");
    s.await(ispShows(b, "show ip route", "^10\\.200\\." + a + "\\.0/24 via " + q(sub) + "5 dev eth2 Bgp", null),
        "village " + b + " routes the tap's LAN straight to it", 20_000);
    s.note("Break the ring between the tap and village " + a);
    s.mutate(r -> r.player().breakBlock(breakAt, north), "Break a span of the ring three blocks toward village " + a + ".");
    s.await(
        r -> {
          var m = CableNetworkManager.getInstance();
          if (m.areOnSameNetwork(tapNic.apply(r), ispNic.apply(r, true))) return "still on village " + a + "'s side";
          if (!m.areOnSameNetwork(tapNic.apply(r), ispNic.apply(r, false)) || !m.carrierOf(tapNic.apply(r)))
            return "lost village " + b + "'s side";
          return null;
        },
        "the tap stays on village " + b + "'s piece of the chord, village " + a + " is cut off",
        5_000);
    s.send("tap", "show bgp ipv4 unicast summary");
    s.expect("tap", "^ " + q(sub) + "2 +" + (65000 + b) + " .*Established", "village " + b + "'s session is untouched");
    s.until("tap", "show bgp ipv4 unicast summary", "^ " + q(sub) + "1 +" + (65000 + a) + " .*(Active|Connect|Idle)",
        "the session to village " + a + " times out (hold 9 s)");
    ping(s, "pc", "100." + (64 + b) + ".0.10", 2, 2, "village " + b + "'s server still answers");
    s.mutate(r -> r.player().place(span, breakAt, north), "Put a Fiber Span back in the gap.");
    s.until("tap", "show bgp ipv4 unicast summary", "^ " + q(sub) + "1 +" + (65000 + a) + " .*Established", "re-peered with village " + a);
    s.note("Always-On: the tap keeps its sessions while its chunk is unloaded [exception: the unload and reload are"
        + " the chunk's own hooks called by the scenario, as a GameTest can't unload a test's chunk;"
        + " docs/SCENARIO_EXCEPTIONS.md]");
    s.mutate(
        r -> {
          var pos = r.where("tap");
          r.player().installModule(pos, com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE.get());
          var be = r.terminal("tap");
          be.onChunkUnloaded();
          if (!ComputerHost.get(r.level().getServer(), be.getComputerId()).isHeadless())
            throw new IllegalStateException("the tap router did not stay running headless");
        },
        "Install a Module Expansion Card and an Always-On Module in the tap router, then leave (its chunk unloads).");
    s.waitMs(10_000, "longer than the 9 s hold time");
    s.await(
        r -> {
          var m = CableNetworkManager.getInstance();
          return m.areOnSameNetwork(tapNic.apply(r), ispNic.apply(r, true)) && m.carrierOf(tapNic.apply(r))
              ? null : "the unloaded tap fell off the fiber";
        },
        "the headless tap's NIC is still on the fiber (registered exit, last-known blocks)",
        5_000);
    s.await(ispShows(a, "show bgp ipv4 unicast summary", "^\\*" + q(sub) + "5 +" + (65200 + a) + " .*Established", null),
        "village " + a + " still has the headless tap Established", 15_000);
    s.mutate(r -> r.terminal("tap").onLoad(), "Come back: the chunk loads and the same computer reattaches.");
    s.send("tap", "show bgp ipv4 unicast summary");
    s.expect("tap", "^ " + q(sub) + "2 +" + (65000 + b) + " .*Established", "both sessions survived the unload");
    return s.build();
  }

  private static Scenario headless() {
    var b =
        nodes(
            "router_headless",
            "Always-On Module, a running child while detached, and same-instance reattachment."
                + " Manual mode: fly away to unload, check /ecm headless list, then return.",
            "pc");
    b.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            return List.of();
          }

          public void build(ScenarioRun r) {
            // A Module Expansion Card and an Always-On Module, clicked into the PC's left side.
            r.player().installModule(r.where("pc"), com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE.get());
          }
        });
    Map<ScenarioRun, ComputerInstance> original = new java.util.WeakHashMap<>();
    b.note("Built for you: the PC with a Module Expansion Card and an Always-On Module.");
    b.send("pc", "echo before-detach");
    b.expect("pc", "^before-detach$", "running kernel");
    b.send("pc", "sleep 1 &");
    b.expect("pc", PlayerKit.PROMPT, "a background job that runs on while the PC is unloaded");
    b.mutate(
        r -> {
          var be = r.terminal("pc");
          original.put(r, be.getComputer());
          be.onChunkUnloaded();
          var h = ComputerHost.get(r.level().getServer(), be.getComputerId());
          if (!h.isHeadless() || h.instance() != original.get(r))
            throw new IllegalStateException("headless instance lost");
        },
        "Unload the PC's chunk (walk away); the Always-On computer must remain in /ecm headless list"
            + " [exception: the chunk's unload and load hooks are called by the scenario, as a GameTest"
            + " can't unload a test's chunk; docs/SCENARIO_EXCEPTIONS.md]");
    b.waitMs(1300, "child executes while headless");
    b.mutate(
        r -> {
          var be = r.terminal("pc");
          be.onLoad();
          if (be.getComputer() != original.remove(r))
            throw new IllegalStateException("reattach rebooted");
        },
        "Return to reload the PC; no reboot banner should appear.");
    b.send("pc", "echo after-reattach");
    b.expect("pc", "^after-reattach$", "same live kernel");
    return b.build();
  }

  // Last: static initialisers run in source order, and the labs use the constants above.
  static {
    add(home());
    add(portForward());
    add(staticRoutes());
    add(wanDhcp());
    add(internet());
    add(fiber());
    add(bgp(2, false));
    add(bgp(10, false));
    add(bgp(2, true));
    add(chat());
    add(headless());
    add(village());
    add(playerFiber());
    add(portsFollowFacing());
    add(fiberTapLab());
  }
  //?}
}
