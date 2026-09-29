# Simulator notes: host differences, contract gaps, findings

Companion to `rust/simulator/README.md`. Things found while building the
simulator as a second host for the event-driven kernel. Nothing here was
fixed in the kernel, `ecm-net`, `ecm-bridge` or the Java host; each item says
where the fix belongs.

## 1. Where the simulator deliberately differs from the Java host

| Area | Java host | Simulator | Why |
|---|---|---|---|
| Kernel `fd_open` / `pipe_create` | stubs returning -1 (the shell then rejects redirects and pipelines) | implemented (files under the node's storage; 16 KiB kernel pipes; fds passed to `process_spawn`) | lets scenarios cover the shell's redirect/pipeline paths; allowed by `host-abi.toml` ("may be unsupported") |
| `process_spawn` stdio fds | ignored: every child gets the terminal pipes | honoured (`-1` = terminal) | same as above |
| Carrier (`net_get_link_state`) | 1 whenever a cable block sits on the face, even if it leads nowhere | 1 only while the cable is plugged into a live segment; pulling a 2-member link drops carrier on both ends | cable-pull tests; matches real PHYs |
| `process_try_wait` | reports the exit as soon as the child is a zombie | reports it only once the child's terminal output is drained | the simulator throttles `process_read_output` to (screen height - 2) lines per kernel call so every line reaches a framebuffer snapshot; this keeps the tail of the output from being cut off. Kernel-visible semantics are unchanged (a short read is always legal) |
| `poll_oneoff` | clock subscriptions only; fd subscriptions yield 0 events immediately | clocks on the simulation clock, and fd_read on pipes waits for data/EOF | a program polling stdin would otherwise spin |
| `fd_readdir` | stops at the first entry that doesn't fit, leaving the buffer short (callers take a short buffer as end-of-directory) | fills the buffer completely (standard WASI) | see §2 |
| Child entropy (`random_get`, `__getrandom_v03_custom`) and kernel entropy | `java.util.Random` / OS entropy | seeded SplitMix64 streams | reproducible runs; never use the simulator's keys for anything real |
| TAP / Internet gateway | TapBridge | not supported | old Linux-only code removed |

## 2. Java host gaps found (fix in `computer/wasi/WasiFunctions.java`)

Import sets of the built programs compared with what `WasiFunctions`
registers:

- `passwd` imports `env::ipc_auth_set_password`; Java does not provide it,
  so `passwd` cannot be instantiated in-game. (The simulator stubs it: -1.)
- `mv` imports `wasi_snapshot_preview1::path_rename`; missing in Java, so
  `mv` cannot run in-game.
- `python` imports `__getrandom_v03_custom` (as a child), `fd_filestat_set_size`,
  `fd_sync`, `path_filestat_set_times`, `path_link`, `path_readlink`,
  `path_rename`; none are registered for children in Java.
- `fd_readdir` truncation (above): a directory whose entries exceed the
  guest's buffer is listed only partially by Rust std's `read_dir`.
- Children's `random_get` uses `java.util.Random` (predictable). Anything
  generating keys in a child (`ssh-keygen`, sshd host keys) should use
  `SecureRandom`.

A cheap guard for this class of bug: a CI step that compares each program's
import section with the Java registration list (the simulator's
`--check-programs` does this for the simulator's own host).

## 3. Contract (ARCHITECTURE.md / host-abi.toml) gaps

- **No output transcript for hosts.** The only way for a host to see what the
  kernel printed is to snapshot the text framebuffer. A single kernel call
  that prints more than a screenful (e.g. `show running-config` on a big
  switch config, `help`) scrolls lines away before any host can see them.
  The simulator minimises this (snapshot after every call, output throttling
  for children), but kernel-printed bursts can still lose lines from the
  scrollback it builds. Suggestion: an optional export like
  `console_scrollback(ptr, cap) -> len` or a `console_log` import, for test
  hosts and for Minecraft's own terminal history.
- **Carrier changes are polled.** The kernel re-reads carrier every 250 ms;
  there is no interrupt for it. Fine for correctness, but link-down reaction
  time is up to 250 ms plus the next tick. If faster failover is wanted, add
  an IRQ (or reuse IRQ_NETWORK) on carrier change.
- **Carrier semantics** differ between hosts (§1). The contract says "1 if
  cabled to a segment and administratively up"; it would help to say whether
  a cable that leads nowhere counts.
- **Ctrl+T input.** Java's `onStringInput` drops the *whole* input string
  when it contains 0x14 (it only raises IRQ_TERMINATE); the kernel also
  handles 0x14 inside `on_input`. The simulator copies Java. Worth stating in
  the contract which one is authoritative.

## 4. Kernel observations

- `sessions.rs:115` prints an em dash (`Terminal OS — logged in as ...`). The
  VTE stores one byte per cell, so the three UTF-8 bytes render as three
  garbage/blank cells (`Terminal OS     logged in as root.` on screen).
  Cosmetic; use ASCII in kernel-printed text or teach the VTE UTF-8.
- With spanning tree disabled (the default), any cabled loop between two
  switches storms forever (`06_stp_disabled_storm.toml`); expected, but maybe
  worth a CLI warning when a second L2 port comes up with STP off.
- No functional kernel bug was found by the 14 scenarios: boot, child
  output/exit, job control, Ctrl+T (foreground job, stuck call via a fake
  kernel), switch learning/flooding/VLANs/trunks/SVIs/STP/LACP/LLDP,
  `switch.cfg` replay, TCP (including under loss/reordering) and SSH all
  behave as specified.

## 5. Simulator limitations

- Graphics planes, the Screen block, mouse, video and redstone are stubs.
- Storm rates are bounded by the livelock breaker (`--max-rounds-per-ms`);
  only the bounded/unbounded distinction is meaningful.
- A compute-bound child that never blocks for 30 s of wall time makes the
  virtual clock move on without it (a warning is printed) — that part of a
  run is no longer reproducible.
- Interactive mode renders the 16-colour attribute palette only; no mouse.
