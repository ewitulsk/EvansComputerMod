# Routers, BGP and Tech Villages

This guide covers the router shipped by this mod. The CLI uses AOS-CX-style
contexts and a supported subset of commands; it does not implement every command
from a commercial switch. Networking and router services work in both builds.
Tech Village generation, fiber placement and SavedData recovery target Minecraft
1.21.1.

## Playable testing laboratories

In a creative test world with cheats enabled, stand in a clear area and run:

```text
/ecm scenario spawn router_home manual
/ecm scenario commands router_home
/ecm scenario links
```

The command places labelled computers, writes their startup configurations,
connects isolated patch leads to the specified NICs, and boots the computers.
`manual` leaves you in control. Open each labelled terminal and follow the
walkthrough; router commands belong in the terminal, while `/ecm` commands
belong in Minecraft chat. Read `cat router.cfg`, `cat network.cfg`, and
`cat services.cfg` to inspect the setup. A lab router's configured `eth0`,
`eth1`, and `eth2` are logical test leads, so no Interface Probe guesswork is
needed. They do not connect to other nearby labs.

Use `auto` to watch the walkthrough execute, or `fast` for the automated checks
without the presentation delay. For example:

```text
/ecm scenario spawn router_bgp_ring fast
/ecm scenario status
/ecm scenario rerun
/ecm scenario clear
```

`clear` removes the placed blocks and lab leads; save your own work before
spawning a lab because its marked footprint is cleared. All thirteen definitions
also run as `ecm_router_scenarios` GameTests on Minecraft 1.21.1. Protocol labs
(and `router_chat`) are available in both versions; headless reattachment,
`router_village` and `router_player_fiber` are 1.21.1-specific.

| Scenario | What to test and expected result |
|---|---|
| `router_home` | LAN DHCP assigns `192.168.90.10`; ping and two-hop traceroute reach `10.90.0.3` through NAT. Inspect routes, ARP, leases, translations and running config; save with `write memory`. `router off` stops forwarding. Restore with `router on`. |
| `router_port_forward` | WAN `curl http://10.91.0.2:8080/index.html` returns `router-forward-ok` from the private HTTP server. The same URL works from the LAN through hairpin NAT. Port 8081 fails. |
| `router_static` | Two routers forward between `10.92.1.0/24` and `10.92.2.0/24`; traceroute reaches the server at hop three. `no ip routing` on r1 prevents replies. |
| `router_wan_dhcp` | Router WAN learns `10.93.0.10`, DNS and default gateway `10.93.0.1`; a separate private DHCP pool serves its LAN. Client HTTP through NAT returns `wan-dhcp-ok`. |
| `router_bgp_pair` | Two four-byte ASNs establish source-bound TCP/179 sessions and exchange IPv4 prefixes; client/server ping works. Inspect `show bgp ipv4 unicast summary`, the BGP table and installed routes. |
| `router_bgp_ring` | Ten ASNs form a ring. Cut `ring10`: the alternate nine-router path works. Also cut `ring1`: r1 is isolated and ping fails. Repair both: sessions and connectivity recover. |
| `router_bgp_policy` | r1's inbound prefix-list/route-map accepts `100.92.0.0/24` and applies local preference 150, MED 20 and community `65101:10`. r2 also redistributes transit prefixes, which must be absent from r1's BGP table. Inspect `router.cfg` and edit policies in the CLI. |
| `router_internet` | A static `10.0.0.50` client (the Internet Gateway serves no DHCP) plus a real host TCP socket returns `host-socket-ok` from an isolated local HTTP fixture. Read `cat gateway-test-url.txt` and curl its URL. Requires an active host IPv4 interface; public Internet is unnecessary for this check. |
| `router_headless` | An installed Always-On Module preserves the same running kernel and a child during detach/reattach. In manual mode fly far enough to unload its chunk, inspect `/ecm headless list`, return and run `echo after-reattach`. |
| `router_fiber` | Inspect the patch panel with a free-floating fiber riser on it and the installed module. All six center fiber arms connect. Break the east neighbor: only the east arm disappears. Replace it: the arm returns. The riser joins the panel below and has no open arm at its top. Connection properties survive block-state serialization. |
| `router_village` | A Tech Village network in miniature with village 5's startup files and real cables: the ISP's DOWN face feeds a buried cable to a home router's DOWN face (WAN DHCP), a patch cable joins the home router's and PC's UP faces (LAN), the web server hangs off the ISP's UP face (the data center LAN). The PC leases `192.168.1.x`, the home router `100.69.1.x`; ping and `curl` reach `100.69.0.10`. Cut the village cable: the ping fails. |
| `router_chat` | `chatd` on one computer, `alice` and `bob` run `chat` (server and nick from `/etc/chat.conf`), exchange messages, `/who`, `/quit`. Control: a port without `chatd` gets the "refused" fix-it hint. |
| `router_player_fiber` | Built by hand: a Fiber Patch Panel on two PCs' UP faces (eth1) and a run of Fiber Span between them. Ping crosses the fiber; breaking a span stops it and placing it back repairs it. Control: copper cable touching the fiber (not through a panel) is not connected. |

