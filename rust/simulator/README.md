# terminal-simulator

Runs the real kernel (`terminal_os.wasm`) and the real WASI programs
(`rust/wasm-programs`, built for `wasm32-wasip1`) without Minecraft, on a
simulated cable topology. It is a second host for the kernel, held to the
same contract as the Java host (`abi/host-abi.toml`,
`docs/refactor/ARCHITECTURE.md` §1, §4, §5), plus the things only a
simulator can do: topologies from a file, cable pulls, fault injection, pcap,
a virtual clock and headless scenarios.

```
cd rust
cargo build --release --target wasm32-unknown-unknown -p terminal-os
cargo build --release --target wasm32-wasip1 -p echo -p sleep -p ifconfig -p ping -p curl -p httpd   # or every program
cargo test  --release -p terminal-simulator                    # unit tests + every scenario
cargo run   --release -p terminal-simulator -- --scenario simulator/scenarios/05_stp_loop.toml
cargo run   --release -p terminal-simulator -- --topology my-lab.toml   # interactive
```

`scripts/run-scenarios.sh [filter...]` builds everything and runs the
scenarios; `scripts/test-{all,networking,processes,ssh,switch}.sh` are
filtered wrappers around it.

## Command line

| Option | Meaning |
|---|---|
| `--scenario FILE` | Headless: topology + `[scenario] steps`. Exit 0 pass, 1 fail (report with screens, host logs, segment counters), 2 bad file, 3 kernel/programs not built. |
| `--topology FILE` | Interactive mode on that topology. |
| `--nodes N` | Interactive mode with N unconnected computers `pc1..pcN` (default 1). |
| `--kernel PATH` | Default `rust/target/wasm32-unknown-unknown/release/terminal_os.wasm`. |
| `--programs DIR` | Mounted read-only at `server-bin/` (like the Java host). Default `rust/target/wasm32-wasip1/release`. |
| `--storage DIR` | One subdirectory per node. Scenarios default to a fresh temp dir (deleted afterwards unless `--keep-storage`); interactive to `./simulator-data`. |
| `--out-dir DIR` | Where pcap files go (default `./sim-out`). |
| `--clock real\|virtual` | Default: virtual for scenarios, real interactively. |
| `--seed N` | Seeds fault injection and all guest "entropy" (default 1). |
| `--max-rounds-per-ms N` | Livelock breaker, see below (default 16). |
| `--watchdog 10s` | A kernel call running longer is trapped and `kernel_recover` is called. |
| `--check-programs` | Link-check every program in `--programs` against the WASI host. |
| `--verbose` | Print host log lines (spawns, kills, crashes, traps) live. |

Interactive keys: everything goes to the selected computer; F1..F12 or
Ctrl+N / Ctrl+P switch computers; Ctrl+] opens a simulator command line
that accepts any scenario step (`link down h1:eth0`, `pcap link=h1:eth0
file=x.pcap`, `frames link=sw1:eth2`, `link set lossy drop=20`); Ctrl+Q
quits. Ctrl+T is delivered the way the Minecraft terminal does it.

## Topology / scenario file

```toml
[sim]                       # all optional
seed = 42
clock = "virtual"
timeout = "10s"             # default for expect / wait_prompt
watchdog = "10s"

[[node]]
name = "sw1"                # ifaces defaults to 5 (a terminal's usable faces)
[[node]]
name = "h1"
ifaces = 1
boot = ["ifconfig eth0 10.0.0.1/24"]    # typed after boot, one per prompt

[[link]]                    # a cable = 2-member segment
a = "h1:eth0"
b = "sw1:eth0"
name = "h1-sw1"             # optional; default "h1:eth0--sw1:eth0"
drop = 5.0                  # % of transmissions lost
delay = "2ms"               # one-way delay (or an integer in ms)
duplicate = 1.0             # % delivered twice
reorder = 2.0               # % held back by reorder_delay (default 5ms)
pcap = "h1-sw1.pcap"        # capture from t=0, relative to --out-dir

[[segment]]                 # hub mesh; same fault/pcap keys
name = "hub"
members = ["a:eth0", "b:eth0", "c:eth0"]

[scenario]
name = "my-test"
description = "..."
wall_timeout = "300s"       # wall-clock budget
steps = '''
send h1 "ping 10.0.0.2 -n 3"
expect h1 /3 packets sent, 3 received/ within 10s
'''
```

`topology = "other.toml"` at the top includes another file's nodes, links
and segments. Endpoints are `node:ethN` (or `node:N`).

