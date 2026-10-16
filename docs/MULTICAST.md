# IP multicast

Computers can send one UDP datagram to a **group** (an address in 224.0.0.0/4), and every
computer on the link that joined the group receives it. Programs use the same socket options
as on Linux. Service discovery ([DISCOVERY.md](DISCOVERY.md)) is built on it.

The stack is `rust/crates/ecm-net` (`stack.rs` and the IGMP messages in `igmp.rs`); the
kernel's socket layer (`operating-system/rust/src/net/ipc.rs`) maps the options onto it. The
hardware needed nothing new: NICs already pass group-addressed frames to the stack, which now
filters them by the groups each interface joined.

## For programs

| Option (level `IPPROTO_IP` = 0) | Value | `ecm_host_abi::socket` helper |
|---|---|---|
| `IP_ADD_MEMBERSHIP` (35) | `ip_mreq`: group, interface address (`0.0.0.0`: the first usable interface) | `join_group(fd, group, iface)` |
| `IP_DROP_MEMBERSHIP` (36) | the same | `leave_group(fd, group, iface)` |
| `IP_MULTICAST_IF` (32) | interface address to send from (`0.0.0.0`: default) | `set_multicast_if(fd, iface)` |
| `IP_MULTICAST_TTL` (33) | int or byte, default **1** (this link only) | `set_multicast_ttl(fd, ttl)` |
| `IP_MULTICAST_LOOP` (34) | int, default **on**: this computer's own members get a copy | `set_multicast_loop(fd, on)` |
| `SO_REUSEADDR` (level `SOL_SOCKET`) | int; set before `bind` | `set_reuse_addr(fd, on)` |

* Receiving: bind the port (usually `0.0.0.0:PORT`), then join. Every socket bound to that
  port (on `0.0.0.0` or the group's address) gets a copy of the group's datagrams while this
  computer is a member on the interface they arrived on (Linux's default behaviour).
* Sockets that all set `SO_REUSEADDR` share a UDP port; each gets every multicast and
  broadcast datagram (unicast goes to one of them).
* Sending: `sendto` the group. It leaves by the multicast interface (the bound address's, or
  the first usable one), from that interface's address, with the multicast TTL, to the group's
  MAC (`01:00:5e` and the group's low 23 bits). No ARP.
* A socket joins at most 20 groups. Joining a group twice on one interface is refused.

## IGMPv2

The first member on an interface sends a **membership report** (to the group, TTL 1); the
last one to leave (or close) sends a **leave** to 224.0.0.2. A membership query (to 224.0.0.1)
is answered with a report for every joined group (or the one asked about). Every interface is
always a member of 224.0.0.1, which is never reported.

## Routing

Routers don't forward multicast (there is no PIM); 224.0.0.0/24 is link-local by definition.
Discovery crosses routers with relays instead. Switches flood group frames like broadcast
(no IGMP snooping).

## Seeing it

```text
/ > ip maddr
1:	eth0
	link  01:00:5e:00:00:01
	inet  224.0.0.1
	link  01:00:5e:00:00:fe
	inet  224.0.0.254
/ > tcpdump igmp
12:00:01.200 IP 10.0.8.1 > 224.0.0.254: igmp v2 report 224.0.0.254
/ > tcpdump multicast and udp
```

`tcpdump` decodes IGMP (queries, v1/v2 reports, leaves) and filters with `igmp` and
`multicast` (any datagram to 224.0.0.0/4).

## Tests

| What | Where | Run |
|---|---|---|
| Members receive and others don't (the MAC filter), leave, TTL 1 and the group MAC; SO_REUSEADDR sharing with a copy each; loopback to own members and IP_MULTICAST_LOOP off; the multicast interface and TTL; IGMP reports, no second report, leaves when the last member closes, query answers | `rust/crates/ecm-net/tests/multicast.rs`, `src/igmp.rs` | `-Rust ecm-net` |
| The socket options through the kernel's socket layer: join, shared port, interface/TTL/loop, bad values refused, drop | `operating-system/rust/src/net/ipc_tests.rs` (`multicast_options_join_a_group_and_share_the_port`) | `-Rust terminal-os` |
| tcpdump's `igmp` / `multicast` filters and IGMP text | `rust/wasm-programs/tcpdump` | `-Rust tcpdump` |
| In game: `ip maddr` shows the responder's group; discovery over multicast | `service_discovery` | `-GameTests ecm_apps -GameTestOnly service_discovery` |