Ring failure controls work in Minecraft chat, and affect the most recently
spawned lab:

```text
/ecm scenario link ring10 down
/ecm scenario link ring1 down
/ecm scenario link ring10 up
/ecm scenario link ring1 up
```

Wait about four seconds after each topology change before inspecting BGP or
pinging. Manual labs boot their configured services but do not run the failure
controls for you. Their startup files, commands, and expected outcomes are the
same definitions used by the automated GameTests.

## Fiber models and village arrival

The fiber assets are authored in Blockbench, with editable projects in `models/`.
Spans have a jacketed cable and sealed couplers; patch panels have duplex fiber
sockets; the module is a circuit board with gold contacts. Run
`py -3 scripts/gen-tech-assets.py` to regenerate recipes and export these projects.
This preserves the authored textures instead of replacing them with vanilla
placeholders.

Fiber Span uses six independent neighbor properties. Placement, removal and
replacement update only the affected connections; it joins other fiber blocks and
patch panels. It needs no support: generated fiber floats over valleys and runs
straight through hills, trees, water and buildings. There is no pole block.
Patch panels face opposite your placement direction.

Fiber Span carries Ethernet like network cable, but it joins only fiber and Fiber
Patch Panels: a copper cable or a computer face touching a span is not connected, so
put a patch panel where copper meets fiber. A panel on a computer's face (or at the end of
a cable) plus a run of Fiber Span to another panel makes one segment, in loaded chunks
and, with the last-known topology, across unloaded ones (`router_player_fiber`). The
generated ring's own fiber cannot be tapped; its links are described in
[the Tech Village network](TECH_VILLAGE_NETWORK.md#5-the-fiber-links-physically-and-logically).

`/ecm techvillage tp 3` loads and resolves the actual generated structure and
places you outside the ISP's front door facing it. The offset rotates with the
building. If that village did not generate (structures disabled, or chunks
generated by an older version of the mod), the command reports the failure and
cancels the teleport. Fresh normal and superflat worlds both generate villages and
fiber.

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
otherwise use the static WAN example below. The Internet Gateway never serves DHCP, so a WAN cabled
straight to it needs a static address in `10.0.0.0/24` with `10.0.0.1` as the
gateway.

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

The village 8 web server address above is available when Tech Villages are enabled.
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

The full network architecture of the ten Tech Villages (address plan, every role and
port, annotated ISP and home router configurations, the fiber links, the Data Center,
chat, a verification runbook with expected output, and joining as a customer or as
your own AS) is in **[the Tech Village network](TECH_VILLAGE_NETWORK.md)**. In short:

On a 1.21.1 server, ten sites are planned around world spawn at radius 5000 with a
seed-derived angular offset. A biome-source search (no chunks generated) moves each
site to the nearest biome that has vanilla villages; the biome picks the village
style: plains, desert, savanna, snowy or taiga (plains where no village biome is
found, and in superflat). Village N uses AS `65000 + N`.

Each site is a normal vanilla village of its style grown from an ISP building:
streets, houses, job sites and villagers come from the vanilla jigsaw pools. The ISP
is the start piece: a building in the style's palette with a lattice mast through
its roof, two Fiber Patch Panels on the mast top, the ISP router (with an Interface
Block for extra ports) and a sign naming the village and AS. Its east side grows the
**Data Center**: a rack row with the village web server, the ring's chat server in one
village, and free racks whose UP faces are already on the data center LAN. About half
of the residential houses (at least two) have a home router and a PC on a desk; which
ones is fixed by the world seed.

**Village network (real cables).** The ISP router's DOWN face (eth0) feeds a cable
buried one block under the streets. It rises into every networked house to the
home router's DOWN face (eth0, WAN). A short patch cable over the router and PC
joins their UP faces (eth1, LAN). On terminals with a horizontal screen, DOWN is
always eth0 and UP eth1. The router's other ports are cabled by code that knows the
building's rotation: eth1 along the ceiling and overhead into the Data Center, eth2
and eth3 up the mast to the panels toward the previous and the next village. No two
of these runs touch.

| ISP router port | Use |
|---|---|
| eth0 (DOWN, cable) | Village access LAN `100.(64+N).1.1/24`; DHCP pool `.10`-`.200` for house routers and players |
| eth1 (UP, cable to the Data Center) | Data center LAN `100.(64+N).0.1/24`: web server `.10`, chat server `.20` (one village), DHCP `.100`-`.199` for added racks |
| eth2 / eth3 (cable up the mast to a panel, then fiber) | Fiber to the previous / next village, `/30`s under `172.31.N.0` |
| eth4 (logical, village 1) | `10.0.0.2/24` to the host gateway; village 1 originates the default and NATs |
| eth5-eth8 | Spare: peering with a player AS (probe the ISP to find them) |

**A player gets online by connecting any computer to the village cable**: dig down
one block under a street (or cable from an existing house router's DOWN face),
connect your PC's NIC to it and use `iface ethN dhcp` (the NIC facing the cable) in its
`network.cfg`. It leases a `100.(64+N).1.x` address and reaches every village, the
chat server and, through village 1, the internet gateway.

The ISP routers, web servers and chat server boot headless when the server starts,
before their terrain generates, so BGP converges and the servers can talk immediately.
Generated blocks attach to the same running computers (never a second copy). House
computers boot when their chunks load.

**Fiber ring.** Every site's two panels and fiber endpoints (the blocks above the
panels) are known before the village generates: the mast is the ISP template's centre
column, the start jigsaw `evanscomputermod:mast_anchor` places it on the site's
(x, z), its height comes from the generator's surface estimate exactly as the jigsaw
structure computes it, and each panel hangs on the mast side facing its neighbour.
Between a village's "next" endpoint and the next village's "previous" endpoint a
straight 3D line is rasterised into face-connected blocks; a feature at the last
decoration step writes it in each chunk, replacing whatever is there except bedrock
and network blocks.

The ISP-to-ISP BGP link is a logical link between the two routers' fiber NICs, so it
works before any terrain exists. It stays up only while the physical path is complete:
no admin cut (`/ecm net cut`), every chord block in place, and at both ends the
router's fiber port cabled to its panel. Break a Fiber Span on the chord, or the cable
up the mast, and that BGP edge goes down; the routers lose carrier, drop the session at
once and traffic takes the long way round the ring. Put the block back to repair it.
Villages that never generated count as intact; unloaded ones use the last-known cabling.

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

Only adjacent ring sites can be cut with the command. `links` reports each edge's
state, carrier, admin cuts, missing fiber blocks and both ends' cabling.

Templates contain role markers, not computer UUIDs. The provisioning processor (ISP,
Data Center) and the village network piece (houses) derive stable identities from
world seed, village number and role (`isp.router`, `datacenter.web`,
`datacenter.chat`, `house<k>.router`, `house<k>.pc`), write missing configuration
files (including `/etc/chat.conf`), and set `wasRunning`. Existing configuration files
are kept. Regenerate the five ISP and Data Center templates, pools and structures with
`py -3 scripts/gen-tech-village.py`; generate models/recipes with
`py -3 scripts/gen-tech-assets.py`.

## Add your own player AS

See [joining as your own AS](TECH_VILLAGE_NETWORK.md#b-your-own-as-peering-with-an-isp)
for the exact configuration of both sides. In short: at village 3, probe a spare ISP
port (eth5 to eth8; their faces depend on the building's rotation), cable it to your
router's eth1, give the ISP side `172.30.3.1/30` and `neighbor 172.30.3.2 remote-as
65200` (activated in `address-family ipv4 unicast`), and your side `172.30.3.2/30`,
AS 65200, `neighbor 172.30.3.1 remote-as 65003` and a `network` for a LAN prefix you
really have, such as `10.200.0.0/24`. Your AS learns the village routes and village 1's
default; the ring learns your LAN prefix. An ISP port accepts only neighbors explicitly
configured by its operator; there is no wildcard peer.

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
`ecm_network` SavedData stores sites (style, start height, fiber endpoint), node
port counts, last-known cables/NIC positions, removed fiber positions and cuts. Unloaded cable regions retain
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
| `worldgen/*` and template generators | Ring placement, styled ISP start pieces on vanilla villages, house network piece, provisioning, the fiber feature |
| `FiberLine` / `FiberChords` | Face-connected line rasterisation, panel sides, per-chunk index and cut bookkeeping (plain Java) |
| `CableRouter` / `IspCabling` | The ISP router's cable runs for any rotation, kept apart (plain Java router + template glue) |
| `CableNetworkManager` | Cable segments (copper, panels, player fiber), last-known topology, the fiber link gate |
| `rust/crates/ecm-chat`, `chatd`, `chat` | Chat protocol, room and client logic (host-tested); the server and client programs |
| `InternetProxy` | Ethernet/host TCP and UDP adapter, ARP/DHCP and bounded flow state |

All protocol engines use caller-supplied time and bounded state. Pure tests and
the simulator use virtual time. The kernel owns networking; Java supplies NIC
frames and schedules worker ticks. This avoids blocking the Minecraft server
thread on BGP connections, DHCP retries or host internet sockets.

Targeted verification commands (see `TESTING.md` for receipts):

```powershell
scripts/Test.ps1 -Area router -Rust ecm-net,ecm-router,ecm-bgp,terminal-os
scripts/Test.ps1 -Area router-sim -Scenarios 15_router,16_bgp,17_bgp
scripts/Test.ps1 -Area chat -Rust ecm-chat
scripts/Test.ps1 -Area tech-world -GameTests ecm_router,ecm_router_scenarios -McVersion 1.21.1
scripts/Test.ps1 -Area tech-client -ClientChecks -ClientSuite tech
scripts/Test.ps1 -Area tech-client-flat -ClientChecks -ClientSuite tech -LevelType flat
scripts/Test.ps1 -Area fiber-line -JUnit FiberLineTest -McVersion 26.1
scripts/Test.ps1 -Area proxy -JUnit InternetProxyTest -McVersion 26.1
scripts/Test.ps1 -Area host -JUnit KernelHostIntegrationTest -McVersion 26.1
scripts/Test.ps1 -Area cancellation -Wasmtime -JUnit InterruptIsolationTest -McVersion 26.1
```

The ring simulator cuts one edge, verifies the long route, then isolates the source
and requires zero echo replies. The in-world tests use real kernels and Minecraft
segments. Each in-world case has a wall-clock limit below one minute. A pass
requires executed tests and markers, not just a successful build.
