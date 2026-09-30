# Routers, BGP and Tech Villages

This guide covers the router shipped by this mod. The CLI uses AOS-CX-style
contexts and a supported subset of commands; it does not implement every command
from a commercial switch. Networking and router services work in both builds.
Tech Village generation, fiber placement and SavedData recovery target Minecraft
1.21.1.

## First router: LAN, DHCP and an internet-facing port

Place a Terminal and use the Interface Probe to identify the ports you cabled.
For a north-facing terminal without expansion blocks, eth0 is down, eth1 up,
eth2 south, eth3 west and eth4 east. The screen face has no NIC. Expansion blocks
change the enumeration: probe again after changing the hardware. The stack accepts
up to 32 interfaces.

Cable your PCs to one LAN segment on eth0 and your upstream network to eth1.
Type these commands **inside the terminal**, one line at a time:

```text
router on
router
configure terminal
ip routing
interface eth0
ip address 192.168.1.1/24
ip nat inside
exit
interface eth1
ip dhcp
ip nat outside
exit
dhcp-server vrf default
pool lan
range 192.168.1.10 192.168.1.200
default-router 192.168.1.1
dns-server 1.1.1.1
lease 86400
enable
end
write memory
exit
```

`router on` enables the service at boot. `router` opens its CLI. `ip routing`
allows forwarding between interfaces. The WAN DHCP client learns its address,
gateway and DNS servers from the upstream DHCP server. It works on a physical
port with no IPv4 address yet. A configured upstream must provide a DHCP lease;
otherwise use the static WAN example below.

On a PC, create `/network.cfg` with `edit network.cfg`:

```text
iface eth0 dhcp
```

Restart that PC. DHCP is read during kernel boot. The router allocates a LAN
address and supplies the default gateway and DNS server. Test from the PC:

```text
ifconfig
ip route
ping 192.168.1.1 -n 2
ping 100.72.0.10 -n 2
traceroute 100.72.0.10
curl http://100.72.0.10/
```

The village 8 server address above is available when Tech Villages are enabled.
For arbitrary topologies, replace it with a reachable server. External HTTP needs
the host internet bridge described below. `curl` supports plain HTTP; this feature
does not add a TLS stack.

## CLI navigation and saving

| Prompt | Context | Enter it | Leave it |
|---|---|---|---|
| `router#` | Operational commands | `router` from shell | `exit` to shell |
| `router(config)#` | Global configuration | `configure terminal` | `exit` to operational |
| `router(config-if)#` | Interface | `interface ethN` | `exit` to global |
| `router(config-dhcp)#` | DHCP server | `dhcp-server vrf default` | `exit` to global |
| `router(config-dhcp-pool)#` | DHCP pool | `pool NAME` | `exit` to DHCP |
| `router(config-bgp)#` | BGP process | `router bgp ASN` | `exit` to global |
| `router(config-bgp-ipv4-uc)#` | IPv4 address family | `address-family ipv4 unicast` | `exit` to BGP |
| `router(config-route-map)#` | Policy entry | `route-map NAME permit SEQ` | `exit` to global |

`end` returns to the operational prompt from any configuration context.
`show running-config` prints a configuration that can be replayed. `write memory`
and `copy running-config startup-config` save it as `/router.cfg`. Leaving the CLI
keeps the router running. `router off`, issued from the shell, stops forwarding,
DHCP/NAT processing and BGP and removes the router's startup entry.

`router.cfg` and `services.cfg` have different purposes:

```text
# services.cfg — one startup command per line
router on
sshd &
httpd 80 &
```

The kernel reads `network.cfg` first, then replays service startup commands once.
Only router/switch service commands and background commands ending in `&` are
started by this file. `router on` updates its own entry while preserving other
entries. Save routing changes before restarting. `switch on` and the switch CLI's
`on` command likewise persist startup; `switch off` removes it.

The router CLI also works through an SSH shell: run `sshd &` on the router, connect
with the mod's `ssh` client, then type `router`. Each console/SSH connection has its
own CLI context and shares the running router configuration. Changes take effect
immediately. Changing BGP configuration recreates its process and its TCP sessions;
configure all neighbor lines before waiting for convergence.

## Interfaces, static routes and forwarding

For a static WAN address instead of DHCP:

