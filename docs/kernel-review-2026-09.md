# Kernel, Switch & Simulator Review — `staging` @ d6952d1 (2026-09-28)

Scope: `rust/operating-system/rust` (kernel), `rust/crates/ecm-net` + `ecm-host-abi`
(network stack / ABI), the switch (`switch*.rs`), the Java host
(`ComputerInstance`, `NetworkHub`, `CableNetworkManager`, `wasi/*`), and
`rust/simulator`.

Verdict: **a structural refactor is warranted.** The individual bugs are
fixable, but most of them come from four design decisions that make
correctness impossible to test and easy to break:

1. **Blocking host calls + re-entry.** The kernel blocks inside host calls
   (`process_wait`, `sleep_ms`, `terminal_read_line`, every `tcp_*`/ARP wait
   loop). While it is blocked, the host calls back *into* the kernel
   (`on_interrupt`, `terminal_print`, `handle_sock_ipc`), and that code takes
   fresh `&mut` references to the same `static mut` globals the blocked frame
   still holds.
2. **No clock, no timer.** Nothing drives the kernel periodically. Timers run
   only as a side effect of blocking loops or of frame arrival. On top of
   that, in-game the kernel's `get_time_ms` isn't registered at all and
   returns 0.
3. **No single owner of the RX queue.** About 15 call sites call `poll_rx()`
   directly. The switch "owns" RX only by swapping the IRQ handler, so any
   blocking loop steals its frames.
4. **Protocol logic, I/O, CLI and globals are fused together.** None of the
   network or switch code can run on the host, so there are zero tests for
   it. Every bug below would have been caught by a basic unit test.

---

## 1. Critical findings (verified)

| # | Finding | Where | Impact |
|---|---|---|---|
| C1 | Kernel `get_time_ms` isn't registered by Java. It falls through to `StubHandler`, which returns 0. The fix exists only on `ewitulsk/switch-os` (`12aa35e`) and was lost in the runtime-agnostic rewrite. | `ComputerInstance.createHostFunctions`, `WasmtimeRuntime.java:~86`, `ChicoryRuntime` | In-game `now_ms == 0` forever. MAC aging, ARP expiry, TCP RTO, STP/LLDP/LACP timers are all dead, and deadline loops (`tcp_recv`, `tcp_accept`, `udp recvfrom`) never time out. |
| C2 | STP and LAG never touch the data plane. `switch_stp::port_forwards` and `switch_lacp::select_member` have **no callers**, so `handle_frame`/`flood_vlan` forward on every link-up port. | `switch.rs:394-546` | `spanning-tree` prevents no loops. A LAG *is* a loop (flood out member A, returned on member B). |
| C3 | Switch-mode clock is frozen even when the host clock works. `switch_irq_handler` replaces `poll_rx` and never refreshes `stack.now_ms`, and nothing calls `poll_timers` when idle. | `switch.rs:553-585`, `ecm-net/lib.rs:238,513` | Aging and protocol timers only move when an unrelated blocking loop runs. |
| C4 | RX stealing. `poll_rx()` is called from TCP/ARP/UDP wait loops, `net_ipc_handler` (every child socket call), the ssh client and `shell.rs`. | see `grep poll_rx` | With `switch on` running, `ping`/`curl`/`ssh` on the switch box eat frames that should have been forwarded. |
| C5 | Re-entrancy UB. `process_wait` (Java) → `drainAndDeliverInterrupts` → `on_interrupt` → e.g. `reset_to_shell` takes `&mut LOCAL_SHELL` while the outer `on_input` frame holds one. The same applies to `NetStack::get() -> &'static mut`, `SWITCH_STATE`, and `switch_log::log` (fresh `&mut SWITCH_STATE` while callers hold `&mut SwitchState`). | `lib.rs`, `switch.rs:59-66`, `switch_log.rs:123`, `ComputerInstance.java:1291-1330` | Undefined behaviour. The optimiser may reorder or cache across these. It works "by luck" today. |
| C6 | Remote kernel panics. IPv4 doesn't check `total_length >= header_len` (`ipv4.rs:45-70`); UDP doesn't check `length >= 8` (`udp.rs:26-37`). There are also unchecked slices in `net_ipc_handler` (`addr_len`, short args), an unvalidated netlink `RTA_OIF` (then `interfaces[idx]` out of bounds), and oversized UDP sendto into the 1500-byte `PAYLOAD_BUF`. | ecm-net, `net_ipc_handler.rs`, `netlink.rs` | Any neighbour on the cable (or any local program) can crash a computer with one frame or one syscall. |
| C7 | DNS compression-pointer loop, and netlink `nlmsg_len == 0` loops. | `dns.rs skip_name`, `ecm-host-abi/net_config.rs` | A malicious reply hangs the kernel forever. |
| C8 | TCP retransmit corrupts the stream. On RTO `tx_sent = 0`, but `snd_nxt` is never rewound to `snd_una`, so old bytes are resent at new sequence numbers. | `tcp.rs:690-700`, `lib.rs:536-554` | First lost segment corrupts the connection. |
| C9 | LACP can't form between two of our switches. The TX LACPDU is ~78 bytes (no reserved tail), but RX requires ≥124. Nothing pads. | `switch_lacp.rs:483-549` | LACP only "works" in fallback. |
| C10 | Predictable crypto. The kernel's `getrandom` 0.2 backend is a fixed-seed xorshift, even though the host provides `__getrandom_v03_custom`. | `lib.rs:35-50`, `crypto.rs:21`, `ssh/kex.rs:25` | SSH ephemeral keys, cookies and padding are identical on every computer on every boot. |
| C11 | Simulator can't run children. `exit_code` is dropped (`let _ = exit_code; // TODO`), so `process_wait` spins forever after any child exits (including `ifconfig`). | `simulator/src/process.rs:196` | The sim can't run any networking scenario at all. |

