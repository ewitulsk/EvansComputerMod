# Switch Instructions

A guide to the L2 switch built into the terminal computer's kernel. Includes the complete command reference (Phases 1 + 2 + 3 + 4 + 5 + 6) and getting-started walkthroughs for basic L2 switching and VLAN configuration.

The switch is a kernel built-in, not a WASI program. You enter it from the main shell by typing `switch`. It uses an Aruba AOS-CX-inspired command syntax with nested configuration contexts (`switch(config)#`, `switch(config-vlan-N)#`, `switch(config-if-ethN)#`).

---

## Table of contents

1. [Entering and leaving switch mode](#entering-and-leaving-switch-mode)
2. [Running the switch in the background](#running-the-switch-in-the-background)
3. [Saving and loading configuration](#saving-and-loading-configuration)
4. [Configuration contexts](#configuration-contexts)
5. [Interface numbering and physical directions](#interface-numbering-and-physical-directions)
6. [Port modes — routed, access, trunk](#port-modes)
7. [Command reference](#command-reference)
    - [Top-level commands](#top-level-commands)
    - [VLAN context commands](#vlan-context-commands)
    - [Interface context commands](#interface-context-commands)
    - [show commands](#show-commands)
    - [clear commands](#clear-commands)
8. [Getting started — basic L2 switching](#getting-started--basic-l2-switching)
9. [Getting started — VLANs](#getting-started--vlans)
10. [Defaults and limits](#defaults-and-limits)
11. [Notes and gotchas](#notes-and-gotchas)
12. [Phase 3 — Logging](#phase-3--logging)
13. [Phase 4 — LLDP](#phase-4--lldp)
14. [Phase 5 — Spanning Tree (single-instance CIST)](#phase-5--spanning-tree-single-instance-cist)
15. [Phase 6 — LACP and Link Aggregation Groups](#phase-6--lacp-and-link-aggregation-groups)
16. [Cut corners (Phases 3–6)](#cut-corners-phases-36)

---

## Entering and leaving switch mode

From the main shell prompt, type:

```
/ > switch
Entering switch configuration mode.
Type 'help' for the command list, 'exit' to return to the shell.
switch(config)#
```

Entering switch mode:
- Enables promiscuous mode on every one of this computer's NICs so the switch sees every frame arriving on any port, not just frames destined for this computer's MAC.
- Replaces the kernel's `IRQ_NETWORK` handler with the switch forwarding path. Frames addressed to one of this computer's own MACs (or broadcasts) are still delivered to the normal kernel network stack so management traffic (ARP, ping, SSH, etc.) keeps working.
- Creates an empty MAC table and a VLAN database containing only VLAN 1 (`default`, active).

To leave switch mode, any of these work:

- `exit` — from the top-level `switch(config)#` context, leaves the CLI. From a nested VLAN or interface context, pops one level back to the top.
- `end` — always leaves the CLI immediately, regardless of context.
- `quit` — identical to `end`.
- `Ctrl+T` — emergency bail-out. Also leaves the CLI **and** force-stops the switch.

What "leaving" actually does depends on how the session was started — see [Running the switch in the background](#running-the-switch-in-the-background). The short version: if you entered with plain `switch`, `exit` tears down forwarding too (old behavior). If you entered after `switch on` (or ran `on` inside the CLI), `exit` just leaves the CLI and the switch keeps forwarding in the background. Only `switch off` or `Ctrl+T` fully stops forwarding in that case.

Config state (VLANs, port modes, static MACs, MAC age-time) can be persisted across sessions with `write memory` — see [Saving and loading configuration](#saving-and-loading-configuration).

---

## Running the switch in the background

By default, `switch` is a two-in-one command: it starts forwarding **and** drops you into the config CLI. When you `exit`, both go away. That's fine for quick edits, but it means you can't run anything else on the terminal while the switch is forwarding — you're stuck in the config prompt.

To work around this, there are two extra invocations:

| Command         | What it does                                                                                                    |
|-----------------|-----------------------------------------------------------------------------------------------------------------|
| `switch`        | Enter the config CLI. If the switch isn't running, start it. If it's already running (e.g. from a prior `switch on`), re-enter without re-initializing — your VLAN / MAC / port config is preserved. |
| `switch on`     | Start forwarding in the background. Stay at the main shell prompt. You can now run other programs (`tcpdump`, `ping`, etc.) while the switch forwards packets. If already running, prints a note and leaves state alone. |
| `switch off`    | Stop forwarding and tear down all switch state. Works from the main shell or from inside the CLI. If not running, prints a note and does nothing. |

Inside the CLI, two more commands let you toggle forwarding state mid-session:

| Command | What it does                                                                                                    |
|---------|-----------------------------------------------------------------------------------------------------------------|
| `on`    | Mark the current CLI session as persistent — subsequent `exit` will leave the CLI without tearing the switch down. |
| `off`   | Same as `switch off`. Stops forwarding and returns you to the main shell.                                       |

### `exit` behavior depends on how you entered

The key rule: **exit preserves forwarding if and only if the session was marked persistent at some point before the exit.**

- Bare `switch` → `exit` → forwarding torn down. (Unchanged from the original behavior — existing muscle memory still works.)
- `switch on` → (switch running, main shell) → `switch` → `exit` → forwarding **kept**.
- Bare `switch` → `on` (inside CLI) → `exit` → forwarding **kept** (promoted mid-session).
- Bare `switch` → `write memory` → `exit` → forwarding torn down (unless you also ran `on`).

When `exit` preserves the switch, you'll see:

```
switch(config)# exit
Exited switch mode. Switch continues running in background.
Use 'switch off' to stop it.
/ >
```

### Worked example: tcpdump alongside the switch

```
/ > switch on
Switch started. Forwarding in background. Use 'switch off' to stop.
/ > switch
Re-entering switch configuration mode (switch is running).
switch(config)# interface eth0
switch(config-if-eth0)# no routing
switch(config-if-eth0)# exit
switch(config)# interface eth1
switch(config-if-eth1)# no routing
switch(config-if-eth1)# exit
switch(config)# exit
Exited switch mode. Switch continues running in background.
Use 'switch off' to stop it.
/ > tcpdump eth0
listening on eth0...
```

Now traffic arriving on `eth0` (which the switch is forwarding to `eth1`) also shows up in tcpdump. Hit Ctrl+T to stop tcpdump, and the switch keeps running.

When you're done:

```
/ > switch off
Switch stopped.
/ >
```

### Ctrl+T still kills everything

`Ctrl+T` always force-stops the switch regardless of the persist flag — the emergency bail-out path in `reset_to_shell` is unconditional. Use `Ctrl+T` when you want a hard reset; use `switch off` for a clean stop.

### State-transition cheat sheet

| You are at…                                  | You run…            | You end up at…                                |
|----------------------------------------------|---------------------|-----------------------------------------------|
| Main shell, switch off                       | `switch`            | CLI, switch running, *non*-persistent         |
| Main shell, switch off                       | `switch on`         | Main shell, switch running, persistent        |
| Main shell, switch running                   | `switch`            | CLI, switch running, same persistence         |
| Main shell, switch running                   | `switch off`        | Main shell, switch off                        |
| CLI, *non*-persistent                        | `exit`/`end`/`quit` | Main shell, switch off                        |
| CLI, persistent                              | `exit`/`end`/`quit` | Main shell, switch still running              |
| CLI, any mode                                | `on`                | CLI, persistent                               |
| CLI, any mode                                | `off`               | Main shell, switch off                        |
| Anywhere                                     | `Ctrl+T`            | Main shell, switch off                        |

---

## Saving and loading configuration

Switch config — VLAN database, per-port modes, MAC age-time, static MAC entries — can be persisted to `/switch.cfg` so it survives `switch off`, `Ctrl+T`, or a server restart.

### Save: `write memory`

Run inside the CLI (from the top-level context):

```
switch(config)# write memory
Configuration saved to /switch.cfg.
```

No autosave: you have to run `write memory` explicitly when you want to persist. This matches Aruba/Cisco semantics and prevents accidental overwrites when you're just experimenting. If you forget, `switch off` / Ctrl+T / a power cycle throw the edits away — exactly like on real network gear.

### Load: automatic on `switch` / `switch on`

Every time the switch starts (either via `switch` or `switch on`), it tries to read `/switch.cfg` and replay the commands it finds there. If the file doesn't exist, nothing happens — that's the normal first-run state. If it does exist, you'll see each applied command echo back:

```
/ > switch on
Loaded switch.cfg (12 commands applied).
Switch started. Forwarding in background. Use 'switch off' to stop.
/ >
```

Re-entering the CLI on an already-running switch (`switch on` then `switch`) does **not** reload. The in-memory state is authoritative — if you want to re-sync from disk, run `switch off` then `switch on` again.

### File format

`/switch.cfg` is a plain-text file where each non-blank, non-comment line is a valid CLI command. Lines starting with `#` are comments; leading whitespace is decorative and is trimmed before parsing. The format is effectively a dump of your running config:

```
# switch.cfg - generated by 'write memory'
mac-address-table age-time 300
vlan 10
 name engineering
 description Engineering network
 no shutdown
exit
vlan 20
 name guest
 no shutdown
exit
interface eth0
 no routing
 vlan access 10
exit
interface eth5
 no routing
 vlan trunk native 1
 vlan trunk allowed 10,20
exit
static-mac 02:00:00:00:00:10 vlan 10 port 3
```

The file is the same grammar as the interactive CLI, which means you can hand-edit it:

```
/ > edit /switch.cfg
```

or inspect it:

```
/ > cat /switch.cfg
```

### What gets saved

- `mac-address-table age-time` (only if non-default)
- Every VLAN other than the trivial default (VLAN 1 with no custom name/description)
- Every port that's been converted away from the default `routed` mode (i.e. access and trunk ports)
- Every static MAC entry

### What does NOT get saved

- **Dynamic (learned) MAC entries.** They regenerate from traffic after the switch restarts.
- **The current sub-shell context** (Top / VLAN / Interface). The loader always finishes at `Top`.
- **Session flags** like `persist_on_exit`. Those are tied to how the session was started, not to the config.
- **Whether the switch was running when you last ran `write memory`.** The file is just config — it does not auto-start the switch on OS boot. Run `switch on` (from the main shell, or via a future startup hook) when you want forwarding.

### Caveats

- **Malformed or out-of-range lines print their usual errors and are skipped.** For example, a `static-mac` referring to a port that no longer exists (e.g. if you removed an `InterfaceBlock` and the terminal now has fewer NICs) prints "`% Port N out of range`" during the load and the rest of the file continues.
- **The loader uses `exec_command` with a `loading` guard**, so a config file that contains `exit` lines at the end of each block doesn't accidentally tear down the switch during replay. You can safely chain VLAN and interface blocks back-to-back.
- **Deleting `/switch.cfg`** fully resets the persisted state; the next `switch on` starts from scratch.

---

## Configuration contexts

The prompt tells you which context you're in:

| Prompt                         | Context                                     |
|--------------------------------|---------------------------------------------|
| `switch(config)#`              | Top-level. Create VLANs, configure global settings, enter sub-contexts. |
| `switch(config-vlan-N)#`       | Editing VLAN `N`. Set name, description, shutdown state. |
| `switch(config-if-ethN)#`      | Editing interface `ethN`. Set port mode and VLAN membership. |

`show` commands work from any context (they're universal). `help` and `?` print a context-aware command list. `exit` pops one level; `end`/`quit` leaves switch mode entirely.

---

## Interface numbering and physical directions

The switch names ports `eth0` through `eth<N-1>`, where `N` is the number of NICs this computer owns. Which physical face each port corresponds to depends on two things:

1. **The terminal's screen face (its `FACING` direction).** The screen face is never a NIC — there's no interface on the side where the monitor lives.
2. **Which of the other five faces have `InterfaceBlock`s attached.** Faces with an `InterfaceBlock` are skipped during the main terminal's own NIC enumeration and instead contribute additional NICs through any free face of the attached `InterfaceBlock` chain (see below).

### Isolated terminal (no InterfaceBlocks attached)

For a terminal with nothing attached to any face, you get exactly **5 NICs** — one per non-screen face. The order is Minecraft's `Direction.values()` iteration (DOWN, UP, NORTH, SOUTH, WEST, EAST), skipping the screen face. That means `eth0` is always the first face in that order that isn't the screen, `eth1` is the second, and so on.

The table below lists the mapping for every possible terminal orientation. "Player's left / right" is written from the perspective of someone **standing in front of the screen and looking at it** — so "player's left" means the face that appears on the viewer's left while facing the screen.

| Screen faces | eth0  | eth1  | eth2         | eth3         | eth4         |
|--------------|-------|-------|--------------|--------------|--------------|
| **North**    | Down  | Up    | South (back) | West (player's right) | East (player's left)  |
| **South**    | Down  | Up    | North (back) | West (player's left)  | East (player's right) |
| **East**     | Down  | Up    | North (player's left) | South (player's right) | West (back) |
| **West**     | Down  | Up    | North (player's right) | South (player's left) | East (back) |
| **Up**       | Down (back) | North | South | West | East |
| **Down**     | Up (back)   | North | South | West | East |

A couple of constants you can rely on regardless of screen orientation:

- **`eth0` is always the bottom face** for any horizontally-facing terminal (screen facing N/S/E/W). Cable from beneath and you always land on `eth0`.
- **`eth1` is always the top face** for any horizontally-facing terminal.
- The "back" face (directly opposite the screen) is always `eth2` for N/S/E screens and `eth4` for a W-facing screen, because `Direction.values()` reaches it at different points in the iteration. If you want a stable "back" cable port across orientations, check with `show mac-address-table` after a test ping.

For vertically-oriented terminals (screen on the top or bottom face — a table-mounted or ceiling-mounted terminal), the four side faces (N/S/E/W) take slots `eth1`–`eth4`, and the opposite vertical face is `eth0`.

### Terminals with `InterfaceBlock`s attached

Attaching an `InterfaceBlock` to a face of the terminal extends that face into a chain of additional NICs. The enumeration algorithm is:

1. Walk the six faces of the main terminal block in `Direction.values()` order. Skip the screen face. Skip any face that has an `InterfaceBlock` directly attached. Every remaining face becomes a built-in NIC, in order (`eth0`, `eth1`, …).
2. After all built-in NICs are assigned, BFS out through the attached `InterfaceBlock` chain. Every **free** face on every `InterfaceBlock` visited in that BFS becomes an additional NIC, in BFS order, continuing the numbering (`eth<next>`, `eth<next+1>`, …).

Consequences:

- **`InterfaceBlock`-mounted NICs always come after the built-in ones.** If your terminal has 3 built-in NICs and 1 `InterfaceBlock` exposing 4 free faces, the built-ins are `eth0`–`eth2` and the interface-block NICs are `eth3`–`eth6`.
- **Removing or adding an `InterfaceBlock` renumbers everything.** The BFS order is stable given the same world state, but if you add a new `InterfaceBlock` the BFS visits more nodes and the tail of your numbering shifts. If you rely on specific port numbers in a config, avoid rewiring the physical topology without re-checking.
- **The NIC on a face attached to an `InterfaceBlock` lives at the *far* end of the chain, not on the terminal itself.** That is, if the south face of the terminal has an `InterfaceBlock` whose east face is free, the resulting NIC (with its exit position on that east face) is what takes a slot in the numbering — the terminal's own south face doesn't get one.

### Verifying which port is which

If you're not sure which physical face corresponds to `ethN`, the easiest way to check is:

1. In switch mode, run `clear mac-address-table dynamic` to start with a clean table.
2. From a known client wired to a known face, send any frame (a `ping` against another machine or even a non-existent IP is enough — the ARP request is a broadcast the switch will learn from).
3. Run `show mac-address-table` on the switch. The client's MAC appears next to the port number that corresponds to the face you cabled.

Repeat for each face you want to identify.

---

## Port modes

Every port has one of three modes. **The default for every port is `routed` (L3)** — this matches Aruba AOS-CX and means a fresh switch does no L2 forwarding until you explicitly convert ports with `no routing`.

### 1. Routed (default)

No L2 switching on this port. Frames addressed to this computer's own MACs (or broadcasts) are still delivered to the kernel stack so you can ping/SSH into it, but forwarding is disabled. This is the `PortMode::Routed` state.

Use for ports you want to stay L3 (e.g. an uplink you'll give an IP to).

### 2. Access

Single untagged VLAN. All ingress frames **must** be untagged and are placed in the port's configured access VLAN. All egress frames are sent untagged. Tagged ingress is dropped.

Use for end-host ports (clients, servers).

### 3. Trunk

802.1Q tagged trunk. Carries one native (untagged by default) VLAN plus an allowed list of tagged VLANs.

- **Native VLAN**: untagged ingress lands in this VID, egress in this VID goes out untagged. If you add the `tag` flag (`vlan trunk native N tag`), untagged ingress is dropped and native-VID egress is also tagged — i.e. the trunk becomes fully tagged.
- **Allowed list**: which VIDs this trunk will pass. Default after `vlan trunk native` is `all` (every VLAN in the database, including ones added later). You can narrow it with `vlan trunk allowed <list>`.

Use for switch-to-switch links.

---

## Command reference

### Top-level commands

Available at `switch(config)#`.

#### `vlan <vlan-id>`

Creates VLAN `<vlan-id>` if it doesn't already exist, then enters the `switch(config-vlan-<id>)#` context. Newly created VLANs start in shutdown state; you must run `no shutdown` inside the VLAN context to activate them.

```
switch(config)# vlan 10
switch(config-vlan-10)#
```

VLAN IDs are in the range `1`–`4094`. VLAN 1 is the default and is always active.

#### `vlan <start>-<end>`

Bulk-creates a contiguous range of VLANs without entering any context. New VLANs are created in shutdown state.

```
switch(config)# vlan 100-110
Created 11 VLANs (range 100-110).
```

#### `no vlan <vlan-id>`

Deletes a VLAN. Fails if any port's config still references the VID (you must unconfigure those ports first). VLAN 1 cannot be deleted. Deleting a VLAN also drops any MAC table entries learned in that VLAN.

#### `interface <port-num | ethN>`

Enters the interface configuration context for port `<port-num>` (e.g. `interface 2` or `interface eth2`).

```
switch(config)# interface eth2
switch(config-if-eth2)#
```

Valid port numbers are `0` through `<iface_count - 1>`, where `iface_count` is the number of NICs the host terminal exposes (typically 6 — one per side of the block).

#### `static-mac <mac> vlan <vlan-id> port <port-num>`

Install a static MAC entry in the forwarding table. Static entries are never aged out and override dynamic learning for the same `(mac, vlan)` pair. `<mac>` can use colons or dashes (`aa:bb:cc:dd:ee:ff` or `aa-bb-cc-dd-ee-ff`).

```
switch(config)# static-mac 02:00:00:00:00:10 vlan 10 port 3
Static MAC entry added.
```

If the table is full (512 entries), the oldest dynamic entry is evicted to make room. If all 512 slots are static, the command fails.

#### `no static-mac <mac> vlan <vlan-id> port <port-num>`

Remove a specific static entry. The `vlan` and `port` must match the existing entry exactly.

#### `mac-address-table age-time <15-1000000>`

Set the dynamic-entry aging time in seconds. Dynamic entries not refreshed within this window are dropped from the table on the next aging pass (which runs at most once per second). Default: `300` seconds. Minimum: `15`, maximum: `1_000_000`.

```
switch(config)# mac-address-table age-time 60
MAC age-time set to 60 seconds.
```

#### `no mac-address-table age-time`

Reset the aging time to the default (300 seconds).

#### `help` / `?`

Context-aware command list. In the top-level context, lists all commands available there; in a VLAN or interface context, lists only the commands valid in that context.

#### `exit` / `end` / `quit`

Leaves the current context (`exit`) or leaves switch mode entirely (`end` / `quit`). See [Entering and leaving switch mode](#entering-and-leaving-switch-mode).

---

### VLAN context commands

Available at `switch(config-vlan-N)#`.

#### `name <name>`

Assign a display name to this VLAN. Used in `show vlan`. Names can contain spaces; everything after `name` is taken as the name.

```
switch(config-vlan-10)# name engineering
VLAN name set.
```

#### `description <text>`

Assign a longer description. Everything after `description` is taken as the text.

#### `no shutdown`

Activate the VLAN. New VLANs (except VLAN 1) are created inactive; they **will not forward traffic** until `no shutdown` has run. Frames in an inactive VLAN are silently dropped at ingress.

```
switch(config-vlan-10)# no shutdown
VLAN activated.
```

#### `shutdown`

Deactivate the VLAN. Frames in this VLAN stop being forwarded. VLAN 1 cannot be shut down.

#### `exit`

Return to `switch(config)#`.

#### `end` / `quit`

Leave switch mode.

---

### Interface context commands

Available at `switch(config-if-ethN)#`.

**Important**: every port starts in `routed` (L3) mode. Before any `vlan access` or `vlan trunk` command will work, you must first run `no routing` to convert the port to L2.

#### `no routing`

Convert the port from L3 (routed) to L2 (access vlan 1 by default). This is required before any VLAN-membership command can be run on the port.

```
switch(config-if-eth1)# no routing
Port converted to L2 (access vlan 1).
```

#### `vlan access <vlan-id>`

Make the port an access port in VLAN `<vlan-id>`. If the VLAN doesn't exist, it's created in shutdown state and you'll see a warning — remember to enter the VLAN context and `no shutdown` it, otherwise traffic in that VLAN is dropped.

```
switch(config-if-eth1)# vlan access 10
Access VLAN set to 10.
```

If the VLAN exists but is in shutdown state, you'll get a warning but the port is still configured.

#### `no vlan access <vlan-id>`

Reset the port's access VLAN back to the default (VLAN 1). The `<vlan-id>` argument is accepted for symmetry with the set form but isn't strictly validated against the current value.

#### `vlan trunk native <vlan-id> [tag]`

Convert the port to trunk mode and set the native (untagged) VLAN. Implicitly sets the allowed list to `all`. Add the `tag` suffix to make the trunk fully tagged — untagged ingress will then be rejected and egress in the native VID will also be tagged.

```
switch(config-if-eth5)# vlan trunk native 1
Trunk native VLAN set to 1.

switch(config-if-eth5)# vlan trunk native 100 tag
Trunk native VLAN set to 100 (tagged).
```

#### `no vlan trunk native <vlan-id>`

Reset the trunk's native VLAN to 1. The port stays in trunk mode. The `<vlan-id>` argument is accepted for symmetry.

#### `vlan trunk allowed <vlan-list | all>`

Set which VLANs this trunk is allowed to pass. Requires the port to already be in trunk mode.

- `all` — every VLAN currently in the database, plus any that get added later.
- A comma-separated list with optional ranges: `10,20,30-40,100`.

```
switch(config-if-eth5)# vlan trunk allowed 10,20,30-40
Trunk allowed VLAN list set.

switch(config-if-eth5)# vlan trunk allowed all
Trunk allowed VLANs set to all.
```

Invalid ranges (e.g. `20-15`, or numbers outside `1-4094`) are rejected with `% Invalid VLAN list: ...`.

#### `no vlan trunk allowed <vlan-list>`

Subtracts the listed VIDs from the trunk's allowed list. If the list was `all`, it is first materialized from the current VLAN database and then the listed VIDs are subtracted. Does not accept `all` as an argument.

```
switch(config-if-eth5)# no vlan trunk allowed 30-40
Removed VLANs from trunk allowed list.
```

#### `exit`

Return to `switch(config)#`.

#### `end` / `quit`

Leave switch mode.

---

### show commands

Available in every context.

#### `show mac-address-table`

Dump the entire MAC table, preceded by the configured age-time and the total entry count.

```
switch(config)# show mac-address-table
MAC age-time            : 300 seconds
Number of MAC addresses : 2

MAC Address         VLAN    Type     Port
-----------------------------------------
02:00:a1:b2:c3:d4   1       dynamic  0
02:00:e5:f6:07:08   10      static   3
```

#### `show mac-address-table dynamic`

Only dynamic (learned) entries. Optional filters:

- `show mac-address-table dynamic port <n>` — only entries learned on port `<n>`.
- `show mac-address-table dynamic vlan <id>` — only entries in VLAN `<id>`.

#### `show mac-address-table static`

Only static (admin-configured) entries.

#### `show mac-address-table vlan <vlan-id>`

All entries in the given VLAN.

#### `show mac-address-table port <port-num>`

All entries whose forwarding port is `<port-num>`.

#### `show mac-address-table address <mac>`

Look up a specific MAC address. Matches regardless of VLAN or port.

#### `show mac-address-table count [dynamic | port <n> | vlan <id>]`

Just the count, with optional filter.

```
switch(config)# show mac-address-table count
Number of entries: 37
```

#### `show mac-address-table mac-move [address <mac> vlan <id> | vlan <id>]`

List entries that have moved between ports (flap detection). Shows the current port, the previous port, move count, and the timestamp of the last move. Useful for spotting link loops or a host that's mis-cabled and appearing on two ports alternately.

```
switch(config)# show mac-address-table mac-move
MAC Address         VLAN    Curr  Prev  Moves  Last-Move
-----------------------------------------------------------
02:00:a1:b2:c3:d4   1       2     0     3      00:14:22.107
```

With filters you can narrow to a single MAC or a single VLAN.

#### `show vlan`

Full VLAN table with per-VLAN status, reason, and port membership.

```
switch(config)# show vlan
VLAN  Name             Status   Reason       Ports (untagged / tagged)
----  ---------------  -------  -----------  -----------------------------
1     default          up       ok           eth0,eth1,eth2 / eth5
10    engineering      up       ok           eth3 / eth5
20    guest            down     no-members   - / -
```

The **Status** / **Reason** columns:

- `up / ok` — the VLAN has at least one member port with link up.
- `down / admin-down` — the VLAN is in shutdown state.
- `down / no-members` — no port is a member of this VLAN.
- `down / no-member-up` — there are members but none have link up.

#### `show vlan <vlan-id>`

The row for a single VLAN, plus the explicit untagged/tagged port lists underneath.

```
switch(config)# show vlan 10
VLAN  Name             Status   Reason       Ports (untagged / tagged)
----  ---------------  -------  -----------  -----------------------------
10    engineering      up       ok           eth3 / eth5

Untagged ports: eth3
Tagged ports:   eth5
```

#### `show vlan summary`

Just the VLAN count.

```
switch(config)# show vlan summary
Number of VLANs: 3
```

#### `show vlan port <port-num>`

Show what a specific port's VLAN config looks like.

```
switch(config)# show vlan port 5
Port: eth5
Mode: trunk
Native VLAN: 1 (untagged)
Allowed VLANs: 1,10,20
```

---

### clear commands

#### `clear mac-address-table dynamic`

Flush every dynamic (learned) entry from the MAC table. Static entries are preserved.

```
switch(config)# clear mac-address-table dynamic
Cleared 37 dynamic entries.
```

#### `clear mac-address-table dynamic vlan <vlan-id>`

Flush dynamic entries in a specific VLAN.

#### `clear mac-address-table dynamic port <port-num>`

Flush dynamic entries learned on a specific port.

#### `clear mac-address-table dynamic address <mac>`

Flush dynamic entries for a specific MAC address (regardless of VLAN).

---

## Getting started — basic L2 switching

Goal: make a terminal block behave as a plain 6-port L2 switch so anything plugged into any face can reach anything on any other face.

### 1. Cable the computers

Place a terminal block (the switch) and at least two other terminal blocks (the clients). Connect each client to a different face of the switch using network cable blocks. Cable network topology is computed by the BFS in `CableNetworkManager`, so any path through network cable blocks between two NIC exit positions counts as "same network".

### 2. Enter switch mode

On the switch terminal:

```
/ > switch
Entering switch configuration mode.
switch(config)#
```

### 3. Convert every port to L2 access

Every port is `routed` by default and will not forward. Convert each wired face to access VLAN 1:

```
switch(config)# interface eth0
switch(config-if-eth0)# no routing
Port converted to L2 (access vlan 1).
switch(config-if-eth0)# exit

switch(config)# interface eth1
switch(config-if-eth1)# no routing
Port converted to L2 (access vlan 1).
switch(config-if-eth1)# exit

switch(config)# interface eth2
switch(config-if-eth2)# no routing
switch(config-if-eth2)# exit
```

Repeat for each face you've cabled. Unused faces can stay routed — they won't interfere.

### 4. Give each client an IP and test

On client A:

```
/ > ifconfig eth0 10.0.0.1/24
```

On client B:

```
/ > ifconfig eth0 10.0.0.2/24
```

From client A:

```
/ > ping 10.0.0.2
```

You should see replies. On the switch, `show mac-address-table` should list both clients' MACs on the correct ports:

```
switch(config)# show mac-address-table
MAC age-time            : 300 seconds
Number of MAC addresses : 2

MAC Address         VLAN    Type     Port
-----------------------------------------
02:00:a1:b2:c3:d4   1       dynamic  0
02:00:e5:f6:07:08   1       dynamic  1
```

### 5. Verify flooding for unknown destinations

Run `clear mac-address-table dynamic`. The next ping will flood (no entry for the destination), a reply will come back, and both MACs will be re-learned. This is how a brand-new switch bootstraps its forwarding table.

---

## Getting started — VLANs

Goal: split the same 6-port switch into two broadcast domains (VLAN 10 "engineering", VLAN 20 "guest") and carry both over a trunk to a second switch.

### 1. Create the VLANs and activate them

```
switch(config)# vlan 10
switch(config-vlan-10)# name engineering
VLAN name set.
switch(config-vlan-10)# no shutdown
VLAN activated.
switch(config-vlan-10)# exit

switch(config)# vlan 20
switch(config-vlan-20)# name guest
switch(config-vlan-20)# no shutdown
switch(config-vlan-20)# exit
```

New VLANs are created in shutdown state — skip `no shutdown` and no traffic flows.

### 2. Assign access ports

Put two clients in VLAN 10 and one client in VLAN 20:

```
switch(config)# interface eth0
switch(config-if-eth0)# no routing
switch(config-if-eth0)# vlan access 10
Access VLAN set to 10.
switch(config-if-eth0)# exit

switch(config)# interface eth1
switch(config-if-eth1)# no routing
switch(config-if-eth1)# vlan access 10
switch(config-if-eth1)# exit

switch(config)# interface eth2
switch(config-if-eth2)# no routing
switch(config-if-eth2)# vlan access 20
switch(config-if-eth2)# exit
```

Clients on `eth0` and `eth1` can now ping each other (same VLAN) but not the client on `eth2` (different VLAN — broadcast-domain-isolated).

### 3. Configure a trunk to a second switch

Suppose `eth5` on this switch is cabled to `eth5` on a second switch, and you want both VLAN 10 and VLAN 20 to cross that trunk.

On this switch:

```
switch(config)# interface eth5
switch(config-if-eth5)# no routing
switch(config-if-eth5)# vlan trunk native 1
Trunk native VLAN set to 1.
switch(config-if-eth5)# vlan trunk allowed 10,20
Trunk allowed VLAN list set.
switch(config-if-eth5)# exit
```

Frames in VLAN 10 and VLAN 20 go out tagged; untagged frames are placed in VLAN 1 (the default native). Frames in any other VLAN are dropped at egress because the allowed list explicitly excludes them.

Repeat the same configuration on the other switch. Then on each switch, create any access ports in VLANs 10 and 20 that need to reach the far side.

### 4. Verify

```
switch(config)# show vlan
VLAN  Name             Status   Reason       Ports (untagged / tagged)
----  ---------------  -------  -----------  -----------------------------
1     default          up       ok           eth5 / -
10    engineering      up       ok           eth0,eth1 / eth5
20    guest            up       ok           eth2 / eth5
```

On a client in VLAN 10 on switch A, pinging a client in VLAN 10 on switch B should succeed. Pinging between VLAN 10 and VLAN 20 clients should not (no L3 routing between VLANs is currently supported — the computer has no inter-VLAN router).

### 5. Narrowing a trunk later

To remove VLAN 20 from the trunk without touching VLAN 10:

```
switch(config)# interface eth5
switch(config-if-eth5)# no vlan trunk allowed 20
Removed VLANs from trunk allowed list.
switch(config-if-eth5)# exit
```

### 6. Tagged native (fully-tagged trunk)

If you want the native VLAN to also carry a tag on the wire (no untagged traffic on the trunk at all), set the native with the `tag` keyword:

```
switch(config)# interface eth5
switch(config-if-eth5)# vlan trunk native 1 tag
Trunk native VLAN set to 1 (tagged).
```

Untagged ingress on this trunk is then dropped, and egress in VLAN 1 carries a `vid=1` tag.

---

## Defaults and limits

| Parameter                      | Default / limit                       |
|--------------------------------|---------------------------------------|
| Default VLAN                   | 1 (`default`), always active          |
| New VLAN state                 | Shutdown (except VLAN 1)              |
| Default port mode              | `routed` (L3, no L2 forwarding)       |
| Default access VLAN after `no routing` | 1                             |
| Default trunk allowed list     | `all`                                 |
| MAC aging time                 | 300 seconds                           |
| MAC aging time range           | 15–1 000 000 seconds                  |
| MAC table capacity             | 512 entries total                     |
| VLAN ID range                  | 1–4094                                |
| Frames per IRQ (switch handler)| 256 (coalesces with Java-side IRQ)    |
| Sub-shell input line length    | 256 bytes                             |
| Saved config file              | `/switch.cfg` (loaded on `switch` / `switch on`, written by `write memory`) |
| Autosave                       | None — explicit `write memory` required |

When the MAC table fills up, learning a new dynamic entry evicts the oldest dynamic entry. If all 512 slots are static, new entries (dynamic or static) are rejected.

---

## Notes and gotchas

- **Every port is L3 until you `no routing` it.** This is the most common mistake when setting up a basic switch — you cable everything, configure the VLANs, and nothing forwards because ports are still routed. Always run `no routing` first.
- **New VLANs are shutdown.** Except VLAN 1, every VLAN you create starts inactive. Run `no shutdown` inside the VLAN context or traffic in that VLAN is silently dropped.
- **Access ports drop tagged ingress.** If a client unexpectedly sends tagged frames to an access port, the switch drops them on the floor. Use `show vlan port <n>` to check what mode a port is in.
- **Trunk ingress honors the allowed list on BOTH directions.** A tagged frame for a disallowed VID is dropped at ingress, and an egress frame in a disallowed VID is dropped at egress.
- **Trunk with `allowed all` still requires VLANs to exist.** `allowed all` means "every VID that's in the database", not "every 12-bit number". VLANs you haven't created are not members.
- **Deleting a VLAN fails if it's still referenced by a port.** You'll get `% VLAN N is in use on port ethM`. Unconfigure the port first (`no vlan access`, `no vlan trunk native`, `no vlan trunk allowed`) and then retry.
- **VLAN 1 cannot be deleted or shut down.** It's always present and always active, matching AOS-CX semantics.
- **Config is saved only on explicit `write memory`.** There is no autosave. `switch off`, `Ctrl+T`, and OS shutdown all discard unsaved changes. See [Saving and loading configuration](#saving-and-loading-configuration) for the full story. Dynamic MAC entries are never saved — only static entries, VLANs, port modes, and the age-time.
- **The switch does not auto-start on boot.** Even if `/switch.cfg` exists, you still have to run `switch on` (or `switch`) after a restart. Loading is triggered by the start command, not by kernel init.
- **Switch mode replaces `IRQ_NETWORK`.** While the switch is running, management traffic for this computer (its own ARP, ping, SSH, etc.) still works — frames destined for one of this terminal's MACs (and broadcasts) are re-injected into the local NetStack. But the kernel's normal frame polling is suspended in favor of the switch handler.
- **`Ctrl+T` is always a safe bail-out.** Even if you're deep inside `config-if-eth3` and something breaks, Ctrl+T will cleanly leave switch mode, restore the IRQ handler, disable promiscuous on every port, and return you to the main shell.
- **Own MACs are not learned.** The switch tracks its own NICs' MAC addresses and skips learning them, so they never show up in `show mac-address-table` and never get mis-attributed to the wrong port after a flood.
- **Self-delivery still works for management.** A broadcast (or a frame addressed to one of this computer's own MACs) arriving on any ingress port is re-injected into the local NetStack (untagged, after stripping any 802.1Q tag) so this computer can still participate in the network in parallel with forwarding.

---

## Phase 3 — Logging

A rolling in-memory event buffer and a severity filter, matching the AOS-CX `logging` / `show logging` / `show events` / `clear logging` surface.

### Architecture

- 256-entry ring buffer, oldest entries are overwritten when full.
- Each entry carries a wall-clock millisecond timestamp, a severity (RFC-5424 `emergency` through `debug`), and a freeform message.
- A single severity filter gates both the buffer and the optional console mirror. Messages at or above (i.e. numerically lower than) the filter are kept.
- The internal CLI logger (`log_cli`) records things that are interesting for the operator even when the severity filter is tighter — for example VLAN create / delete. Those entries are stamped `info`.

### Top-level commands

| Command                         | Effect |
|---------------------------------|--------|
| `logging <ip>`                  | Add a remote syslog target (accepted; transmission is stubbed — see Cut Corners). |
| `no logging <ip>`               | Remove a remote syslog target. |
| `logging severity <level>`      | Set the severity filter. `level` ∈ {emergency, alert, critical, error, warning, notice, info, debug}. Default: `info`. |
| `logging console`               | Mirror log messages to the switch CLI shell. |
| `no logging console`            | Stop mirroring. |

### Show / clear

| Command                         | Effect |
|---------------------------------|--------|
| `show logging`                  | Print the filter, console state, and all buffered entries oldest-first. |
| `show logging -r`               | Same but newest-first. |
| `show events`                   | Alias of `show logging`. |
| `show events -r`                | Alias of `show logging -r`. |
| `clear logging`                 | Wipe all buffered entries. |

### Events emitted automatically

- `MAC learn`  — when a source MAC is first learned on a port/VLAN (`debug`).
- `MAC move`   — when a learned MAC moves to a different port (`notice`).
- `Port up/down` — state transition vs. previous IRQ (`notice`).
- `VLAN create / delete` (`info`).
- `STP root change`, `STP topology change` (`warning` / `notice`).
- `LACP bundle change`, `LACP port select/unselect` (`notice`).
- `LLDP neighbor add / remove` (`info`).

### Examples

```
switch(config)# logging severity debug
Logging severity set to debug.
switch(config)# vlan 42
switch(config-vlan-42)# exit
switch(config)# show logging
Logging severity: debug
Console: off
Entries: 1
[00:12:34.567]      info  VLAN 42 created
```

---

## Phase 4 — LLDP

Link Layer Discovery Protocol (IEEE 802.1AB). Sends multicast LLDPDUs to `01:80:c2:00:00:0e` with ethertype `0x88cc` and learns neighbors per port. Enabled by default on every interface with both transmit and receive on.

### Global knobs

| Command | Effect |
|---------|--------|
| `lldp` / `no lldp` | Enable / disable the LLDP agent globally. Default: enabled. |
| `lldp timer <secs>` | Transmit interval. Default 30. |
| `lldp holdtime <mult>` | TTL multiplier (TTL = timer × mult). Default 4. |
| `lldp reinit <secs>` | Delay before re-transmit after a disable→enable cycle. Default 2. |
| `lldp txdelay <secs>` | Minimum gap between consecutive LLDPDUs on a port. Default 2. |
| `lldp management-ipv4-address <ip>` | Value for the Management Address TLV. |
| `no lldp management-ipv4-address` | Clear. |
| `lldp select-tlv <name>` | Include an optional TLV. Names: `port-desc`, `sys-name`, `sys-desc`, `sys-caps`, `mgmt-addr`. |
| `no lldp select-tlv <name>` | Omit that optional TLV. |

### Per-interface

| Command | Effect |
|---------|--------|
| `lldp transmit` / `no lldp transmit` | Permit / forbid TX on this port. |
| `lldp receive` / `no lldp receive` | Permit / forbid processing RX frames on this port. |

### Show

| Command | Effect |
|---------|--------|
| `show lldp configuration` | Global settings + per-port tx/rx. |
| `show lldp neighbor-info` | Summary table of all learned neighbors. |
| `show lldp neighbor-info <port>` | Detailed view of one port's neighbor. |
| `show lldp statistics` | Per-port frame counters. |
| `show lldp tlv` | Which optional TLVs this switch will emit. |
| `show lldp local-device` | Locally-advertised system name / descr / mgmt address / chassis / capabilities. |

### Clear

| Command | Effect |
|---------|--------|
| `clear lldp neighbors` | Drop all neighbor state. |
| `clear lldp statistics` | Zero all port counters. |

---

## Phase 5 — Spanning Tree (single-instance CIST)

A simplified RSTP-style implementation with one spanning tree that covers all VLANs (CIST only — no MSTP per-instance topology). BPDUs are LLC-encapsulated (DSAP/SSAP `0x42`, control `0x03`) and sent to `01:80:c2:00:00:00`. STP is **disabled by default** — run `spanning-tree` at the top level to enable.

### Global knobs

| Command | Effect |
|---------|--------|
| `spanning-tree` / `no spanning-tree` | Enable / disable STP. |
| `spanning-tree priority <val>` | Bridge priority (0–61440 in steps of 4096). Default 32768. |
| `spanning-tree forward-delay <secs>` | Forward-delay timer. Default 15. Range 4–30. |
| `spanning-tree hello-time <secs>` | Hello interval. Default 2. Range 1–10. |
| `spanning-tree max-age <secs>` | Max-age timer. Default 20. Range 6–40. |
| `spanning-tree config-name <str>` | MST region name (cosmetic — single-instance build). |
| `spanning-tree config-revision <int>` | MST region revision (cosmetic). |

### Per-interface

| Command | Effect |
|---------|--------|
| `spanning-tree port-priority <val>` | Port priority (0–240 in steps of 16). Default 128. |
| `spanning-tree cost <val>` | Path cost override. Default 20000 (GigE-equivalent). |
| `spanning-tree admin-edge-port` / `no spanning-tree admin-edge-port` | Mark as edge. Edge ports skip listening/learning. |
| `spanning-tree bpdu-guard` / `no spanning-tree bpdu-guard` | Err-disable the port when a BPDU arrives. |
| `spanning-tree root-guard` / `no spanning-tree root-guard` | Block superior BPDUs from becoming root on this port. |
| `spanning-tree tcn-guard` / `no spanning-tree tcn-guard` | Ignore topology-change notifications from this port. |

### Show

`show spanning-tree` prints enabled state, bridge-id, root-id, root port, root cost, hello/FD/max-age, topology-change count, and a per-port table of `Role / State / Cost / Prio / PeerBridge`.

### State model (simplified)

Ports transition `Disabled → Blocking → Learning → Forwarding`, each non-edge step waiting `forward_delay_secs`. Edge ports jump straight to Forwarding. The root bridge designates every port as Designated; alternates and roots are re-elected on every BPDU ingress.

---

## Phase 6 — LACP and Link Aggregation Groups

IEEE 802.1AX link aggregation with LACP control. LAGs are created as synthetic `interface lag <id>` contexts (1–256). Physical ports join with `lag <id>` inside their own interface context.

### Creating and deleting LAGs

```
switch(config)# interface lag 1
switch(config-lag-1)# lacp mode active
switch(config-lag-1)# lacp rate fast
switch(config-lag-1)# hash l3
switch(config-lag-1)# fallback
switch(config-lag-1)# exit
switch(config)# interface eth0
switch(config-if-eth0)# lag 1
switch(config-if-eth0)# exit
switch(config)# no interface lag 1    # deletes the LAG and releases all members
```

### LAG-context commands

| Command | Effect |
|---------|--------|
| `lacp mode {active\|passive}` / `no lacp mode` | LACP activity for all members of this LAG. `no` → static LAG (no LACPDUs). |
| `lacp rate {fast\|slow}` | Partner timeout preference. `fast` = 1s tx / 3s timeout, `slow` = 30s / 90s. |
| `hash {l2\|l3\|l4-src-dst}` | Egress hashing policy when distributing to members. |
| `fallback` / `no fallback` | After ~90s without an LACP partner, bring up the LAG with a single selected member. |
| Same VLAN / port-mode commands as a physical interface. | These apply to the logical LAG (shared by all members). |

### Per-interface

| Command | Effect |
|---------|--------|
| `lag <id>` | Join this port to the given LAG (id must already exist). |
| `no lag` | Remove this port from its current LAG. |

### Show

| Command | Effect |
|---------|--------|
| `show lacp configuration` | System priority + system MAC. |
| `show lacp aggregates` | Each LAG with mode/rate/hash/members. |
| `show lacp interfaces` | Per-port LACP state: actor state bits, partner key, selected flag, TX/RX counters. |
| `show interface lag <id>` | Full detail for a single LAG. |

### Cut corners

See the [Cut corners](#cut-corners-phases-3-6) section below — LACP selection is simplified, marker protocol is not implemented, and churn detection is skipped.

---

## Cut corners (Phases 3–6)

These are deliberate simplifications. They are documented so you can tell the difference between "bug" and "deliberate stub":

**Logging (Phase 3)**
- *Remote syslog is a stub.* `logging <ip>` is accepted and persists across reboots, but nothing is actually transmitted — the simulator's NetStack does not expose a simple `udp_send_from(ip, port, buf)` primitive, so the transport is not wired. Console mirror and the in-memory ring buffer both work normally.

**LLDP (Phase 4)**
- *No dot1 / dot3 / med extended TLV families.* Only the mandatory trio (Chassis, Port, TTL) plus Port Description, System Name, System Description, System Capabilities, and Management Address are emitted.
- *Receive-side TLV parsing is limited to those same TLVs* — unknown TLVs are skipped silently.
- *No LLDP-MED.* No power-over-ethernet, location, or inventory extensions.

**Spanning tree (Phase 5)**
- *CIST-only.* One spanning-tree instance covers every VLAN. There is no MSTP region mapping, no per-MSTI path cost, and `config-name` / `config-revision` are stored for display but do not affect any election.
- *No proposal / agreement handshake.* RSTP's fast-transition mechanism is skipped. Non-edge ports always wait `forward-delay` to move `Blocking → Learning → Forwarding`.
- *Simplified topology-change.* TC is signaled as a counter increment plus a log event; we do not flush the MAC table or propagate TC BPDUs. (In this network the MAC age-time (300s) does the job eventually.)
- *No TCN handshake on edge port flaps.*
- *BPDU guard / root guard / tcn guard* toggle internal flags but the guards err-disable silently — the port does not produce a syslog trap other than a log event.

**LACP / LAG (Phase 6)**
- *Partner-key-based selection.* The full IEEE "Selection Logic" is replaced by the rule: a member is selected iff the partner's `key` equals our local `lag_id` (or the LAG is configured for fallback and the timer has expired).
- *No churn detector.* Ports don't track oscillation; we trust the partner-aging + timeout bits.
- *No marker protocol.* Marker request/response frames are not generated or consumed, so LACP-driven graceful drain is not available.
- *Hash is advisory.* The hash policy setting is stored and shown, but the built-in forwarding path picks an egress member by `src_mac XOR dst_mac` regardless of whether you asked for `l3` or `l4-src-dst`.

**Timers (all four phases)**
- *No host-driven tick source.* Periodic work (LLDP TX, STP hello, LACP TX, LLDP/neighbor aging, STP forward-delay timers, LACP partner aging) runs as a piggyback inside the `IRQ_NETWORK` path. That means a switch that receives **no** frames for a long period makes no forward progress on its timers. In practice, ambient LLDP/STP/LACP frames between neighbors keep things moving. Solo switches won't time-out neighbors or transition forward-delay until one frame arrives.
