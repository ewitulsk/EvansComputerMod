# Networking Brainstorm

Goal: Expose the lowest level of networking through host functions, then rebuild the entire networking stack (switching, routing, TCP/IP) in the operating system. Computers should be able to network with each other inside Minecraft AND access the real internet.

---

## Chosen Direction: Ethernet Frame Level (Layer 2)

Expose a virtual NIC per computer. The host is a dumb wire — all intelligence lives in the OS.

### Host Functions (the only networking primitives)

```
net_get_mac(buf_ptr) -> len                          // 6-byte MAC derived from computer UUID
net_tx_frame(buf_ptr, frame_len) -> status           // send an ethernet frame
net_rx_frame(buf_ptr, buf_len) -> frame_len          // receive next frame (-1 if none)
net_rx_frame_blocking(buf_ptr, buf_len, timeout_ms) -> frame_len  // blocking variant
net_set_promiscuous(enabled) -> status               // receive frames not addressed to this MAC
```

Plus `IRQ_NETWORK = 3` interrupt fired when a frame arrives.

That's it. Everything else — ARP, IP, ICMP, UDP, TCP, DNS, DHCP — is implemented in the Rust OS.

### In-Game Architecture

```
Computer A ──┐
Computer B ──┼── Virtual Hub/Switch (host-managed) ── [Gateway Computer] ── TAP ── Internet
Computer C ──┘         (dumb frame delivery)          (runs full IP stack)
```

The host delivers frames between computers on the same network. A designated gateway computer bridges to the real internet via a TAP interface.

---

## OS Networking Stack (built in Rust, runs in WASM)

The OS must implement the full stack from scratch:

| Layer | Protocol | Purpose |
|-------|----------|---------|
| 2 | Ethernet | Frame parsing, MAC addressing |
| 2.5 | ARP | Resolve IP addresses to MACs on the virtual LAN |
| 3 | IP (v4) | Addressing, packet routing, TTL, fragmentation |
| 3 | ICMP | Ping, destination unreachable |
| 4 | UDP | Simple unreliable datagrams |
| 4 | TCP | Reliable streams, 3-way handshake, flow control, retransmission |
| 7 | DNS | Name resolution (forward queries through gateway) |
| 7 | DHCP | Optional — auto-assign IPs on the virtual LAN |

This is the core educational/technical value: users can inspect, modify, and learn every layer.

---

## Internet Access via TAP

A TAP device is a kernel-provided virtual network interface at Layer 2. The host opens `/dev/net/tun`, configures TAP mode, and gets a file descriptor. Frames written to the fd enter the kernel networking stack; frames read from it come from the kernel.

### How It Connects

```
Minecraft Computer (WASM)
    |
    | net_tx_frame() / net_rx_frame()       <- host functions
    |
Host (Java mod or Simulator)
    |
    | write()/read() raw frames             <- OS-level TAP device
    |
TAP Virtual NIC (e.g. tap0)
    |
    | bridged or NAT'd                      <- Linux networking
    |
Physical NIC (eth0 / wlan0)
    |
Internet
```

### Routing Options

| Option | How | Root Required | Visibility |
|--------|-----|---------------|------------|
| **Bridge** | Bridge `tap0` with physical NIC. Computer gets real LAN IP via DHCP. | Yes | Full peer on the network |
| **NAT** | Give `tap0` its own subnet, enable IP forwarding + iptables masquerade. | Yes | Can reach out, not reachable from outside |
| **Slirp (user-space NAT)** | Library parses frames, handles ARP/DHCP/DNS internally, proxies TCP/UDP via regular sockets. | No | Can reach out, not reachable from outside |

For the Minecraft mod, slirp is most practical (no root, cross-platform). For the simulator, TAP with NAT is simplest to prototype.

### In-Game Model

Not every computer needs a TAP device. The intended flow:

1. Computers on the same virtual LAN exchange frames through the host's in-memory hub
2. One computer acts as the **gateway** — it has the TAP bridge to the real internet
3. Other computers configure that gateway as their default route
4. The gateway computer runs NAT (implemented in the OS!) to translate between the virtual LAN and the real internet

This means players build real network topologies: a computer acting as a router, another as a DNS server, etc.

---

## Minecraft-Specific Considerations

### Network Topology in the World

How do computers form networks?

| Option | Description |
|--------|-------------|
| **Cable blocks** | Physical network cable blocks placed in-world. Computers connected by cables share a LAN. Most gameplay-rich. |
| **Proximity** | All computers within N blocks auto-form a LAN. Simple but less interesting. |
| **Network block** | A "switch" or "hub" block — all computers adjacent to it share a LAN. Middle ground. |
| **Global** | All computers everywhere share one LAN. Simplest host implementation, least realistic. |

Cable blocks or a switch block are most interesting for gameplay and align with the "build your own network" vision.

### Security

Real internet access from Minecraft is a security concern:

- Server operators need a config toggle (`enable-internet-access: false` by default)
- Rate limiting on the TAP bridge to prevent abuse
- Optionally restrict to whitelisted IPs/domains
- The slirp approach naturally sandboxes (no raw socket access, just proxied TCP/UDP)

### Addressing

- **MAC**: Derived deterministically from computer UUID (e.g., first 6 bytes of UUID). Unique per computer.
- **IP**: Assigned by the OS. Could be static config or DHCP from a computer running a DHCP server.
- **Hostname**: Optional, resolved by a computer running DNS on the LAN.

---

## Simulator Implementation Plan

The simulator is the best place to prototype since it's native Rust.

### Phase 1: In-Process Multi-Computer

Run multiple WASM instances in one simulator process, connected by an in-memory hub:

```
[WASM Instance A] <--frame queue--> [Hub] <--frame queue--> [WASM Instance B]
```

This lets us develop and test the OS networking stack without any real networking.

### Phase 2: TAP Bridge

Add `--tap` flag to the simulator. One instance gets a TAP device bridged to the host OS:

```bash
# Terminal 1: gateway computer with TAP
cargo run --release -- --wasm ../wasm-bin/terminal_os.wasm --tap

# Terminal 2: regular computer, connects via localhost UDP to the hub
cargo run --release -- --wasm ../wasm-bin/terminal_os.wasm --hub localhost:9000
```

### Phase 3: Inter-Process Networking

Multiple simulator instances exchange frames over localhost UDP (or Unix domain sockets), simulating separate computers on a LAN without needing Minecraft.

