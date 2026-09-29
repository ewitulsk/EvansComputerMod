# Switch Instructions

Any terminal computer can act as a managed L2 switch. It runs 802.1Q VLANs, a
MAC table, Rapid Spanning Tree, LACP link aggregation, LLDP and VLAN
management interfaces (SVIs), all configured through an Aruba AOS-CX-style CLI.
The switch is a background **kernel service** built from the `ecm-bridge` crate
(`rust/crates/ecm-bridge`).

Part 1 is the user guide. Part 2 describes the design, the timers and limits,
and the known differences from real AOS-CX and IEEE 802.1.

---

# Part 1 — User guide

## 1. Starting, entering and stopping

| Shell command | Effect |
|---|---|
| `switch on` | Start the switch service in the background (the saved `/switch.cfg` is replayed on start). You stay at the shell. |
| `switch` | Enter the switch CLI (`switch(config)#`), starting the service if it isn't running. |
| `switch off` | Stop the switch service. |

Things to know about the service:

- The service is independent of the shell. Pressing **Ctrl+T to stop a
  program never stops the switch**, and a detached switch keeps running when
  you leave its CLI.
- Inside the CLI, `on` *detaches* the session. From then on, leaving the CLI
  keeps the switch running. This is the same state `switch on` gives you.
- Leaving the CLI (top-level `exit`, or Ctrl+T while in the CLI) of a session
  that was never detached stops the switch.
- `exit` at the top-level context leaves the CLI. In a sub-context, `exit`
  (and `end`/`quit` anywhere) returns to `switch(config)#`.
- `off` inside the CLI is not a command. It only prints a hint: use `exit`,
  then `switch off`.

## 2. Cabling a switch in-game

Every non-screen face of the terminal, and every free face of an attached
`InterfaceBlock` chain, is a network port, named `eth0`, `eth1` and so on. For
a horizontally-facing terminal with nothing attached:

| Screen faces | eth0 | eth1 | eth2 | eth3 | eth4 |
|---|---|---|---|---|---|
| North | Down | Up | South (back) | West | East |
| South | Down | Up | North (back) | West | East |
| East  | Down | Up | North | South | West (back) |
| West  | Down | Up | North | South | East (back) |

`InterfaceBlock` ports are numbered after the built-in faces, in BFS order.
Adding or removing blocks renumbers them. To identify a face, cable a host to
it, send any traffic (for example a ping), then check
`show mac-address-table`.

**Every port starts *routed*.** A routed port does not switch: its frames go to
the computer's own network stack. Run `no routing` on a port (making it access
VLAN 1), or configure it as access or trunk, before it forwards anything.
Frames on routed ports never reach the switch.

## 3. Contexts and prompts

| Prompt | Entered with |
|---|---|
| `switch(config)# ` | top level |
| `switch(config-vlan-N)# ` | `vlan N` |
| `switch(config-if-ethN)# ` | `interface ethN` or `interface N` |
| `switch(config-lag-N)# ` | `interface lag N` (or `interface lagN`) |
| `switch(config-if-vlan-N)# ` | `interface vlan N` (or `interface vlanN`) |

These commands work in every context: `show …`, `write memory`, `help` / `?`
(context-aware), `exit`, `end` and `quit`.

Errors and warnings start with `% `. Blank lines and lines starting with `#`
or `!` are ignored.

## 4. Command reference

### 4.1 Top level