```text
router
configure terminal
interface eth1
ip address 10.0.0.2/24
ip nat outside
exit
ip route 0.0.0.0/0 10.0.0.1 eth1
end
write memory
```

Other route examples:

```text
ip route 10.20.0.0/16 10.0.0.3 eth1
ip route 10.30.0.0/16 10.0.0.3
```

The optional interface is inferred by looking up the next hop. That next hop must
be reachable. An address creates a connected route automatically. Readdressing
replaces that interface's connected route without deleting static or learned
routes. The route table holds up to 512 entries, including competing routes.

Forwarding picks the longest prefix first, then administrative distance and metric.
Connected routes use distance 0, static 1, DHCP defaults 5, eBGP 20 and iBGP 200.
Only static routes are saved by the generic network configuration writer. DHCP
and BGP reconstruct their routes at startup. BGP exports are backed by actual
connected/static/DHCP entries; merely typing `network` does not create a route.

Forwarding is disabled by default. Enable it with `ip routing`, or disable it with
`no ip routing`. Forwarded IPv4 packets lose one TTL and get a new header checksum.
TTL expiry returns ICMP Time Exceeded; missing routes and unresolved neighbors
return Destination Unreachable. ICMP errors are rate limited and are not sent in
response to other errors, broadcast/multicast packets or invalid source addresses.
Closed unicast UDP ports return Port Unreachable. IPv4 fragmentation/reassembly
is not implemented; packets beyond the interface MTU receive an appropriate error.

## NAT and port forwarding

Mark LAN-facing ports `ip nat inside` and the upstream port `ip nat outside`.
Packets whose source route leads through an inside port are translated when they
leave an outside port. This includes prefixes learned over an inside BGP link,
not only directly connected LAN hosts. The router's own local services retain
their addresses. Return traffic is translated before the routing lookup.

Mappings are endpoint independent: the same inside address/port/protocol uses the
same outside port for multiple remote destinations. Reply filtering permits remote
endpoints to use an existing mapping. Dynamic ports are 49152–65535. A table holds
4096 mappings; exhaustion drops new flows instead of evicting live ones.

| Mapping | Idle timeout |
|---|---|
| UDP | 5 minutes |
| Established TCP | 2 hours 4 minutes |
| Opening/closing TCP | 4 minutes |
| ICMP Echo identifiers | 1 minute |

Static forwards use the outside interface's current address:

```text
configure terminal
ip nat inside source static tcp 192.168.1.10 80 8080
ip nat inside source static udp 192.168.1.20 53 1053
end
write memory
show ip nat translations
```

The TCP example maps outside port 8080 to the LAN web server's port 80. The UDP
example maps 1053 to 53. Static mappings reserve their ports and translate the
server's replies. NAT also handles inside clients contacting the outside address
(hairpin translation) and reverses addresses/identifiers in quoted ICMP errors.
Mappings expire in memory; they are rebuilt by traffic after a restart. The static
forward definitions persist in `router.cfg`.

## DHCP pools and leases

The supported DHCP server is local to a routed interface; DHCP relay is not
implemented. A pool's `default-router` must match that receiving interface's IP.
Ranges are within one /24 and replies advertise a /24 mask. Do not reuse an
address in different pools on the same segment. Lease duration is in seconds,
from 4 through 31536000. `disable` stops offering that pool; `enable` resumes it.

```text
configure terminal
dhcp-server vrf default
pool guest
range 192.168.2.10 192.168.2.100
default-router 192.168.2.1
dns-server 1.1.1.1
lease 3600
enable
end
show dhcp-server leases
write memory
```

Before using that pool, give another router interface `192.168.2.1/24` and connect
the guest segment there. DISCOVER/OFFER/REQUEST/ACK, renew/rebind, release, decline,
NAK and INFORM are handled. A declined address is quarantined for ten minutes.
Leases are written to `/router.leases`; unexpired leases survive a server restart.
The client retries on virtual/kernel time, renews at T1, broadcasts at T2, and
removes its address and DHCP default route on expiry. DNS comes from the lease.
This router does not provide a DNS forwarding service; point clients at a DNS
server they can reach.

## Two BGP routers

Connect A eth1 to B eth0. Configure the interface IPs and an attached network on
each side first. On A:

