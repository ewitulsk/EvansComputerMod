# Event-driven kernel refactor — architecture & interface contract

This is the binding contract for the `refactor/event-driven-kernel` work.
Background and the bugs being fixed: `docs/kernel-review-2026-09.md`.
If you need to deviate from an interface here, write the deviation and the
reason in the "Deviations" section at the bottom of this file.

## 0. Global rules (all Rust crates touched by the refactor)

- **No `static mut`.** No `unsafe` except inside the kernel's `hal` module and
  the single `KernelCell`.
- **Pure crates** (`ecm-net`, `ecm-bridge`) have no `extern "C"`, no global
  state, no sleeping and no clock reads. Time is always an argument
  (`now_ms: i64`, milliseconds, monotonic, may start at any value).
  Randomness is a seed passed at construction.
- **Pure crates build and test on the host** (`cargo test -p ecm-net`,
  `cargo test -p ecm-bridge`, native target) *and* on `wasm32-unknown-unknown`.
  `std` is allowed; the kernel is `std` on wasm32-unknown-unknown.
- **No panics on any external input.** That covers frames from the wire,
  syscall args from programs, and CLI text. No unchecked slicing or indexing
  in parsers; return `Option`/`Result`. Every parser gets a unit test with
  truncated and garbage input.
- **Every table is bounded** (FDB, ARP, neighbours, LLDP neighbours, sockets,
  queues) with a documented drop policy.
- **Outputs are queued, not performed.** Pure crates push to an internal
  output queue that the caller drains.

## 1. Runtime model

```
Host (Java worker thread / simulator) ──calls──▶ kernel exports (never re-entered)
  main()                         boot
  on_input(ptr,len)              keyboard bytes
  on_interrupt(irq,ptr,len)      IRQ payload bytes (binary); IRQ_NETWORK = frames ready
  on_tick(now_ms) -> i64         run timers/jobs; returns next absolute deadline ms, or -1
  handle_sock_ipc(session, syscall, args_ptr, args_len, res_ptr, res_len) -> i32
                                 >=0 result len, <0 error, IPC_PENDING (-11) = not ready, retry
  abi_layout(ptr) -> i32         writes the kernel's buffer addresses (see §4)
```

- The kernel **never blocks** and never calls a host function that calls back
  into it. Kernel imports are all non-blocking; the kernel no longer imports
  `sleep_ms`, `terminal_read_line` or `process_wait`.
- The host calls `on_tick` when the last returned deadline has passed, after
  any `on_interrupt(IRQ_NETWORK)`, and whenever a child-process event wakes
  the worker. Calling it early is always safe.
- The host must not call a kernel export while another kernel export is
  running on the stack. The kernel enforces this (`KernelCell` borrow flag
  → trap).

## 2. `ecm-net` (crate `crates/ecm-net`): sans-IO host stack