| Command | Notes |
|---|---|
| `mac-address-table age-time <15-1000000>` / `no mac-address-table age-time` | Dynamic-entry ageing in seconds. The default is 300. |
| `static-mac <mac> vlan <1-4094> port <N\|ethN\|lagN>` / `no static-mac …` | The MAC may be written `aa:bb:…`, `aa-bb-…` or `aabb.ccdd.eeff`. Static entries never age and win over learning. `no` needs an exact match. |
| `vlan <id>` | Creates the VLAN (**shut down**) if absent and enters its context. |
| `vlan <a>-<b>` | Bulk-creates a range: `Created 11 VLANs (range 100-110).` |
| `no vlan <id>` | Refused for VLAN 1, for a VLAN still used by a port or LAG, and for a VLAN that has an SVI. Removes the VLAN's MAC entries. |
| `interface ethN` \| `interface lag <1-256>` \| `interface vlan <id>` | `interface lag` creates the LAG. `interface vlan` needs the VLAN to exist. |
| `no interface lag <id>` | Deletes the LAG and releases its members. |
| `no interface vlan <id>` | Removes the SVI. |
| `logging <ip>` / `no logging <ip>` | Adds or removes a remote collector (at most 8). This only records configuration; the switch sends nothing itself. |
| `logging severity <emergency…debug>` / `no logging severity` | Filter for the log buffer. The default is `info`. |
| `[no] logging console` | Asks the kernel to print log events on the console. |
| `[no] lldp` | Enables or disables LLDP globally (enabled by default). |
| `lldp timer <5-32768>` | Default 30. |
| `lldp holdtime <2-10>` | Default 4. |
| `lldp reinit <1-10>` | Default 2. |
| `lldp txdelay <1-8192>` | Default 2. A `no` form resets each of these four. |
| `lldp management-ipv4-address <ip>` / `no …` | Without it, LLDP advertises the first SVI address. |
| `[no] lldp select-tlv <port-desc\|sys-name\|sys-desc\|sys-caps\|mgmt-addr>` | Optional TLVs, all on by default. |
| `[no] spanning-tree` | Enables or disables RSTP. **Disabled by default.** |
| `spanning-tree priority <0-61440, step 4096>` | Default 32768. |
| `spanning-tree forward-delay <4-30>` | Default 15. |
| `spanning-tree hello-time <1-10>` | Default 2. |
| `spanning-tree max-age <6-40>` | Default 20. |
| `spanning-tree config-name <text>` / `config-revision <n>` | Stored and shown only; they have no effect. |
| `on` | Detaches the session (the switch keeps running after exit). |
| `write memory` | Saves the running config to `/switch.cfg`. |
| `clear …` | See [4.7](#47-clear). |

### 4.2 VLAN context

| Command | Notes |
|---|---|
| `name <text>` / `no name` | |
| `description <text>` / `no description` | |
| `no shutdown` / `shutdown` | New VLANs are shut down, and traffic in a shut-down VLAN is dropped. VLAN 1 is always active. |

```
switch(config)# vlan 10
switch(config-vlan-10)# name engineering
VLAN name set.
switch(config-vlan-10)# no shutdown
VLAN activated.
```

### 4.3 Interface context (`ethN`)

| Command | Notes |
|---|---|
| `no routing` | Converts the port to L2, access VLAN 1. |
| `routing` | Converts it back to L3. |
| `[no] shutdown` | Administrative down/up. `no shutdown` also clears a BPDU-guard err-disable. |
| `vlan access <id>` / `no vlan access` | A missing VLAN is created shut down, with a warning. |
| `vlan trunk native <id> [tag]` / `no vlan trunk native` | Makes the port a trunk with allowed list `all`. `tag` tags the native VLAN too, and untagged ingress is then dropped. `no` resets the native VLAN to 1. |
| `vlan trunk allowed <list\|all>` / `no vlan trunk allowed <list>` | Lists look like `10,20,30-40`. The port must already be a trunk. `no` subtracts; from `all` it first expands the current VLAN database. |
| `[no] lldp transmit` / `[no] lldp receive` | |
| `[no] spanning-tree` | Per-port STP. With `no`, the port forwards without STP and ignores BPDUs. |
| `spanning-tree port-priority <0-240, step 16>` / `cost <n>` | Defaults are 128 and 20000. A `no` form resets them. |
| `[no] spanning-tree admin-edge-port \| bpdu-guard \| root-guard \| tcn-guard` | |
| `lag <id>` / `no lag` | Joins a LAG (it must exist; at most 8 members). Joining a second LAG leaves the first. |

VLAN commands need an L2 port: `% Run 'no routing' first to convert this port
to L2`. While a port is a LAG member, the **LAG's** VLAN and STP config apply
and the port's own settings are ignored.

```
switch(config)# interface eth5
switch(config-if-eth5)# no routing
Port converted to L2 (access vlan 1).
switch(config-if-eth5)# vlan trunk native 1
Trunk native VLAN set to 1.
switch(config-if-eth5)# vlan trunk allowed 10,20
Trunk allowed VLAN list set.
```

### 4.4 LAG context (`interface lag N`)

The LAG context accepts the same `no routing` / `routing` / `[no] shutdown` /
`vlan …` / `spanning-tree …` commands as a physical port, plus these:

| Command | Notes |
|---|---|
| `lacp mode active\|passive` / `no lacp mode` | `no` makes it a static LAG (no LACPDUs). The default is static. |
| `lacp rate fast\|slow` / `no lacp rate` | Fast is 1 s transmit / 3 s timeout; slow is 30 s / 90 s. The default is slow. |
| `[no] lacp fallback` (or `[no] fallback`) | With no LACP partner on any member, the lowest link-up member forwards alone. |
| `hash l2-src-dst\|l3-src-dst\|l4-src-dst` / `no hash` | Egress member selection. The default is `l3-src-dst`. |

```
switch(config)# interface lag 1
switch(config-lag-1)# no routing
switch(config-lag-1)# lacp mode active
switch(config-lag-1)# lacp rate fast
switch(config-lag-1)# exit
switch(config)# interface eth0
switch(config-if-eth0)# lag 1
eth0 joined LAG 1.
```

### 4.5 SVI context (`interface vlan N`) — managing the switch

An SVI gives the switch its own IP address in a VLAN. The kernel creates a stack
interface (using the switch's bridge MAC) for each SVI. This is how you ping or
SSH to the switch through switched ports, including over tagged trunks.

| Command | Notes |
|---|---|
| `ip address <a.b.c.d/nn>` or `ip address <a.b.c.d> <mask>` | Creates or readdresses the SVI. |
| `no ip address` | Removes the SVI. |

```
switch(config)# vlan 10
switch(config-vlan-10)# no shutdown
switch(config-vlan-10)# exit
switch(config)# interface vlan 10
switch(config-if-vlan-10)# ip address 10.0.10.1/24
Interface vlan 10 address 10.0.10.1/24.
```

### 4.6 show

| Command | Output |
|---|---|
| `show mac-address-table` | The whole table, with age-time and count. |
| `show mac-address-table [dynamic\|static] [port <p>] [vlan <v>] [address <mac>]` | A filtered table. |
| `show mac-address-table count [dynamic] [filters]` | `Number of entries: N` |
| `show mac-address-table mac-move [filters]` | Entries that moved: current and previous port, move count, time of last move. |
| `show vlan` / `show vlan <id>` / `show vlan summary` / `show vlan port <p\|lagN>` | |
| `show interface [brief]` | Per port: link, admin state, mode, LAG, forwarding. |
| `show interface lag <id>` | |
| `show interface vlan` | |
| `show spanning-tree` | |
| `show lacp configuration\|aggregates\|interfaces` | |
| `show lldp configuration\|neighbor-info [ethN]\|statistics\|tlv\|local-device` | |
| `show logging [-r] [severity <lvl>]` (alias `show events`) | |
| `show running-config` | |

Sample output. The port column shows `N` for `ethN`, `lagN` for a LAG, and
`cpu` for the switch itself:

```
switch(config)# show mac-address-table
MAC age-time            : 300 seconds
Number of MAC addresses : 3

MAC Address         VLAN    Type     Port
-----------------------------------------
02:00:a1:b2:c3:d4   1       dynamic  0
02:00:e5:f6:07:08   10      static   3
02:00:00:00:77:01   10      dynamic  lag1

switch(config)# show vlan
VLAN  Name             Status   Reason       Ports (untagged / tagged)
----  ---------------  -------  -----------  -----------------------------
1     default          up       ok           eth0 / eth5
10    engineering      up       ok           eth3 / eth5,lag1
20    guest            down     admin-down   - / -

switch(config)# show spanning-tree
Spanning Tree: enabled (RSTP, CIST)
  Bridge ID  : 8000.02b000.000002
  Root ID    : 8000.02b000.000001
  Root port  : eth0
  Root cost  : 20000
  Hello/FD/MaxAge: 2/15/20
  Topo changes: 1 (last at 00:00:31.010)

Port    Role         State        Cost       Prio  Flags      Designated-Bridge
--------------------------------------------------------------------------------
eth0    Root         Forwarding   20000      128   -          8000.02b000.000001
eth1    Alternate    Discarding   20000      128   -          8000.02b000.000001
eth2    Designated   Forwarding   20000      128   -          8000.02b000.000002

switch(config)# show lacp aggregates
LAG   Members                 Mode      Rate   Up  Distributing
1     eth0,eth1               active    fast   y   eth0,eth1
```

The STP flags are `ERR` (err-disabled), `RINC` (root-guard inconsistent),
`Edge` (operational edge port) and `NoSTP` (STP off for the port).
`show lacp interfaces` prints the actor and partner state bits as letters:
`A` active, `F` fast, `G` aggregatable, `S` sync, `C` collecting,
`D` distributing, `d` defaulted, `e` expired.

### 4.7 clear

| Command | Output |
|---|---|
| `clear mac-address-table dynamic [vlan <id> \| port <p> \| address <mac>]` | `Cleared N dynamic entries…` |
| `clear logging` (or `clear events`) | Empties the log buffer. |
| `clear lldp neighbors` / `clear lldp statistics` | |

## 5. Saving and loading: `/switch.cfg`

`write memory` writes the running config (the same text as
`show running-config`) to `/switch.cfg`. There is no autosave.

When the service starts, the file is replayed line by line through a single CLI
session. Errors are reported and skipped, and the load ends with
`Loaded switch.cfg (N commands applied).`

The file is plain CLI text. Each block is closed with an indented ` exit` that
only returns to the top level; the file never contains a top-level `exit`.
Only non-default settings are written:

```
# switch.cfg - generated by 'write memory'
mac-address-table age-time 120
spanning-tree
vlan 10
 name engineering
 no shutdown
 exit
interface lag 1
 no routing
 vlan trunk native 1
 lacp mode active
 lacp rate fast
 exit
interface eth0
 lag 1
 exit
interface eth2
 no routing
 vlan access 10
 spanning-tree admin-edge-port
 exit
interface vlan 10
 ip address 10.0.10.1/24
 exit
static-mac 02:00:00:00:00:10 vlan 10 port 2
```

Dynamic MAC entries and session state are not saved. You can edit the file by
hand, and files written by the previous switch implementation still load.

## 6. Quick start: a 3-port switch with VLANs

```
switch(config)# vlan 10
switch(config-vlan-10)# no shutdown
switch(config-vlan-10)# exit
switch(config)# interface eth0
switch(config-if-eth0)# vlan access 10
% Run 'no routing' first to convert this port to L2
switch(config-if-eth0)# no routing
switch(config-if-eth0)# vlan access 10
switch(config-if-eth0)# exit
switch(config)# interface eth1
switch(config-if-eth1)# no routing
switch(config-if-eth1)# vlan access 10
switch(config-if-eth1)# exit
switch(config)# on
switch(config)# write memory
switch(config)# exit
```

Hosts on `eth0` and `eth1` can now reach each other in VLAN 10. If other
switches may be cabled into loops, also enable `spanning-tree`.

---

# Part 2 — Design

## 7. Architecture

`ecm-bridge` is a pure, sans-IO crate: it has no host calls, no global state
and no clock reads. The kernel feeds it frames, link changes and time
(`handle_frame`, `send_local`, `set_link`, `poll(now)`), then drains its
queued outputs. There are three kinds:

- `Tx` — a frame to send on a port;
- `Local` — an untagged frame for the SVI of a VLAN;
- `Log` — an event.

`poll` returns the next timer deadline. The kernel hands a port's frames to the
bridge only when the port is L2 (`is_l2_port`); routed ports go straight to the
host stack.

### Port hierarchy and forwarding gate

```
physical port ethN ──(optional)──► LAG lagN ──► bridge port ──► VLAN / FDB / STP
                                                     ▲
                                  internal CPU port (SVIs, send_local)
```

- **Bridge ports.** A bridge port is either a standalone L2 physical port or an
  L2 LAG. STP, VLAN membership, learning and flooding all operate on bridge
  ports. LAG members inherit the LAG's VLAN and STP config.
- **Forwarding gate.** A bridge port forwards data only when all of these hold:
  - it is operationally up: link up, admin up, not err-disabled, and for a
    LAG, at least one member collecting/distributing;
  - STP is in the Forwarding state (or STP is off globally or for that port).
- **Learning.** Addresses are learned in the Learning and Forwarding states.
- **LAG members.** A member only accepts data frames while it is
  collecting/distributing. Egress picks one distributing member per frame by
  hash.
- **Link-local frames.** Frames to `01:80:c2:00:00:00`–`0f`, and all
  slow-protocol frames (EtherType 0x8809), are consumed and never forwarded.
- **VLAN semantics.**
  - Access ports drop tagged ingress; priority-tagged frames (VID 0) count as
    untagged.
  - Trunks accept untagged frames into the native VLAN (unless `tag`) and
    tagged frames for allowed VIDs.
  - Egress is untagged for access ports and the untagged native VLAN, and
    tagged otherwise. PCP is preserved.
  - An allowed list of `all` means every VLAN in the database.
- **SVIs.**
  - Frames to the switch's MAC in a VLAN with an SVI go to that SVI, untagged.
    In a VLAN without an SVI they are dropped.
  - Broadcast and multicast frames in a VLAN with an SVI are both delivered
    locally and flooded.
  - Frames the switch's stack sends (`send_local`) enter as if from a CPU port
    that is an untagged member of the VLAN. The source MAC is learned on
    `cpu`, then the frame is forwarded or flooded and tagged per egress port.
- **Looped-back frames.** Data frames carrying one of the switch's own MACs as
  source are dropped.

### FDB

- Keyed by (VLAN, MAC). **1024 entries**; when full, the least-recently-seen
  dynamic entry is evicted. A new static entry is refused only if every entry
  is static.
- Dynamic entries age after the configured age-time.
- MAC moves are counted and logged at `notice`.
- A bridge port's entries are flushed when it goes down, stops forwarding or
  changes mode.

### RSTP (802.1D-2004 clause 17, single CIST)

- **Priority vectors.** Priority vectors are compared as (root, cost,
  designated bridge, designated port), with port IDs as tie-breaks. The port
  ID is the priority's high nibble plus the port number (`ethN` = N+1,
  `lagN` = 0x400|N).
- **Roles.** Root, Designated, Alternate, Backup and Disabled.
- **States.** Discarding, Learning and Forwarding. A port that becomes Root or
  Designated waits forward-delay in Discarding and forward-delay in Learning;
  this is the timer fallback. Edge ports (`admin-edge-port`) go straight to
  Forwarding and lose edge status when a BPDU arrives.
- **Received information.**
  - A message is accepted if it is superior, or comes from the same designated
    bridge and port.
  - It expires after 3 × the received hello time.
  - Message age increments by 1 s per hop, and information whose message
    age + 1 exceeds max-age is discarded.
- **Transmission.**
  - Designated ports send every hello time, and immediately on change.
  - Root ports transmit only while a topology change is active.
  - At most 6 BPDUs per port per second.
- **Topology change.**
  - A non-edge port reaching Forwarding, or a TC received on a Root or
    Designated port, sets the TC flag on the other ports for 2 × hello.
  - It flushes dynamic FDB entries on all other ports, and ages entries with
    forward-delay for the next forward-delay period.
- **STP compatibility.** Receiving a v0 config BPDU or a TCN switches that port
  to v0 BPDUs. The root port then signals TC with TCNs, and a Designated port
  acknowledges received TCNs with the TCA flag.
- **Guards.**
  - **BPDU guard:** a BPDU err-disables the port until `no shutdown` or a link
    flap. It applies even with STP disabled globally.
  - **Root guard:** the port can never become Root. While superior BPDUs
    arrive it is Alternate/Discarding and marked root-inconsistent (logged);
    it recovers only after that information expires.
  - **TCN guard:** TC flags and TCNs received on the port are ignored.

### LACP (802.1AX)

- **Actor identity.** System priority 32768 with the switch MAC. The key is
  the LAG id, and the port number is N+1. Keys need not match the partner's.
- **Receive.** Current / Expired / Defaulted. The timeout is 3 s for
  `lacp rate fast` and 90 s for slow; an expired partner gets one more 3 s
  before the port is defaulted.
- **Transmit.**
  - Periodic transmission uses 1 s or 30 s, following the partner's timeout
    bit.
  - Nothing is sent when both ends are passive (a passive–passive LAG never
    forms).
  - At most 3 LACPDUs per port per second.
  - A LACPDU frame is 124 octets: 110 octets of payload (128 on the wire with
    the FCS).
- **Selection.**
  - A member is eligible when its partner is aggregatable, is not the switch
    itself, and is Current or Expired.
  - Members aggregate only with the same partner (system priority, system,
    key). Members cabled to different switches do not bundle: one partner
    group is kept (sticky, otherwise the largest group).
- **Mux (coupled control).** Detached → Waiting (2 s) → Attached (Sync) →
  Collecting+Distributing once the partner reports Sync with a correct view of
  this switch.
- **Static LAG.** Every link-up member distributes.
- **Fallback.** When every running member is defaulted, the lowest link-up
  member distributes alone (about 3 s after link-up).
- **Hashing.**
  - `l2-src-dst`: MACs.
  - `l3-src-dst`: IPv4 addresses, or ARP sender/target addresses; falls back
    to L2.
  - `l4-src-dst`: adds TCP/UDP ports for unfragmented IPv4; falls back to L3.
  - 802.1Q tags are skipped before hashing.

### LLDP (802.1AB)

- **Transmit.** LLDPDUs are sent to `01:80:c2:00:00:0e` from each port's MAC,
  on link-up L2 ports with transmit enabled, every `timer` seconds.
  - The TTL is timer × holdtime + 1 (121 s by default).
  - The first frame goes out immediately at link-up.
  - Local changes trigger a send, spaced at least `txdelay` apart.
  - Disabling transmit on an up link sends a shutdown LLDPDU (TTL 0) and
    blocks transmission for `reinit` seconds.
- **TLVs.** The chassis ID is the switch MAC; the port ID is the interface
  name `ethN`. Optional TLVs: port description, system name, system
  description, capabilities (bridge), and the management IPv4 address with
  ifIndex N+1.
- **Receive.**
  - Only frames to the nearest-bridge address are consumed.
  - The first three TLVs must be Chassis ID, Port ID and TTL, in that order.
    Malformed frames are counted as errors.
  - TTL 0 deletes the neighbour, and a neighbour expires after its TTL.
  - Link-down purges the port's neighbours.
- **Scope.** LLDP runs only on L2 ports.

### Logging

- Events pass a severity filter, then go into a **256-entry** ring (oldest
  dropped) and out as `Log` outputs.
- Logged events: MAC learn (`debug`), MAC move, port and LAG up/down, VLAN and
  LAG create/delete, STP root, state and topology changes, guard trips,
  LACP state changes, and LLDP neighbour add/remove/age.

### Limits

| Item | Limit |
|---|---|
| FDB | 1024 entries |
| LLDP neighbours | 64 per port (new ones dropped when full, counted as `TooMany`) |
| Log ring | 256 entries |
| Remote log collectors | 8 |
| LAGs | ids 1–256, 8 members each |
| VLAN ids | 1–4094 |
| Physical ports | 256 |
| Queued outputs | 8192 (overflow dropped and counted) |
| CLI line length | 1024 characters |

## 8. Known deviations from AOS-CX and IEEE 802.1

These are deliberate simplifications:

- **RSTP proposal/agreement is not implemented.** Every non-edge port takes
  2 × forward-delay (30 s by default) to forward. There is no dispute
  mechanism and no auto-edge detection.
- **One spanning tree (CIST) covers all VLANs.** There is no MSTP;
  `config-name` and `config-revision` have no effect.
- **The mcheck/migration timer is simplified.** A port uses v0 BPDUs after
  receiving one and switches back to RSTP after receiving an RST BPDU.
- **LACP has no churn detection and no marker protocol.** Marker PDUs are
  dropped. System and port priorities are fixed at 32768, and there is no
  per-port `lacp` tuning (the command is accepted as a no-op).
- **LLDP covers nearest-bridge scope only.** There are no 802.1/802.3/MED
  extension TLVs, and unknown TLVs are skipped.
- **Remote syslog and the console mirror are configuration only.** The switch
  itself sends nothing. `Log` events leave the bridge only if they pass the
  severity filter.
- **There is no inter-VLAN routing.** SVIs are management interfaces for the
  switch's own stack.
- **Real AOS-CX creates VLANs active; here they start shut down.** VLAN 1 can
  be neither deleted nor shut down.