```text
router on
router
configure terminal
ip routing
interface eth0
ip address 100.80.0.1/24
exit
interface eth1
ip address 172.30.0.1/30
exit
router bgp 65100
bgp router-id 100.80.0.1
timers bgp 3 9
neighbor 172.30.0.2 remote-as 65101
address-family ipv4 unicast
neighbor 172.30.0.2 activate
network 100.80.0.0/24
end
write memory
```

On B, use eth0 `172.30.0.2/30`, eth1 `100.81.0.1/24`, AS 65101, router ID
`100.81.0.1`, neighbor `172.30.0.1 remote-as 65100`, and `network 100.81.0.0/24`.
Activate the neighbor in the IPv4 unicast address family. Then inspect:

```text
show bgp ipv4 unicast summary
show bgp ipv4 unicast neighbors
show bgp ipv4 unicast
show ip route
```

An established peer appears as `Established`. A router ID is a stable, unique IPv4
identifier; the neighbor address is the reachable TCP endpoint. Connections use
the kernel's TCP port 179 and source binding. `neighbor ADDRESS update-source ethN`
or `neighbor ADDRESS update-source IP` selects another local source; the remote
neighbor statement must match the source address it will actually see.

`timers bgp KEEPALIVE HOLD` uses seconds. Defaults are 60/180. Hold 0 disables the
hold timer; nonzero hold must be at least 3. The negotiated hold is the smaller
advertised hold. Direct carrier loss drops a bound BGP connection quickly;
silent failures are detected by the hold timer. Short timers help small labs.

Within `address-family ipv4 unicast`:

```text
redistribute connected
redistribute static
neighbor 172.30.0.2 default-originate
```

Redistribution adds matching route sources. `default-originate` advertises a
default even without a local default route; use it only when the router can
actually supply upstream connectivity. A `network 0.0.0.0/0` statement instead
requires an existing default route. iBGP is selected by using the same AS on both
sides. It does not prepend that AS, and it does not relay routes between iBGP
peers: route reflectors are not implemented.

## Prefix lists and route maps

Prefix lists are ordered by sequence. Entries are permit/deny, with optional
`ge`/`le` prefix-length bounds. No matching entry means deny. Route maps have
the same ordered, implicit-deny behavior. An entry without a match condition
matches all prefixes. Missing map/list names deny, so define a policy before
attaching it.

```text
configure terminal
ip prefix-list CUSTOMER seq 10 permit 100.80.0.0/24
ip prefix-list CUSTOMER seq 20 permit 10.80.0.0/16 ge 24 le 28
route-map FROM-CUSTOMER permit 10
match ip address prefix-list CUSTOMER
set local-preference 150
set metric 20
set community 65100:10
exit
router bgp 65101
address-family ipv4 unicast
neighbor 172.30.0.1 route-map FROM-CUSTOMER in
end
write memory
```

`neighbor ADDRESS route-map NAME out` applies an export policy. The supported set
operations are local preference, MED (`metric`), and a community (`ASN:VALUE` or a
32-bit integer). `set community` adds the community to the existing set. Local
preference is a local/iBGP decision input; eBGP-received local preference is
ignored before your inbound policy is applied. Route selection prefers higher
local preference, shorter AS path, lower origin, lower MED from the same
neighboring AS, eBGP over iBGP, then lower router ID/peer address. Locally originated
networks win over remote copies. Paths containing the local AS are rejected.
AS_SET membership survives export and each set counts as one hop in path length.

The wire layer supports BGP-4, four-byte AS numbers, legacy AS_TRANS/AS4_PATH,
IPv4 multiprotocol attributes, route refresh and communities. NO_EXPORT and
NO_ADVERTISE are respected. Unknown optional transitive attributes are preserved
with the partial bit on export. Recoverable malformed IPv4 UPDATE attributes
are treated as withdrawals; framing or NLRI damage that cannot be delimited
safely resets the session. Each peer's input is bounded to 64 KiB and its RIB to
512 prefixes. There is no IPv6 BGP, graceful restart, route reflection or OSPF.

## Tech Villages and customer connections

On a 1.21.1 server, ten sites are planned around world spawn at radius 5000 with a
seed-derived angular offset. A biome-source search moves ocean sites to the
nearest available non-ocean biome without generating distant chunks. Village N
uses AS `65000 + N`, access aggregate `100.(64 + N).0.0/16`, and server
`100.(64 + N).0.10`. For example village 3 is AS 65003 and server `100.67.0.10`.