```rust
pub struct Stack;                         // owns everything; no globals
pub struct StackConfig { pub seed: u64 }
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SocketHandle { idx: u16, gen: u16 }   // generation-checked; stale => Err(BadHandle)

impl Stack {
    pub fn new(cfg: StackConfig) -> Self;

    // ---- interfaces (max 16) ----
    pub fn add_interface(&mut self, name: &str, mac: MacAddr) -> Option<usize>;
    pub fn remove_interface(&mut self, idx: usize);        // closes sockets bound to its IP
    pub fn iface_count(&self) -> usize;                    // highest idx+1 (slots may be empty)
    pub fn iface(&self, idx: usize) -> Option<&Interface>; // name, mac, ip, prefix, link_up, admin_up, vlan, stats
    pub fn find_iface(&self, name: &str) -> Option<usize>;
    pub fn set_link(&mut self, idx: usize, up: bool, now_ms: i64);      // carrier
    pub fn set_admin_up(&mut self, idx: usize, up: bool, now_ms: i64);  // ifconfig up/down
    pub fn set_vlan(&mut self, idx: usize, vid: Option<u16>);           // host 802.1Q tagging
    pub fn configure_addr(&mut self, idx: usize, ip: Ipv4Addr, prefix: u8, now_ms: i64);
        // replaces the old connected route, flushes ARP on that iface,
        // sends gratuitous ARP
    pub fn clear_addr(&mut self, idx: usize);
    pub fn routes(&self) -> &[Route];
    pub fn add_route(&mut self, dst: Ipv4Addr, prefix: u8, gw: Ipv4Addr, iface: usize) -> Result<(), NetError>; // validates iface
    pub fn del_route(&mut self, dst: Ipv4Addr, prefix: u8) -> Result<(), NetError>;
    pub fn dns_server(&self) -> Ipv4Addr;  pub fn set_dns_server(&mut self, ip: Ipv4Addr);
    pub fn neighbors(&self, idx: usize) -> impl Iterator<Item = (Ipv4Addr, MacAddr, NeighborState)>;

    // ---- I/O ----
    pub fn handle_frame(&mut self, iface: usize, frame: &[u8], now_ms: i64); // never panics
    pub fn poll(&mut self, now_ms: i64) -> Option<i64>;  // run timers; returns next deadline
    pub fn pop_tx(&mut self) -> Option<(usize, Vec<u8>)>; // (iface, full ethernet frame)

    // ---- TCP (non-blocking) ----
    pub fn tcp_listen(&mut self, local: SocketAddr, backlog: usize) -> Result<SocketHandle, NetError>;
    pub fn tcp_accept(&mut self, listener: SocketHandle) -> Result<Option<SocketHandle>, NetError>;
    pub fn tcp_connect(&mut self, remote: SocketAddr, now_ms: i64) -> Result<SocketHandle, NetError>;
    pub fn tcp_state(&self, h: SocketHandle) -> Result<TcpState, NetError>;
    pub fn tcp_send(&mut self, h: SocketHandle, data: &[u8], now_ms: i64) -> Result<usize, NetError>; // WouldBlock if full
    pub fn tcp_recv(&mut self, h: SocketHandle, buf: &mut [u8]) -> Result<usize, NetError>; // Ok(0)=EOF, WouldBlock
    pub fn tcp_shutdown_write(&mut self, h: SocketHandle, now_ms: i64) -> Result<(), NetError>; // FIN after TX drains
    pub fn tcp_close(&mut self, h: SocketHandle, now_ms: i64);  // graceful; handle invalid afterwards
    pub fn tcp_abort(&mut self, h: SocketHandle);               // RST
    pub fn tcp_local_addr / tcp_peer_addr(&self, h) -> Result<SocketAddr, NetError>;
    pub fn tcp_can_read / tcp_can_write(&self, h) -> bool;

    // ---- UDP ----
    pub fn udp_bind(&mut self, local: SocketAddr) -> Result<SocketHandle, NetError>; // port 0 => ephemeral
    pub fn udp_send_to(&mut self, h, dst: SocketAddr, data: &[u8], now_ms: i64) -> Result<usize, NetError>;
        // queues behind ARP resolution; MessageTooLong > 1472
    pub fn udp_recv_from(&mut self, h, buf: &mut [u8]) -> Result<Option<(SocketAddr, usize)>, NetError>;
    pub fn udp_close(&mut self, h);

    // ---- raw ICMP (one queue PER socket) ----
    pub fn icmp_open(&mut self) -> Result<SocketHandle, NetError>;
    pub fn icmp_send(&mut self, h, dst: Ipv4Addr, icmp_packet: &[u8], now_ms: i64) -> Result<usize, NetError>;
    pub fn icmp_recv(&mut self, h, buf: &mut [u8]) -> Result<Option<(Ipv4Addr, usize)>, NetError>;
    pub fn icmp_close(&mut self, h);

    // ---- DNS (non-blocking query) ----
    pub fn dns_query(&mut self, name: &str, now_ms: i64) -> Result<DnsHandle, NetError>;
    pub fn dns_poll(&mut self, q: DnsHandle) -> DnsStatus; // Pending | Resolved(Ipv4Addr) | Failed(NetError); frees on non-Pending
}
```

Required behaviour:
- **ARP:** a per-neighbour pending queue (≤4 packets per neighbour, ≤32
  neighbours). Retry 3× at 1 s, then drop and report `HostUnreachable` to
  sockets. Learn only from replies and from requests targeting our IP.
  Entries expire after 60 s (stale → re-probe).
