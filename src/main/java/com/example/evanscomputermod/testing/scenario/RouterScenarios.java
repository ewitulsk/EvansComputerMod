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
    add(chat());
    //? if <=1.21.1 {
    add(headless());
    add(village());
    add(playerFiber());
    add(fiberTapLab());
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
            for (var f : node.getValue().entrySet()) {
              var file = root.resolve(f.getKey());
              Files.createDirectories(file.getParent());
              Files.writeString(file, f.getValue());
            }
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

  /**
   * The chat service on one segment: chatd on "server", two clients reading /etc/chat.conf.
   * Control first: a port with no chatd gets the "refused" fix-it hint.
   */
  private static Scenario chat() {
    var b =
        nodes(
            "router_chat",
            "Instant messaging: chatd on 'server', 'alice' and 'bob' run chat (server from"
                + " /etc/chat.conf), talk, /who and /quit. Control: a port without chatd is"
                + " refused with a fix-it hint.",
            "server",
            "alice",
            "bob");
    b.decor(
        setup(
            Map.of(
                "server",
                Map.of("network.cfg", "iface eth0 10.95.0.1/24\n", "services.cfg", "chatd 7777 &\n"),
                "alice",
                Map.of("network.cfg", "iface eth0 10.95.0.2/24\n", "etc/chat.conf",
                    "server 10.95.0.1\nport 7777\nnick alice\n"),
                "bob",
                Map.of("network.cfg", "iface eth0 10.95.0.3/24\n", "etc/chat.conf",
                    "server 10.95.0.1\nnick bob\n")),
            new Segment("lan-alice", "server", 0, "alice", 0),
            new Segment("lan-bob", "server", 0, "bob", 0)));
    b.note("server runs 'chatd 7777 &' from services.cfg; cat /etc/chat.conf on alice and bob.");
    b.send("alice", "chat 10.95.0.1:7778");
    b.expect("alice", "refused", "control: no chat server on port 7778 (fix-it hint)");
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
  /**
   * A Tech Village network in miniature, with the exact startup files of village 5 and
   * real cables (no lab leads): ISP router DOWN (eth0, village access + DHCP) to a home
   * router's DOWN (eth0, WAN DHCP + NAT) through a buried run; the home router's UP (eth1,
   * LAN) to the PC's UP over a patch cable; ISP UP (eth1) to the server's UP.
   */
  private static Scenario village() {
    var b =
        Scenario.builder(
                "router_village",
                "Tech Village wiring: buried village cable from the ISP's DOWN face to a home"
                    + " router's WAN, patch cable over the router and PC (LAN), server on the"
                    + " ISP's UP face. Same files as village 5. Control: cut the village cable.")
            .timeLimit(55_000);
    b.host("isp", new BlockPos(0, 1, 0), null);
    b.host("server", new BlockPos(2, 1, 0), null);
    b.host("home", new BlockPos(6, 1, 0), null);
    b.host("pc", new BlockPos(7, 1, 0), null);
    b.link("server-lan", "isp", net.minecraft.core.Direction.UP, "server", net.minecraft.core.Direction.UP,
        Scenario.Path.from(new BlockPos(0, 2, 0)).go(net.minecraft.core.Direction.EAST, 2));
    b.link("village-cable", "isp", net.minecraft.core.Direction.DOWN, "home", net.minecraft.core.Direction.DOWN,
        Scenario.Path.from(new BlockPos(0, 0, 0)).go(net.minecraft.core.Direction.DOWN, 1)
            .go(net.minecraft.core.Direction.EAST, 6).go(net.minecraft.core.Direction.UP, 1));
    b.link("home-lan", "home", net.minecraft.core.Direction.UP, "pc", net.minecraft.core.Direction.UP,
        Scenario.Path.from(new BlockPos(6, 2, 0)).go(net.minecraft.core.Direction.EAST, 1));
    b.decor(
        setup(
            Map.of(
                "isp", WorldNetwork.configs(5, WorldNetwork.ISP_ROUTER, 5, List.of()),
                "server", WorldNetwork.configs(5, WorldNetwork.WEB, 5, List.of()),
                "home", WorldNetwork.configs(5, "house1.router", 5, List.of()),
                "pc", WorldNetwork.configs(5, "house1.pc", 5, List.of()))));
    b.note(
        "Terminals boot with village 5's files: cat router.cfg on isp and home, cat network.cfg"
            + " on pc. DOWN is eth0 and UP is eth1 on every terminal.");
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
            var south = net.minecraft.core.Direction.SOUTH;
            hands.place(panel, r.abs(new BlockPos(0, 2, 0)), south);
            hands.place(panel, r.abs(new BlockPos(8, 2, 0)), south);
            for (int x = 0; x <= 8; x++) hands.place(span, r.abs(new BlockPos(x, 3, 0)), south);
            // The tap: copper from the third PC's UP face, touching the fiber's side.
            hands.place(cable, r.abs(new BlockPos(4, 2, 1)), south);
            hands.place(cable, r.abs(new BlockPos(4, 3, 1)), south);
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
        r -> r.player().breakBlock(r.abs(breakAt), net.minecraft.core.Direction.SOUTH),
        "Break the middle Fiber Span: the run is cut.");
    b.waitMs(500, "carrier loss");
    ping(b, "west", "10.94.0.2", 1, 0, "no path with a span missing");
    b.mutate(
        r -> r.player().place(span, r.abs(breakAt), net.minecraft.core.Direction.SOUTH),
        "Place a Fiber Span back in the gap: the run carries traffic again.");
    b.waitMs(500, "link up again");
    ping(b, "west", "10.94.0.2", 2, 2, "west reaches east again");
    return b.build();
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
        setup(
            Map.of(
                "ispA", labIsp(1),
                "ispB", labIsp(2),
                "srvA", labServer(1),
                "srvB", labServer(2),
                "pc", tapPc(0))));
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
        + " fiber run; each has a server on its UP face. cat router.cfg on ispA shows the open"
        + " peering group TAPS.");
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
    var s =
        Scenario.builder(
                "router_ring_tap_" + a + (cut ? "_cut" : ""),
                "Tap the Tech Village ring between villages " + a + " and " + b + ": a patch panel"
                    + " on a span, a router (AS " + (65200 + a) + ", LAN 10.200." + a + ".0/24) at " + sub
                    + "5/28 peering with both ISPs.")
            .timeLimit(55_000)
            .asPlayer();
    var up = net.minecraft.core.Direction.UP;
    var north = net.minecraft.core.Direction.NORTH;
    s.host("tap", new BlockPos(0, 2, 0), null);
    s.host("pc", new BlockPos(1, 2, 0), null);
    s.host("ctl", new BlockPos(2, 1, 0), null);
    s.link("lan", "tap", up, "pc", up, Scenario.Path.from(new BlockPos(0, 3, 0)).go(net.minecraft.core.Direction.EAST, 1));
    var panel = com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get();
    var span = com.example.evanscomputermod.block.ModBlocks.FIBER_SPAN.get();
    var cable = com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get();
    s.decor(setup(Map.of("pc", tapPc(a))));
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
    if (!cut) {
      s.send("tap", "tcpdump -i eth0 -c 1");
      s.expect("tap", q(sub) + "[12]", "the ISPs' keepalives give away the link (" + sub + "1 and .2)");
      s.expect("tap", "/ >", "tcpdump done");
    }
    s.send("tap", "router on");
    s.expect("tap", "Router started", "router service");
    s.send("tap", "router");
    s.expect("tap", "router#", "router CLI");
    for (String line :
        tapConfig(65200 + a, a, sub + "5/28", sub + "1", 65000 + a, sub + "2", 65000 + b,
            cut ? List.of() : List.of("ip route 100." + (64 + far) + ".0.0/24 10.200." + a + ".99"),
            cut ? List.of() : List.of("network 100." + (64 + far) + ".0.0/24",
                "neighbor " + sub + "1 default-originate", "neighbor " + sub + "2 default-originate")))
      s.send("tap", line);
    s.expect("tap", "Configuration saved", "write memory");
    s.until("tap", "show bgp ipv4 unicast summary", "^ " + q(sub) + "1 +" + (65000 + a) + " .*Established", "peered with village " + a);
    s.until("tap", "show bgp ipv4 unicast summary", "^ " + q(sub) + "2 +" + (65000 + b) + " .*Established", "peered with village " + b);
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
      s.note("On village " + a + "'s ISP: the tap is a dynamic neighbor; its /24 is in, its hijack and default are out");
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
            return log.contains("tapper joined from 10.200." + a + ".10") ? null : "chatd log:\n" + log;
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
    s.note("Always-On: the tap keeps its sessions while its chunk is unloaded");
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