## 2. High-severity findings

**Kernel / OS**
- **Ctrl+T kills a detached switch.** `reset_to_shell` → `switch::cleanup_on_reset` tears down unconditionally, even after `switch on`.
- **One child blocks the whole kernel.** A single child socket call blocks the worker for up to 5 s (recv/recvfrom/getaddrinfo) or 11.5 s (connect) *inside* `handle_sock_ipc`. That freezes input, output, interrupts and every other child's I/O. It's serviced ≤4 at a time (`MAX_PER_CALL`), so the worst case is 20 s+.
- **Interrupt payloads are forced into `&str`.** They go through `from_utf8_unchecked` (`lib.rs:215,541`), but IRQ_MOUSE is binary. This is UB the moment a handler inspects it.
- **Host buffers live inside the stack.** Memory layout: the kernel is linked stack-first (stack 0x100000 ↓ 0), and the host writes fixed buffers at 0x10000 (input), 0x11000 (IRQ), 0x13000/0x14000 (IPC) and 0x20000 (framebuffer), all *inside the stack region*. There's no guard, so deep stacks silently corrupt host buffers. The heap and screen carve-out at 0x300000 happen to be safe only because dlmalloc grows past the initial 4 MiB.
- **Missing imports are silent.** Both hosts turn missing imports into zero-returning stubs. That's how C1 shipped unnoticed.
- **Dead or confusing code.** `run_sshd` in the kernel has no caller (sshd is a WASI program now), and there's the `add()` export plus unused `sock_*` externs.

**Switch (L2)**
- **Management traffic ignores VLANs and the bridge.** Frames for the switch are handed to `process_frame_on(ingress)` untagged, and replies leave through the local stack straight out of that interface. They're untagged even on a tagged trunk VLAN, and the switch can only reach hosts on the ingress segment. There's no SVI/bridge-interface model.
- **Unfair RX.** `net_rx_frame_any` (Java) always scans iface 0 first, so a busy port 0 starves the others (up to 256 frames/IRQ).
- **STP:**
  - Message age is always 0 and received timers are never parsed, so a dead root never ages out (count-to-infinity).
  - Root ports send BPDUs.
  - Flag encoding is wrong (Root sent as 0x24; Designated always claims Forwarding; TC never set).
  - `port-priority` is ignored and there's no port-ID tie-break.
  - There's no Listening state.
  - BPDU guard re-enables on the next tick, and root guard is undone by the promotion timer.
  - Received info always overwrites stored info.
  - There's no TC/FDB flush.