- **Loopback:** traffic to any of our own IPs (or 127/8) is delivered locally
  without touching an interface.
- **Source address** = the socket's bound local IP; egress is chosen by route.
- **TCP:**
  - Retransmission queue from `snd_una` with RTO/backoff, rewinding
    `snd_nxt`.
  - Honour `snd_wnd` and send window updates.
  - ACK duplicate and out-of-window segments.
  - Validate the FIN sequence, and send FIN only after TX drains.
  - Timeouts: TIME_WAIT 2×MSL (MSL = 15 s), FIN_WAIT_2 60 s, CLOSING/LAST_ACK
    retransmit, and an abort after 10 retries.
  - Validate SYN-ACK / ACK numbers and the RST sequence window.
  - Verify the RX checksum.
  - MSS option (parse and send).
  - ISN from a seeded PRNG plus time.
  - ≥32 connections.
- **IPv4:**
  - Validate version/IHL/`total_length ≥ ihl*4`/checksum.
  - Drop fragments (MF or offset ≠ 0) with a counter.
  - Egress MTU 1500.
- **UDP:** validate `8 ≤ length ≤ available`, verify the checksum if nonzero,
  and give each socket a bounded datagram queue (drop newest when full).
- **DNS:**
  - Random transaction ID.
  - Source IP/port check.
  - Compression-pointer hop limit (16).
  - Timeout 3 s × 2 tries.
- **802.1Q host tagging per interface:** tag egress; accept only frames with a
  matching tag. When `vlan` is `None`, accept untagged frames and priority-tagged
  frames (VID 0).
- **Stats** per interface: rx/tx packets, bytes, errors, drops.
- **Tests:**
  - Unit tests for every parser.
  - An in-process two-stack harness (`tests/`) that wires two `Stack`s via
    `pop_tx`/`handle_frame` with a virtual clock. It must cover: ARP; ping;
    UDP; DNS against a fake server; a 1 MiB TCP transfer with 5% loss and
    reordering, byte-exact; close with pending TX; 40 concurrent connects;
    RST handling.
  - Fuzz-style tests: 100k random or mutated frames and no panic.

## 3. `ecm-bridge` (new crate `crates/ecm-bridge`): pure L2 switch

```rust
pub struct Bridge;
pub enum Output {
    Tx { port: usize, frame: Vec<u8> },         // to a physical port
    Local { vlan: u16, frame: Vec<u8> },         // untagged frame for the switch's own SVI on `vlan`
    Log { severity: Severity, msg: String },
}
impl Bridge {
    pub fn new(port_macs: &[MacAddr], bridge_mac: MacAddr, now_ms: i64) -> Self;
    pub fn handle_frame(&mut self, port: usize, frame: &[u8], now_ms: i64); // from a physical port; never panics
    pub fn send_local(&mut self, vlan: u16, frame: &[u8], now_ms: i64);     // from the switch's own stack via SVI
    pub fn set_link(&mut self, port: usize, up: bool, now_ms: i64);
    pub fn poll(&mut self, now_ms: i64) -> Option<i64>;
    pub fn pop_output(&mut self) -> Option<Output>;
    pub fn is_l2_port(&self, port: usize) -> bool; // false => routed port, the kernel sends it to the host stack directly
    pub fn bridge_mac(&self) -> MacAddr;
    // typed config + introspection API used by `cli` and tests (design as needed)
}
pub mod cli {
    pub struct CliSession;                 // current context (config / config-vlan-N / config-if-ethN / config-lag-N / config-if-vlan-N)
    pub enum CliEffect {
        SviAddress { vlan: u16, ip: Ipv4Addr, prefix: u8 },
        SviRemove { vlan: u16 },
        SaveConfig(String),                // `write memory` → kernel writes /switch.cfg
        ExitCli,
    }
    pub struct CliResult { pub output: String, pub effects: Vec<CliEffect> }
    pub fn exec(bridge: &mut Bridge, s: &mut CliSession, line: &str, now_ms: i64) -> CliResult;
    pub fn prompt(s: &CliSession) -> String;
    pub fn running_config(bridge: &Bridge, svis: &[(u16, Ipv4Addr, u8)]) -> String; // replayable through exec
}
```