The two backbone ports are eth0 (previous village) and eth1 (next village).
Each chord has a /30 under `172.31.N.0`. Eth2 feeds the house access switch;
eth3 feeds the local server. Village 1 uses eth4 `10.0.0.2/24` for the host
gateway and originates the default. Its inside ports NAT traffic toward eth4.
An expansion block supplies additional customer/peering ports.

House routers use their eth0 LAN with `192.168.1.1/24`, eth1 WAN DHCP and NAT.
The house PC uses DHCP on its private LAN. Houses can reuse the same private
subnet because each has a separate segment and its own translation table.
The access switch bridges house ports 1–6 in VLAN 1 to port 0 and ISP eth2.
Its customer port 7 is in VLAN 20 with port 8, which connects to ISP eth5
`100.(64 + N).2.1/24`. Cable a player PC to the access switch's port 7 and put
`iface eth0 dhcp` in its `network.cfg`. Use the probe to locate that port after
the structure is rotated. It leases `100.(64 + N).2.10` through `.200`.

Servers start `httpd` and `sshd`. ISP routers start `router` and `sshd`.
The twenty ISP router/server kernels boot headless even before their terrain
generates, so BGP can converge and the servers can communicate immediately.
Generated blocks attach to the same instances. Access switches and house
computers boot when their chunks load. If their access switch sleeps, its houses
have no upstream connectivity until it returns.

Operator commands, entered in **Minecraft chat**:

```text
/ecm techvillage list
/ecm techvillage tp 3
/ecm techvillage info 3
/ecm net links
/ecm net cut 3 4
/ecm net repair 3 4
/ecm headless list
```

Only adjacent ring sites can be cut with the command. Administrative repair clears
that command's cut; a physically broken generated span/pole must also be replaced.
Missing/unloaded chunks are intact. Loaded chunks materialize only their chord
fragments, with poles roughly every 16 blocks and conduits on ocean floors.
Placement runs after chunk promotion on a server tick. Boundary fragments wait
until surrounding chunks are loaded, so block updates and physics do not force
neighboring terrain to load. The logical link remains intact while they wait.
Break a generated pole/span to cut the edge. Place a crafted Pole/Fiber Span at
the broken position to repair it. Player builds away from generated edges do not
change the ring. The backbone's saved logical edges connect the ISP patch-panel
segments independently of intermediate chunk loading.

Templates contain role markers, not computer UUIDs. The provisioning processor
derives stable identities from world seed, village number and role, writes missing
configuration files, and sets `wasRunning`. Existing configuration files are kept.
Regenerate the seven templates and pools with `py -3 scripts/gen-tech-village.py`.
Generate models/recipes with `py -3 scripts/gen-tech-assets.py`.

## Add your own player AS

At village 3, probe a spare ISP port (for example eth6), cable it to your router's
eth1, and configure the ISP:

```text
router
configure terminal
interface eth6
ip address 172.30.3.1/30
exit
router bgp 65003
neighbor 172.30.3.2 remote-as 65200
address-family ipv4 unicast
neighbor 172.30.3.2 activate
end
write memory
```

On your router, assign eth1 `172.30.3.2/30`, enable routing, use AS 65200,
add and activate neighbor `172.30.3.1 remote-as 65003`, and originate an actually
configured LAN prefix such as `10.200.0.0/24`. Give your PCs a default route to
your router. Your AS learns the village routes and village 1's default; the ring
learns your LAN prefix. Choose unique prefixes/AS numbers. An ISP port accepts
only neighbors explicitly configured by its operator; there is no wildcard peer.

## Always-On computers, saves and the Windows uplink

Install a Module Expansion Card and an Always-On Module in a terminal bay. On
chunk unload its instance detaches, keeps its worker, network state and processes,
and reattaches without reboot on load. Other terminals retain the existing unload
shutdown behavior. Server restart boots saved headless nodes again; live TCP/NAT
state is not serialized, and BGP reconverges. The infrastructure nodes do not
consume the player headless cap. This feature does not cover moving Sable ships.

Server launch properties:

```text
-Devanscomputermod.headlessCap=64
-Devanscomputermod.techVillages=true
-Devanscomputermod.internetProxy=true
```

The cap bounds detached player computers; if it is reached, additional terminals
shut down on unload. These are JVM system properties supplied to the Minecraft
server, not router commands. Turning off automatic village boot still allows
saved player Always-On computers to recover.