- **LACP:**
  - Partner timeout is re-selected in the same tick.
  - The timeout is always 90 s (fast rate ignored).
  - Keys must equal on both sides.
  - There's no same-partner check, no two-way SYNC handshake, and no `link_up` check.
  - Fallback selects all members.
  - The L3/L4 hash reads the wrong offsets for tagged/ARP/IPv6 frames.
  - A port can join two LAGs.
- **LLDP:**
  - TTL 0 is treated as an error rather than a delete.
  - The neighbour table is unbounded (memory DoS).
  - `reinit`/`txdelay` are unused.
  - The ifIndex is 0-based.
  - Every 0x88cc frame is consumed regardless of destination.
- **Spec vs code.** `switch_instructions.md` claims LAG hashing, LAG-level VLAN config, 2×FD transitions, TC handling and 3 s fast LACP timeout, none of which exist.

**Network stack (ecm-net)**
- **TCP:**
  - Duplicate or out-of-order segments are never ACKed.
  - FIN is accepted without a sequence check.
  - Unsent data is dropped on close (one MSS per flush, FIN sent immediately).
  - Handshake ACK numbers and RST sequence numbers are unchecked.
  - FinWait2 and Closing have no timeout.
  - Connections that reach CloseWait before `accept` leak.
  - `snd_wnd` is ignored.
  - There's no checksum verification on RX.
  - ISN = `now_ms * k` (and `now_ms` is 0 in-game).
  - There are only 8 slots, shared by everything.
- **Shared scratch buffer.** The global `PAYLOAD_BUF`/`TX_BUF` get overwritten by nested `poll_rx` during ARP retry (the retry sends a corrupted payload).
- **Wrong source IP.** Source-address selection uses the egress interface's IP, not the connection's local IP, which breaks multi-homed TCP checksums and 4-tuples.
- **Netlink address changes bypass `configure_iface`.** They leave stale connected routes and never flush ARP.
- **Loose ARP learning.** ARP learns from every packet, which allows poisoning.
- **Fragments treated as whole packets.**
- **Other:**
  - The UDP ring overwrites unread datagrams on wrap.
  - The DNS socket leaks on error.
  - Ping matches on id only.
  - The raw ICMP reply queue is shared across sessions.
  - Userland DNS hard-codes 8.8.8.8 with txid 1.
- **Unsafe socket handles:**
  - Raw `conn_idx` handles with no generation counter, so a stale fd can close someone else's connection.
  - `shutdown` frees the fd.
  - A 5 s idle recv returns 0, which reads as EOF.
  - A 4-byte payload with a negative LE value reads as EOF (`SocketFd.java`).
  - The Java connect timeout (10 s) is shorter than the kernel's (11.5 s).

**Java host (network model)**
- **Link-down doesn't cut delivery.** `net_set_link_state` only flips a visual mask; `CableNetworkManager` ignores it.
- **Not wire semantics.** Unicast goes to the destination MAC's mailbox and multicast only to promiscuous NICs. A real segment delivers every frame to every NIC and lets the NIC filter. Today LLDP/STP work only because switch ports are promiscuous.
- **Per-frame cost.** Every unicast iterates *all NICs in the world* for promiscuous delivery.

## 3. Simulator fidelity (why it can't test the switch today)

| Area | Java host | Simulator |
|---|---|---|
| Topology | One segment per cable mesh; each computer face is its own segment | **One global MAC hub**; every NIC hears every broadcast, and unicast goes straight to the destination (the switch is bypassed) |
| Kernel clock | 0 (bug C1) | Wall clock |
| IRQ_NETWORK | Coalesced to one pending | One per frame, unbounded |
| Interrupts during `process_wait` | Delivered | **Not delivered** (a detached switch stops forwarding) |
| stdin to child during `process_wait` | Yes | No |
| Ctrl+T / epoch interrupt | Yes | No |
| `poll_oneoff` (child sleep) | Implemented | Returns ENOTSUP |
| `gfx_*`/`screen_*`/`mouse_*` for children | Implemented | Missing (child fails to instantiate) |
| Child exit | Works | **Hangs forever** (C11) |
| Link up/down | Visual only (bug) | No-op |
| Interfaces/computer | 5 usable faces | 6 by default |
| Test scripts | — | Wrong paths (`rust/wasm-bin`, `$PROJECT/simulator`); `grep` matches echoed input; fixed sleeps; 1 instance; CLI-only |