Required behaviour:
- **Hierarchy:** physical port → (optional) LAG aggregator → bridge port. STP,
  VLAN membership, FDB learning and flooding all operate on **bridge ports**.
  A LAG's members inherit the LAG's VLAN config.
- **Forwarding gate** per bridge port: `link_up && lacp_distributing (if LAG)
  && stp_state ∈ {Forwarding}`. Learning is allowed in {Learning, Forwarding}.
  BPDUs, LACPDUs and LLDPDUs (reserved `01:80:c2:00:00:0x`) are never
  forwarded.
- **FDB:** keyed by (MAC, VLAN). Bounded (1024). Ageing (default 300 s,
  configurable). Static entries. MAC-move tracking. Flush on STP topology
  change (fast ageing = forward delay).
- **VLANs:** access/trunk, native tagged/untagged, allowed lists, ingress
  filtering (same semantics and AOS-CX CLI as the current `switch.rs`).
  Routed ports (the default) aren't bridge members.
- **RSTP (802.1D-2004 cl. 17) on the CIST:**
  - Priority vectors with port-ID tie-breaks.
  - Roles: Root, Designated, Alternate, Backup, Disabled.
  - States: Discarding, Learning, Forwarding.
  - Proposal/agreement is optional; the timer-based fallback (2×forward-delay)
    is acceptable.
  - Message age increments; the info expires at max-age (or 3×hello with no
    BPDU).
  - TC handling: set the TC flag and flush the FDB on other ports.
  - Admin/oper edge.
  - Features: BPDU guard (err-disable until `no shutdown`/link flap), root
    guard (root-inconsistent = discarding while superior BPDUs arrive), and
    per-port disable (the port forwards without STP).
  - Correct BPDU encoding (RST BPDU v2 and config BPDU v0 accepted).
- **LACP (802.1AX):**
  - Active/passive, fast (1 s tx, 3 s timeout) and slow (30 s / 90 s).
  - Selection by (partner system, partner key); members to different
    partners must not aggregate.
  - SYNC/collecting/distributing handshake.
  - Static LAG mode.
  - Hash modes l2-src-dst / l3-src-dst / l4-src-dst, parsing tags, ARP and
    IPv4 properly.
  - Fallback to a single member.
  - LACPDU exactly 110 bytes of payload, padded to a 128-byte frame
    (60-byte minimum respected).
- **LLDP (802.1AB):**
  - TX interval / hold / reinit / tx-delay.
  - TTL 0 deletes the neighbour.
  - Neighbour table ≤ 64 per port (drop new when full).
  - Purge on link-down.
  - Only consume frames to the nearest-bridge address.
- **Logging:** a severity-filtered ring buffer (256 entries).
- **CLI:** port the full command set and show-output style of the current
  `switch.rs`/`switch_*.rs` (see `switch_instructions.md`). Add
  `interface vlan <N>` + `ip address a.b.c.d/nn` (SVI) → `CliEffect`. Keep
  `write memory` and the `switch.cfg` replay format.
- **Tests:** unit tests for each protocol and CLI parse, plus an in-process
  multi-bridge `netsim` harness (`tests/`, virtual clock, links as queues,
  optional drop) covering:
  - learning/flooding/aging
  - VLAN isolation
  - 2- and 3-bridge rings converging to exactly one discarding port with a
    bounded frame count
  - root failure re-election
  - BPDU guard
  - LAG between two bridges forming and failing over
  - a LAG to two different peers not bundling
  - partner timeout (fast 3 s)
  - LLDP neighbour discovery and TTL-0 delete
  - fuzzed frames (no panic)

## 4. Kernel (`operating-system/rust`)