Files live in `<world-save>/computer-data/<UUID>/`. A legacy working-directory
`computer-data/<UUID>` is copied on first use when the world destination is absent.
The old directory is retained, and existing world files are not overwritten.
`ecm_network` SavedData stores sites, node port counts, last-known cables/NIC
positions, generated fiber positions and cuts. Unloaded cable regions retain
their last topology snapshot; loaded regions refresh it.

On Windows, the unprivileged Java socket proxy supplies gateway `10.0.0.1`.
It answers ARP and DHCP and bridges TCP/UDP over ordinary host sockets. Village
1's WAN connects to it through a logical gateway segment. A normal terminal can
also use a cable segment reaching the Internet Gateway block at world origin.
The host OS supplies outbound connectivity and firewall policy. UDP allows DNS;
TCP allows plain HTTP. The proxy bounds flows to 256 and receive queues to 256
frames; TCP buffers are bounded and replies are acknowledged/retransmitted.
It does not emulate raw ICMP echo to the real internet, inbound public listeners,
IPv6 or fragmented IP. Use `curl`/DNS to check external access. Linux retains the
existing TAP setup in `scripts/setup-tap.sh`.

The compiled kernel is cached by file content hash. Chicory shares its compiled
module across instances. The pinned Wasmtime Java binding cannot serialize
compiled modules or supply a per-store epoch callback, so Wasmtime retains cached
metadata/bytes and uses an engine per instance for cancellation isolation. It
recompiles for that engine. Interrupting one computer therefore cannot advance
another computer's cancellation epoch.

## Diagnosis and implementation map

Start with `ifconfig`/the probe to check the intended interface and carrier. Then
check `show ip route`, `show arp`, `show dhcp-server leases`,
`show ip nat translations`, and `show bgp ipv4 unicast summary`. A correct route
without an ARP neighbor usually means wrong cabling/addressing. An established
peer without a route usually means missing `network`, a policy denial, an AS loop,
or an unreachable next hop. `traceroute HOST [MAX-HOPS]` sends ICMP Echo probes
with increasing TTL; `*` means no matching response within the receive timeout.

| Component | Responsibility |
|---|---|
| `rust/crates/ecm-net` | IPv4 forwarding, sockets, TCP, ICMP errors, DHCP client and route selection |
| `rust/crates/ecm-router` | Pure CLI/config model, NAPT and DHCP server; takes frames/time without host calls |
| `rust/crates/ecm-bgp` | Pure wire codec, FSM/timers, Adj-RIB-In/Out, Loc-RIB, best path and policies |
| Kernel `router_svc.rs` / `bgp_svc.rs` | Console/SSH integration, persistence, nonblocking TCP adapter and FIB installation |
| `ComputerHost` / `WorldNetwork` | Server-owned instance lifetimes and world-save topology/identities |
| `worldgen/*` and template generators | Ring placement, role provisioning, chunk-local fiber and jigsaw content |
| `InternetProxy` | Ethernet/host TCP and UDP adapter, ARP/DHCP and bounded flow state |

All protocol engines use caller-supplied time and bounded state. Pure tests and
the simulator use virtual time. The kernel owns networking; Java supplies NIC
frames and schedules worker ticks. This avoids blocking the Minecraft server
thread on BGP connections, DHCP retries or host internet sockets.

Targeted verification commands (see `TESTING.md` for receipts):

```powershell
scripts/Test.ps1 -Area router -Rust ecm-net,ecm-router,ecm-bgp,terminal-os
scripts/Test.ps1 -Area router-sim -Scenarios 15_router,16_bgp,17_bgp
scripts/Test.ps1 -Area tech-world -GameTests ecm_router -McVersion 1.21.1
scripts/Test.ps1 -Area proxy -JUnit InternetProxyTest -McVersion 26.1
scripts/Test.ps1 -Area host -JUnit KernelHostIntegrationTest -McVersion 26.1
scripts/Test.ps1 -Area cancellation -Wasmtime -JUnit InterruptIsolationTest -McVersion 26.1
```

The ring simulator cuts one edge, verifies the long route, then isolates the source
and requires zero echo replies. The in-world tests use real kernels and Minecraft
segments. Each in-world case has a wall-clock limit below one minute. A pass
requires executed tests and markers, not just a successful build.