---

## 4. Target architecture

```
                 ┌───────────────────────── kernel (wasm) ─────────────────────────┐
 host exports →  │ on_input · on_frame_ready · on_tick(now)->next_deadline         │
                 │ on_child_event · on_ipc_request                                 │
                 │                    │ single entry: KERNEL.with(|k| …) (panics   │
                 │                    ▼   on re-entry instead of UB)                │
                 │  ┌──────────── Kernel { shell, jobs, net, bridge, ipc } ───────┐ │
                 │  │ NetDispatcher: the ONLY RX consumer                         │ │
                 │  │   rx → Bridge (if enabled) → {forward, deliver to CPU port} │ │
                 │  │   CPU port / SVI → Stack (sans-IO) → tx via Bridge egress   │ │
                 │  │ Bridge (pure crate): fdb · vlan · stp · lacp · lldp         │ │
                 │  │ Stack  (pure crate): arp · ipv4 · icmp · udp · tcp · dns    │ │
                 │  │ Shell  (state machine; never blocks)                        │ │
                 │  └─────────────────────────────────────────────────────────────┘ │
                 │ hal::{time, net, proc, fb, rand} — the ONLY extern "C" module   │
                 └─────────────────────────────────────────────────────────────────┘
```

Principles:
- **Sans-IO cores.** `ecm-net` and a new `ecm-bridge` take `(input, now)` and return
  actions or frames plus a next deadline. No `extern "C"`, no `static mut`, no
  sleeping. Both build and test on the host (`cargo test`, `cargo fuzz`).
- **The kernel never blocks.** `process_wait` becomes "set foreground job and return";
  the host reports `on_child_event(pid, exited|output)`. Socket IPC becomes
  non-blocking: requests are queued, completed on readiness during `on_tick`/RX, and
  Java's child thread waits on its own future (the kernel thread is never parked).
  `sleep_ms` and `terminal_read_line` go away for the kernel.