- `hal.rs` is the **only** module with `extern "C"`. It exposes safe wrappers.
  Kernel imports (final list; also recorded in `abi/host-abi.toml`):
  - Time and randomness: `get_time_ms`, `__getrandom_v03_custom`.
  - Frames and links: `net_get_interface_count`, `net_get_interface_mac`,
    `net_tx_frame_on`, `net_rx_frame_any`, `net_set_promiscuous_on`,
    `net_set_link_state`, `net_get_link_state` (new: carrier), `net_pcap_*`
    (kept for tcpdump plumbing if used).
  - Processes: `process_spawn`, `process_try_wait(pid, code_ptr) -> i32`
    (new), `process_read_output(pid, buf, len) -> i32` (new),
    `process_write_input(pid, buf, len) -> i32` (new), `process_kill`,
    `process_list`, `process_state`.
  - Files: the file-system imports (unchanged), `fd_*`/`pipe_create`
    (unchanged).
  - Other subsystems: redstone, screen, visual editor, module_* (unchanged).
  - `fb_sync`.
- A single `static KERNEL: KernelCell<Kernel>` (borrow flag; re-entry → trap).
  `Kernel { shell, jobs, net: NetDispatcher { stack, bridge: Option<Bridge>, svis }, ipc, gfx_job, ... }`.
- `NetDispatcher` is the only RX consumer. RX on port p:
  - L2 bridge port → `bridge.handle_frame`;
  - otherwise → `stack.handle_frame(p)`.

  Bridge `Output::Local{vlan}` → the stack's SVI iface for that VLAN.
  Stack TX on an SVI iface → `bridge.send_local`. Stack TX on a physical
  iface → HAL, but only if that port is routed or the bridge is off.
- The switch is a kernel service, independent of shell state (Ctrl+T doesn't
  stop it). `switch on/off` starts or stops it; `switch` enters the CLI.
  Config persists in `/switch.cfg`.
- **Shell jobs:** running a program spawns it and sets the foreground job,
  then returns. `on_tick` pumps child output into the VTE, forwards keyboard
  input to the foreground child's stdin, and prints the exit status and the
  prompt when the job ends. Ctrl+T kills the foreground job.
- **Socket IPC:** handlers never block. They return `IPC_PENDING` when not
  ready; the host retries after the next tick or RX. Timeouts are kept per
  socket in the kernel (SO_RCVTIMEO; connect 10 s; DNS 6 s); a recv with no
  timeout set waits indefinitely.
- **Memory ABI:** `abi_layout(ptr)` writes little-endian u32s:
  `[input_buf, input_cap, irq_buf, irq_cap, ipc_args, ipc_args_cap, ipc_result, ipc_result_cap, framebuffer]`.
  The buffers are kernel statics (the linker places them). The framebuffer
  keeps its header format. Hosts must use these addresses.
- Randomness: getrandom 0.2's custom backend calls the host
  `__getrandom_v03_custom`.
- Removed: the kernel `ssh/` client/server (dead code), `crypto.rs`,
  SSH-output shell plumbing, and the `add` export. `gfxtest` becomes a
  tick-driven job. `git`'s interactive message prompt is replaced by
  requiring `-m`.

## 5. Hosts

- **Java** (`ComputerInstance`):
  - Implements the ABI above: the worker loop is driven by
    `on_tick` deadlines, and IPC requests are retried while `IPC_PENDING`.
  - Registers `get_time_ms` and the new process imports.
  - Strict linking: a missing kernel import is a load error (an allow-list
    covers legacy optional imports).
  - `NetworkHub`: wire semantics per segment (deliver to every NIC on the
    segment, the NIC filters on its own MAC, broadcast, multicast and
    promiscuous), link-down cuts delivery, and `net_rx_frame_any` is
    round-robin.
- **Simulator:** the same ABI and the same hub semantics, plus a topology
  file, virtual clock, scenario runner, pcap and fault injection
  (`docs/kernel-review-2026-09.md` §6).

## Deviations

- **`ecm-net` is an in-house sans-IO rewrite, not smoltcp.** The kernel needs
  multi-interface routing with per-interface VLAN tagging, raw per-socket ICMP,
  SVIs added and removed at runtime, and full introspection for
  netlink/`ifconfig`/`ip`. Mapping those onto smoltcp's single-device
  `Interface` would cost as much as a correct rewrite, and would still leave
  the ARP/route/stat introspection on the outside. The rewrite is held to the
  test bar in §2 instead.
