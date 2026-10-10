package com.example.evanscomputermod.testing.scenario;

import com.example.evanscomputermod.computer.*;
import java.nio.file.Files;
import java.util.*;
import net.minecraft.core.BlockPos;

/** Playable router laboratories. Startup files and steps are shared with GameTests. */
public final class RouterScenarios {
  public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
  private static final Map<ScenarioRun, List<Segment>> ACTIVE_LINKS = new java.util.WeakHashMap<>();

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
    //? if <=1.21.1 {
    add(headless());
    //?}
  }

  private static void add(Scenario s) {
    ALL.put(s.name, s);
  }

  private static Scenario.Builder nodes(String name, String description, String... names) {
    var b = Scenario.builder(name, description).timeLimit(55_000);
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

  private record Segment(String name, String a, int pa, String b, int pb) {}

  private static void edge(ScenarioRun r, Segment s, boolean intact) {
    CableNetworkManager.getInstance()
        .logicalLink(
            key(r, s.name),
            NetworkHub.deriveMac(r.terminal(s.a).getComputerId(), s.pa),
            NetworkHub.deriveMac(r.terminal(s.b).getComputerId(), s.pb),
            intact);
  }

  private static String key(ScenarioRun r, String name) {
    return "router-lab-"
        + r.terminal(r.scenario().nodes.keySet().iterator().next()).getComputerId()
        + "-"
        + name;
  }

  private static Scenario.Decor setup(Map<String, Map<String, String>> files, Segment... segments) {
    return new Scenario.Decor() {
      public List<BlockPos> footprint() {
        return List.of();
      }

      public void build(ScenarioRun r) {
        try {
          for (var node : files.entrySet()) {
            var root = ComputerStorage.path(r.terminal(node.getKey()));
            Files.createDirectories(root);
            for (var f : node.getValue().entrySet())
              Files.writeString(root.resolve(f.getKey()), f.getValue());
          }
          for (var s : segments) edge(r, s, true);
          ACTIVE_LINKS.put(r, List.of(segments));
        } catch (Exception e) {
          throw new IllegalStateException("Router lab provisioning", e);
        }
      }

      public void clear(ScenarioRun r) {
        if (r.terminal(r.scenario().nodes.keySet().iterator().next()) == null) return;
        for (var s : segments) CableNetworkManager.getInstance().removeLogicalLink(key(r, s.name));
        ACTIVE_LINKS.remove(r);
      }
    };
  }

  public static List<String> patchLeads(ScenarioRun r) {
    return ACTIVE_LINKS.getOrDefault(r, List.of()).stream()
        .map(s -> s.name + ": " + s.a + " eth" + s.pa + " <-> " + s.b + " eth" + s.pb)
        .toList();
  }

  public static boolean patch(ScenarioRun r, String name, boolean intact) {
    for (var s : ACTIVE_LINKS.getOrDefault(r, List.of()))
      if (s.name.equals(name)) {
        edge(r, s, intact);
        return true;
      }
    return false;
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
            "DHCP LAN, WAN NAT, route/ARP/lease displays, traceroute, and forwarding-off control."
                + " Logical patch leads keep each port isolated.",
            "client",
            "router",
            "server");
    b.decor(
        setup(
            Map.of(
                "client",
                Map.of("network.cfg", "iface eth0 dhcp\n"),
                "server",
                host("10.90.0.3/24", null),
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
            new Segment("lan", "client", 0, "router", 0),
            new Segment("wan", "router", 1, "server", 0)));
    b.note(
        "Preconfigured startup files are readable with cat router.cfg / cat network.cfg. Open the"
            + " labelled client first.");
    b.waitMs(1200, "DHCP DORA");
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
    var lan = new HashMap<>(host("192.168.91.10/24", "192.168.91.1"));
    lan.put("services.cfg", "httpd 80 &\n");
    lan.put("index.html", "<h1>router-forward-ok</h1>\n");
    b.decor(
        setup(
            Map.of(
                "lan",
                lan,
                "wan",
                host("10.91.0.3/24", null),
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
                        + "ip nat inside source static tcp 192.168.91.10 80 8080\n")),
            new Segment("lan", "lan", 0, "router", 0),
            new Segment("wan", "router", 1, "wan", 0)));
    b.waitMs(700, "HTTP listener");
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
    b.decor(
        setup(
            Map.of(
                "client",
                host("10.92.1.10/24", "10.92.1.1"),
                "r1",
                router(
                    address(0, "10.92.1.1/24")
                        + address(1, "172.30.92.1/30")
                        + "ip route 10.92.2.0/24 172.30.92.2\n"),
                "r2",
                router(
                    address(0, "172.30.92.2/30")
                        + address(1, "10.92.2.1/24")
                        + "ip route 10.92.1.0/24 172.30.92.1\n"),
                "server",
                host("10.92.2.10/24", "10.92.2.1")),
            new Segment("lan1", "client", 0, "r1", 0),
            new Segment("transit", "r1", 1, "r2", 0),
            new Segment("lan2", "r2", 1, "server", 0)));
    ping(b, "client", "10.92.2.10", 2, 2, "two-router static path");
    b.send("client", "traceroute 10.92.2.10 5");
    b.expect("client", "^3\\s+10\\.92\\.2\\.10\\s*$", "TTL probes reach destination at hop three");
    b.expect("client", "/ >", "traceroute returned to shell");
    b.send("r1", "router", "show ip route", "configure terminal", "no ip routing", "end", "exit");
    b.expect("r1", "/ >", "shell");
    ping(b, "client", "10.92.2.10", 1, 0, "routing disabled control");
    return b.build();
  }

  private static Scenario bgp(int count, boolean policies) {
    String name =
        policies ? "router_bgp_policy" : count == 2 ? "router_bgp_pair" : "router_bgp_ring";
    String[] names = new String[count + 2];
    for (int i = 0; i < count; i++) names[i] = "r" + (i + 1);
    names[count] = "client";
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
    Map<String, Map<String, String>> files = new HashMap<>();
    List<Segment> links = new ArrayList<>();
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
      links.add(new Segment("ring" + i, "r" + i, 1, "r" + next, 0));
    }
    files.put("client", host("100.91.0.10/24", "100.91.0.1"));
    files.put("server", host("100." + (90 + count) + ".0.10/24", "100." + (90 + count) + ".0.1"));
    links.add(new Segment("client", "client", 0, "r1", 2));
    links.add(new Segment("server", "r" + count, 2, "server", 0));
    b.decor(setup(files, links.toArray(Segment[]::new)));
    String target = "100." + (90 + count) + ".0.10";
    b.waitMs(3500, "BGP OPEN/KEEPALIVE and UPDATE convergence");
    ping(b, "client", target, 2, 2, "learned route and return path");
    b.send("r1", "router", "show bgp ipv4 unicast summary");
    b.expect("r1", "Established", "BGP established");
    b.send("r1", "show bgp ipv4 unicast", "show ip route");
    if (policies) {
      b.send("r1", "show bgp ipv4 unicast");
      b.expect("r1", "100.92.0.0/24", "allowed aggregate");
      b.expect("r1", "local-pref 150 MED 20 community 65101:10", "inbound attributes applied");
      b.expect("r1", "router#", "policy table completed");
      b.mutate(
          r -> {
            var output = r.latestOutput("r1");
            if (output == null
                || java.util.regex.Pattern.compile("(?m)^172\\.29\\.").matcher(output).find())
              throw new IllegalStateException("INBOUND leaked denied transit prefixes");
          },
          "Transit 172.29 prefixes are redistributed by r2 but denied by INBOUND. Inspect the"
              + " absence of transit routes in show bgp ipv4 unicast.");
    }
    if (count == 10) {
      var direct = links.get(count - 1);
      var alternate = links.get(0);
      b.mutate(
          r -> edge(r, direct, false),
          "Run /ecm scenario link ring10 down to cut the direct r10-r1 lead.");
      b.waitMs(3500, "withdrawal and nine-hop route");
      ping(b, "client", target, 1, 1, "alternate ring path");
      b.mutate(
          r -> edge(r, alternate, false), "Run /ecm scenario link ring1 down as well: isolate r1.");
      b.waitMs(3500, "isolation withdrawal");
      ping(b, "client", target, 1, 0, "no route when isolated");
      b.mutate(
          r -> {
            edge(r, direct, true);
            edge(r, alternate, true);
          },
          "Run /ecm scenario link ring10 up and /ecm scenario link ring1 up to repair.");
      b.waitMs(3500, "repair convergence");
      ping(b, "client", target, 1, 1, "repaired connectivity");
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
    b.decor(
        setup(
            Map.of(
                "client",
                Map.of("network.cfg", "iface eth0 dhcp\n"),
                "isp",
                isp,
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
            new Segment("lan", "client", 0, "router", 0),
            new Segment("wan", "router", 1, "isp", 0)));
    b.waitMs(1800, "WAN and LAN DHCP");
    b.send("router", "ifconfig");
    b.expect("router", "10.93.0.10", "WAN lease");
    b.expect("router", "/ >", "ifconfig returned to shell");
    b.send("router", "ip route");
    b.expect("router", "10.93.0.1", "DHCP default gateway");
    b.expect("router", "/ >", "route display returned to shell");
    b.send("client", "curl http://10.93.0.1/index.html");
    b.expect("client", "wan-dhcp-ok", "HTTP through leased WAN NAT");
    return b.build();
  }

  private static Scenario internet() {
    var b =
        nodes(
            "router_internet",
            "The userspace gateway bridges real host TCP sockets for a statically addressed client (the gateway serves no DHCP). An isolated local HTTP fixture"
                + " avoids reliance on a public website; manual mode can also curl a public"
                + " plain-HTTP URL.",
            "client");
    b.decor(
        new Scenario.Decor() {
          record Fixture(java.net.ServerSocket listener, String lead) {}

          final Map<ScenarioRun, Fixture> fixtures = new java.util.WeakHashMap<>();

          public List<BlockPos> footprint() {
            return List.of();
          }

          public void build(ScenarioRun r) {
            try {
              var root = ComputerStorage.path(r.terminal("client"));
              Files.createDirectories(root);
              Files.writeString(
                  root.resolve("network.cfg"),
                  "iface eth0 10.0.0.50/24\nroute default via 10.0.0.1 dev eth0\ndns 1.1.1.1\n");
              NetworkHub.getInstance().enableInternetProxy();
              String lead = "internet-router-lab-" + r.terminal("client").getComputerId();
              CableNetworkManager.getInstance()
                  .logicalLink(
                      lead,
                      NetworkHub.deriveMac(r.terminal("client").getComputerId(), 0),
                      InternetProxy.MAC,
                      true);
              var listener = new java.net.ServerSocket(0);
              fixtures.put(r, new Fixture(listener, lead));
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
              Files.writeString(
                  root.resolve("gateway-test-url.txt"),
                  "http://" + address + ":" + listener.getLocalPort() + "/\n");
            } catch (Exception e) {
              throw new IllegalStateException(e);
            }
          }

          public void clear(ScenarioRun r) {
            var fixture = fixtures.remove(r);
            if (fixture == null) return;
            try {
              fixture.listener().close();
            } catch (Exception ignored) {
            }
            CableNetworkManager.getInstance().removeLogicalLink(fixture.lead());
          }
        });
    b.waitMs(1200, "static address on the gateway LAN");
    b.send("client", "ifconfig");
    b.expect("client", "10.0.0.50", "static gateway-LAN address (the gateway serves no DHCP)");
    b.expect("client", "/ >", "ifconfig returned to shell");
    b.mutate(
        r -> {
          try {
            var url =
                Files.readString(
                        ComputerStorage.path(r.terminal("client")).resolve("gateway-test-url.txt"))
                    .trim();
            r.terminal("client").getComputer().sendInput("curl " + url + "\n");
          } catch (Exception e) {
            throw new IllegalStateException(e);
          }
        },
        "Read cat gateway-test-url.txt, then curl that URL to exercise the real host socket"
            + " proxy.");
    b.waitMs(6000, "host TCP round trip");
    b.mutate(
        r -> {
          if (!ScenarioRun.screen(r.terminal("client").getDisplay()).contains("host-socket-ok"))
            throw new IllegalStateException("Host socket response absent");
        },
        "Expected HTTP body: host-socket-ok. External ICMP and HTTPS are intentionally"
            + " unsupported.");
    return b.build();
  }

  private static Scenario fiber() {
    var b =
        nodes(
            "router_fiber",
            "Physical six-way fiber joins, removal/repair, serialized states, a fiber riser onto"
                + " the patch panel, and Always-On Module model inspection.",
            "pc");
    b.decor(
        new Scenario.Decor() {
          public List<BlockPos> footprint() {
            return List.of(new BlockPos(0, 1, 3), new BlockPos(10, 5, 7));
          }

          public void build(ScenarioRun r) {
            var l = r.level();
            var center = r.abs(new BlockPos(5, 4, 5));
            l.setBlock(
                center,
                com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get().defaultBlockState(),
                3);
            for (var d : net.minecraft.core.Direction.values())
              l.setBlock(
                  center.relative(d),
                  com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get().defaultBlockState(),
                  3);
            // A free-floating riser: spans need no support and join the panel below.
            for (int y = 3; y <= 5; y++)
              l.setBlock(
                  r.abs(new BlockPos(9, y, 5)),
                  com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get().defaultBlockState(),
                  3);
            l.setBlock(
                r.abs(new BlockPos(9, 2, 5)),
                com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL
                    .get()
                    .defaultBlockState(),
                3);
            r.terminal("pc").getModuleBays().installCard(0);
            r.terminal("pc")
                .getModuleBays()
                .installModule(
                    com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE
                        .get()
                        .getDefaultInstance(),
                    0);
          }
        });
    b.note(
        "Inspect the labelled PC's installed Always-On Module, the rack patch panel with a"
            + " floating fiber riser on it, and the six-way fiber cross. Break the east arm, then"
            + " replace it with Fiber Span: only that arm must change.");
    b.mutate(
        r -> {
          var l = r.level();
          var s = l.getBlockState(r.abs(new BlockPos(5, 4, 5)));
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
          var p = r.abs(new BlockPos(6, 4, 5));
          r.level().removeBlock(p, false);
          if (r.level()
              .getBlockState(p.west())
              .getValue(com.example.evanscomputermod.block.NetworkCableBlock.EAST))
            throw new IllegalStateException("Disconnected east arm remained");
        },
        "Break the east neighbor of the six-way center; its east arm must disappear.");
    b.mutate(
        r -> {
          var p = r.abs(new BlockPos(6, 4, 5));
          var l = r.level();
          l.setBlock(
              p,
              com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get().defaultBlockState(),
              3);
          if (!l.getBlockState(p.west())
              .getValue(com.example.evanscomputermod.block.NetworkCableBlock.EAST))
            throw new IllegalStateException("Repaired east arm missing");
        },
        "Replace the east neighbor; the arm reconnects.");
    //? if <=1.21.1 {
    b.mutate(
        r -> {
          var p = r.abs(new BlockPos(5, 4, 5));
          var state = r.level().getBlockState(p);
          var saved = net.minecraft.nbt.NbtUtils.writeBlockState(state);
          var loaded =
              net.minecraft.nbt.NbtUtils.readBlockState(
                  r.level().holderLookup(net.minecraft.core.registries.Registries.BLOCK), saved);
          if (!state.equals(loaded))
            throw new IllegalStateException("Fiber connection state failed save/reload");
        },
        "Save and reload the world: connection properties persist in block-state NBT.");
    //?}
    return b.build();
  }

  //? if <=1.21.1 {
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
            var be = r.terminal("pc");
            be.getModuleBays().installCard(0);
            be.getModuleBays()
                .installModule(
                    com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE
                        .get()
                        .getDefaultInstance(),
                    0);
          }
        });
    Map<ScenarioRun, ComputerInstance> original = new java.util.WeakHashMap<>();
    b.send("pc", "echo before-detach");
    b.expect("pc", "^before-detach$", "running kernel");
    b.mutate(
        r -> {
          var be = r.terminal("pc");
          original.put(r, be.getComputer());
          be.onChunkUnloaded();
          var h = ComputerHost.get(r.level().getServer(), be.getComputerId());
          if (!h.isHeadless() || h.instance() != original.get(r))
            throw new IllegalStateException("headless instance lost");
          original.get(r).sendInput("sleep 1 &\n");
        },
        "Unload the PC's chunk; the Always-On computer must remain in /ecm headless list.");
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
  //?}
}