- **Time is an input.** The host calls `on_tick(now)` when the returned deadline
  passes (the Java worker's `wait(timeout)` uses it). This fixes C1/C3 by
  construction and gives STP/LLDP/LACP real timers.
- **One RX owner.** Only `NetDispatcher` calls `net_rx_frame_*`. The switch is a kernel
  *service* independent of the shell (Ctrl+T can't kill it). The CLI attaches to it.
  Management uses a bridge interface/SVI per VLAN, like Linux `br0`/`br0.10`.
- **One global, borrowed once.** A single `static KERNEL: KernelCell` with a borrow
  flag, taken once per export. Nested entry becomes a loud panic in debug builds and
  can't happen at all once nothing blocks.
- **Memory ABI by export, not by magic number.** The kernel exports the addresses of
  its IO buffers (`abi_layout()`), allocated as statics, so the linker places them
  and the stack can't collide. Interrupt payloads are `&[u8]`.
- **Strict ABI.** `host-abi.toml` lists every import (name, signature, blocking
  semantics, return codes). Both hosts fail to link on a missing import; CI diffs the
  kernel's import section against the spec.

Build vs buy:
- **Adopt `smoltcp` for L3/L4.** The in-house TCP is wrong in fundamental ways (C8 and
  the list above); smoltcp is `no_std`, sans-IO, multi-interface capable and heavily
  tested. Keep our own netlink/config and the ABI glue.
- **Keep the bridge in-house** (nothing suitable exists). Model STP on 802.1D-2004 RSTP
  and LACP on 802.1AX state machines.
- **The switch stays in the kernel, not in a WASI program.** The data plane has to own
  RX at frame rate (the same reason Linux bridging is in-kernel). Drop the
  `switch-os` fork idea: a "switch" is a kernel with the bridge service enabled, so a
  Switch block can be a preset.

---

## 5. Refactor plan (phased; each phase leaves `staging` shippable)

### Phase 0 — Stop the bleeding (small, independent PRs)
1. Register kernel `get_time_ms` in `ComputerInstance`, and add a startup assertion that
   the kernel's imports have no stubs except an explicit allow-list.
2. Bounds-check C6/C7: IPv4 `total_length`, UDP `length`, IPC arg/addr lengths, netlink
   OIF/index/prefix, UDP send size, DNS pointer-hop limit, `nlmsg_len == 0`.
3. Route kernel `getrandom` to the host import (C10).
4. Ctrl+T leaves a `switch on` switch running.
5. Simulator: store the child exit code (C11), fix the default `--wasm` path and the
   script `cd` paths.
6. Refresh `now_ms` in `switch_irq_handler` and call `poll_timers` from it (a stopgap
   for C3 until Phase 4).
7. Mark STP/LACP as experimental in `help`/docs. Don't patch them; they get rewritten
   in Phase 5. Wiring `port_forwards` into a broken STP would create new failure
   modes.

### Phase 1 — Simulator that can tell the truth (see §6, steps S1–S4)
Land this *before* the big refactor, so every later phase is verified against the same
scenarios. Record today's behaviour as characterization tests, marking known bugs as
expected-fail.

### Phase 2 — ABI contract
- `host-abi.toml` plus a generator or checker for the Rust HAL, the Java registrations
  and the simulator registrations.
- Strict linking in all three.
- `abi_conformance.wasm` guest run under both the simulator and a headless Java JUnit
  harness, comparing outputs.

### Phase 3 — Sans-IO network stack
- New `ecm-net` built on smoltcp (or rewritten sans-IO), with a multi-interface
  `Stack::handle_frame / poll(now) -> deadline / transmit queue`.
- Socket handles with generation counters.
- Unit tests, `cargo fuzz` on every parser, and in-process two-stack tests (TCP
  loss/reorder, ARP, DNS).
- The kernel temporarily adapts it behind the old blocking API so Phase 3 ships alone.

### Phase 4 — Kernel core: event-driven, no re-entry
- `hal` module (the only `extern "C"`), a `Kernel` struct, and the `KernelCell`.
  Remove all `static mut`.
- New exports `on_tick`, `on_child_event`, `on_ipc_request`. Remove kernel use of
  `sleep_ms`/`terminal_read_line`/blocking `process_wait`. Java `process_wait` becomes
  a non-blocking protocol; `NetIpcBridge` completes futures from the kernel's replies.
- `NetDispatcher` becomes the single RX owner, and all `poll_rx()` call sites go.
- Byte-slice interrupt payloads, exported memory layout, round-robin
  `net_rx_frame_any`.
- Keep the old exports behind a feature for one release to derisk the Java changes.

### Phase 5 — Bridge rebuilt as `ecm-bridge`
- Port → Aggregator → Bridge-port hierarchy. STP runs on aggregators. Forwarding is
  gated by `link × LACP(collecting/distributing) × STP(forwarding/learning)`.
- FDB (HashMap, aging, TC flush), VLAN ingress/egress (existing logic is decent; port
  it), RSTP, LACP, LLDP, logging. Each is `on_rx/on_tick/on_link -> Vec<Action>`.
- The CLI becomes a separate module that produces config ops (keep the AOS-CX syntax
  and `switch.cfg` format), with SVI management interfaces.
- Tests: in-process multi-bridge `netsim` (§6 S5), plus WASM scenarios.

### Phase 6 — Java network model cleanup
- Segment-indexed delivery with wire semantics (every frame to every NIC on the
  segment; the NIC filters by own MAC, broadcast, multicast and promiscuous).
- Link-down actually cuts delivery, and a carrier-change interrupt is sent to the
  kernel.
- Remove the O(all NICs) scan.

Rough size: P0 ≈ 2–3 days · P1 ≈ 1 week · P2 ≈ 3 days · P3 ≈ 1.5–2 weeks ·
P4 ≈ 2–3 weeks · P5 ≈ 2–3 weeks · P6 ≈ 3–4 days.

---

## 6. Simulator & test plan

Three test layers. Each catches what the one below it can't.

| Layer | What runs | Speed | Catches |
|---|---|---|---|
| **L1 `netsim`** (pure Rust crate) | N × `Stack`/`Bridge` in-process, virtual clock, topology | ms, deterministic | Protocol logic: STP convergence, LACP, TCP under loss, FDB |
| **L2 WASM simulator** | Real `terminal_os.wasm` + real WASI children, same topology file | seconds | Kernel integration, ABI, shell/CLI, process + socket IPC |
| **L3 Java conformance** | Headless JUnit: `WasmtimeRuntime` + fake `NetworkHub`/`CableNetworkManager` | seconds | Host divergence (C1-class bugs) |

Simulator work items:
- **S1 Correctness fixes.**
  - Store the child exit code.
  - Coalesce IRQ_NETWORK to one pending.
  - Deliver interrupts and stdin inside `process_wait`.
  - Epoch interruption and Ctrl+T.
  - Implement `poll_oneoff`.
  - Register `gfx`/`screen`/`mouse` for children (stubs are fine).
  - Strict imports.
  - Default to 5 NICs.
- **S2 Topology.** Replace the global MAC hub with segments loaded from a file.
  Semantics should match the Java model exactly (64-frame drop-oldest queues,
  immediate delivery, wire semantics after P6):
  ```toml
  [[node]]  name = "sw1"  ifaces = 4
  [[node]]  name = "h1"   ifaces = 1
  [[link]]  a = "h1:eth0" b = "sw1:eth0"          # 2-member segment = cable
  [[segment]] members = ["sw1:eth3", "sw2:eth3", "h9:eth0"]   # >2 = hub mesh
  ```
  Runtime `link down/up`, plus fault injection (drop %, delay, duplicate, reorder).
- **S3 Virtual clock.** A single scheduler steps all instances. `get_time_ms`,
  `sleep_ms`, `poll_oneoff` and `on_tick` deadlines use virtual time, and frames move
  at step boundaries. Same seed means the same run, so CI failures are reproducible.
  Wall-clock mode stays for interactive use.
- **S4 Scenario runner.** `terminal-simulator --scenario tests/switch/stp_ring.toml`:
  - `send h1 "ping 10.0.0.3"`
  - `expect h1 /bytes from 10.0.0.3/ within 5s` (virtual time; matches **framebuffer
    text**, not the echoed ANSI stream)
  - `assert_frames link=sw1:eth2 max=100 during 10s` (storm detector)
  - `pcap` per link (Wireshark-readable)
  - exits non-zero on failure

  Replaces the bash scripts, and runs in CI via `cargo test` (one test per scenario
  file).
- **S5 `netsim` crate.** The same topology format driving pure `Stack`/`Bridge`
  objects without WASM. This is where the protocol test matrix lives.

Minimum scenario matrix:
- **L2 basics:** 3 hosts + 1 switch (learning, flooding, unknown unicast, aging after
  300 s virtual, MAC move).
- **VLAN:** access/trunk isolation, native tagged/untagged, allowed-list, management
  over a tagged SVI.
- **STP:**
  - 2- and 3-switch rings: converge, exactly one blocked port, frame count bounded.
  - Root failure and re-election within max-age plus 2×FD.
  - BPDU guard and root guard stay tripped.
- **LACP:**
  - 2-link LAG between two switches forms (catches C9).
  - Member down → failover; partner timeout at 3 s fast / 90 s slow.
  - LAG cabled to two different peers must *not* bundle.
- **TCP:** transfer 1 MB over a 5 % loss link with byte-exact checksum (catches C8);
  close with a pending TX buffer; 8+ concurrent connections.
- **Robustness:** fuzzed frames from a hostile node, the kernel must not panic
  (catches C6/C7).
- **OS integration:**
  - `switch on` + `ping` from the switch box keeps forwarding (catches C4).
  - Ctrl+T keeps a detached switch (§2).
  - A child blocked in recv doesn't freeze the shell.