### Steps

| Step | |
|---|---|
| `send NODE "text"` | Type the text and Enter. Sets the node's *mark*. Escapes: `\n \r \t \e \\ \" \xNN`. |
| `type NODE "text"` | Raw keystrokes, no Enter, mark unchanged. |
| `ctrl_t NODE` | Ctrl+T: interrupt a stuck kernel call (>250 ms) and queue IRQ 15, like `TerminalBlockEntity`. |
| `expect NODE /re/ [within 5s]` | Wait until the regex matches the text produced after the mark. |
| `expect_any NODE /re/ [within 5s]` | Same, over the node's whole transcript. |
| `expect_not NODE /re/ [for 3s]` | Fail if it matches now or at any time during the window. |
| `wait_prompt NODE [within 5s]` | Cursor is on a line after the mark that ends in `>` or `#`. |
| `wait 2s` | Let time pass. |
| `link down EP` / `link up EP` | Pull / re-plug a cable. For a 2-member link both ends lose carrier; for a hub segment only that member is detached. A segment name works too. |
| `link set EP drop=10 delay=5ms duplicate=0 reorder=0 reorder_delay=5ms` | Change faults at runtime. |
| `assert_frames link=EP [max=N] [min=N] during 10s` | Run for the window and count frames transmitted onto the segment (both directions, before fault injection). Storm detector. |
| `frames link=EP` | Print the counters. |
| `pcap link=EP file=F` / `pcap_stop link=EP` | Start/stop a libpcap capture (Ethernet, µs timestamps in simulated time). |
| `dump NODE` | Print the screen. |
| `log "text"` | Print a note. |

**What `expect` matches.** Text is taken from the kernel's framebuffer
after every kernel call, not from any byte stream. Lines that scroll off
are kept in a scrollback (by diffing consecutive snapshots). Rows are
right-trimmed; non-printable cells are spaces. Regexes are Rust `regex`
with `(?m)`, so `^`/`$` anchor at screen lines; `/.../i` is
case-insensitive, `\/` is a literal slash. `send` records the cursor
position; `expect` only considers the rest of that line with the echo of the
typed text removed, plus every later line — so `send h1 "echo hi"` followed
by `expect h1 /hi/` can never be satisfied by the echoed command, while
output of a program that doesn't echo its input (keyboard to a running
child) still matches.

## Design

- **Kernel host (`kernel.rs`).** Every import of `abi/host-abi.toml` with
  Java semantics. Strict linking: before instantiation every kernel import
  is compared (name and signature) with what the host provides, and all
  problems are reported at once. Region addresses come from `abi_scratch` +
  `abi_layout` and are bounds-checked against memory. There are no stubs for
  the kernel: `fd_open`/`pipe_create` are implemented (the Java host stubs
  them, so redirects and pipelines only work in the simulator), the
  screen cluster reports "no screen attached".
- **Worker loop (`Kernel::step`).** One iteration of `ComputerInstance.workerLoop`:
  queued interrupts, one coalesced IRQ_NETWORK (flag cleared before
  delivery), keyboard input in input-region-sized chunks, then `on_tick`
  when the returned deadline passed or anything happened (a deadline of -1
  means "again in 100 ms", as in Java), then socket IPC: new requests plus
  retries of `IPC_PENDING` ones, and another tick if any completed.
- **Traps.** Epoch interruption is on everywhere; a ticker thread bumps the
  epoch every 10 ms. A kernel call is trapped on Ctrl+T while stuck
  (>250 ms) or after the watchdog; the host then calls `kernel_recover`. Any
  other trap faults that computer. A child is trapped as soon as it is
  killed, even in a compute loop.
- **Children (`child.rs`, `proc.rs`, `ipc.rs`).** One thread and one store
  per process; modules are compiled once and cached (and wasmtime's disk
  cache is enabled). 16 KiB terminal-output pipe and 4 KiB stdin pipe,
  read/written by the kernel without blocking. Exit codes are stored (the old
  "never exits" hang is gone): `proc_exit(n)` → n, killed → 130, trap → 1,
  load/link failure → 126/127 with a message on the terminal. Socket
  functions and the sshd session functions (`ipc_spawn_shell`,
  `ipc_session_*`) are proxied to `handle_sock_ipc` with the Java argument
  layouts and `[status][payload]` results; the child waits until the kernel
  answers. `poll_oneoff` sleeps on the simulation clock (and also wakes for
  readable pipes), `fd_fdstat_set_flags(O_NONBLOCK)` on pipe ends gives
  `EAGAIN` reads. gfx/screen/mouse/video imports are stubs (-1 / no events);
  `ipc_auth_set_password` returns -1.
- **Wire (`net.rs`).** Exactly `NetworkHub`: a frame goes to every other NIC
  on the segment, each NIC accepts its own MAC, broadcast, multicast or
  everything if promiscuous; admin-down NICs neither send nor receive; 256
  frames per NIC, drop oldest; round-robin `net_rx_frame_any`. Carrier =
  admin up and cable plugged into a live segment. MACs are
  `02:<iface>:5e:00:<node>` (node counted from 1).
- **Clock (`sched.rs`, `sim.rs`).** All kernels run on the main thread, one
  worker-loop iteration each per round, in node order. Every child thread is
  an actor that is either running or blocked in a host call (IPC wait,
  sleep, pipe read, full-pipe write, poll). In virtual mode each round starts
  by waiting until no actor runs; if a round has nothing to do, time jumps to
  the earliest deadline (kernel ticks, child sleeps, delayed frames).
  Blocking predicates are evaluated under the scheduler lock and wakers
  count the woken actor as busy before it runs, so time never advances
  between a wake-up and the woken child running.
- **Livelock breaker.** After `--max-rounds-per-ms` busy rounds at the same
  instant the clock moves on by 1 ms. Zero-delay links therefore still have a
  finite frame rate: a broadcast storm on a two-switch loop shows up as about
  two frames per link per round, ~32 per link per virtual ms at the default
  cap (see `06_stp_disabled_storm.toml`). Absolute storm rates are an
  artefact of this cap; use `assert_frames` to tell bounded from unbounded.
- **Spin protection.** A child that reads the clock 2000 times, or polls a
  non-blocking pipe, without ever blocking is treated as waiting for time and
  sleeps 1 virtual ms.

**Determinism.** Given the same seed a virtual-clock run is reproducible:
kernels are stepped in a fixed order at quiescence, IPC requests are served
in session order, fault injection and guest entropy come from seeded
streams. Residual nondeterminism: (1) a child that computes for more than 30 s
of wall time without blocking stops holding the clock back (a warning is
printed), after which its progress relative to virtual time depends on the
machine; (2) the watchdog is wall-clock based; (3) children writing the same
file concurrently race as they would anywhere.

## Scenarios (`scenarios/`)

| File | Covers |
|---|---|
| `01_boot_echo` | boot banner, child output, prompt, echo exclusion |
| `02_two_hosts_ping` | ifconfig, ping both ways, cable pull/replug, pcap |
| `03_switch_three_hosts` | `switch on` + CLI port config, pings via switch, MAC learning, unknown-unicast flooding after `clear mac-address-table` |
| `04_vlan_isolation` | two VLANs on one switch, cross-VLAN fails, broadcasts stay in their VLAN |
| `05_stp_loop` | RSTP on a two-switch loop: roles/states, pings, bounded frame counts |
| `06_stp_disabled_storm` | same loop without STP: storm detected (characterization) |
| `07_lacp_failover` | LACP fast LAG, member pulled mid-ping (both members in turn) |
| `08_tcp_http_via_switch` | httpd in the background, curl GET/POST/-v, refused connect, redirects |
| `09_ctrl_t_switch_keeps_forwarding` | Ctrl+T on the switch box and on a host while the detached switch forwards |
| `10_faults_and_hub` | drop/delay/dup/reorder, TCP under loss, hub segment, single-member pull |
| `11_processes` | pipes, redirects, stdin to a child, background jobs, exit codes |
| `12_ssh` | sshd + ssh between two computers through kernel-hosted sessions |
| `13_switch_cli_svi_config` | CLI show output, SVI management, `write memory` + restart replay |
| `14_vlan_trunk_lldp` | tagged trunk between switches, SVI over the trunk, LLDP neighbours |

`tests/scenarios.rs` runs them all (`cargo test -p terminal-simulator
--test scenarios [-- word...]`). It fails, printing the build commands, if
the kernel or the needed programs are not built.

## Not supported

- TAP / real internet (the old Linux-only `--tap` is gone; the Java host
  still has its TapBridge).
- Graphics planes, the in-world screen, mouse, video, redstone (stubs).
- Old kernels: only the event-driven ABI (layout version 1) is accepted.

Known gaps and host differences are tracked in
`docs/refactor/SIMULATOR_NOTES.md`.
