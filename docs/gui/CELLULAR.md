# Cellular, as built (P7)

LTE and mmWave cellular data for computers: modem modules, SIM cards, player-built
cell towers and carriers. This is the as-built design of [ROADMAP §P7](ROADMAP.md#p7--cellular);
the phone (P5) adds the SIM tray, the Settings pane and the status-bar bars on top of it.

## What players get

| Thing | ID | What it does |
| --- | --- | --- |
| Cell Tower | `cell_tower` | Base station at the foot of a mast. Cable it (RJ45 underneath or on a side, not the door or top). Right-click: carrier screen (owner, op, creative), anyone else gets the status in chat. Right-click with a blank SIM: issues a SIM of its carrier. Lamp lit while on the air and cabled. |
| LTE Sector Antenna | `lte_antenna` | 14 dBi 65° x 20° sector panel (3GPP TR 36.814 pattern, 3° downtilt, 25 dB front-to-back). |
| mmWave Array Antenna | `mmwave_antenna` | 20 dBi beam-steered array (±60° x ±45° scan, cos^1.5 scan loss). |
| LTE Modem, mmWave Modem | `lte_modem`, `mmwave_modem` | Module-bay modems: `wwan0` for the kernel. |
| SIM Card | `sim_card` | Blank until a tower issues it; then carries the `evanscomputermod:sim_card` component (carrier, IMSI, 128-bit key). |

Building a tower: place the Cell Tower, stack mast blocks on it (`#evanscomputermod:cell_masts`:
Lattice Mast, Antenna Rod, iron bars, chains), click antennas onto the sides of mast blocks (they
point away from the mast) or on top. Each technology uses its highest antenna as its position and
the union of its panels' beams as its pattern; LTE also loses 0.04 dB per metre of feeder down the
mast. A tower serves LTE and/or mmWave depending on which antennas it has.

Using it: put an expansion card and a modem in a computer's bay, right-click the modem's bay with an
issued SIM (the SIM it held pops out), then configure `wwan0` like `eth0`
(`ifconfig wwan0 dhcp`, `dhclient wwan0`, or a static address; `iface wwan0 ...` in network.cfg is
applied when the interface appears). `cell` shows the modem; `cell scan` lists towers heard.
Sneak-right-click the air holding a loose modem to take its SIM out.

Carriers are players: whoever first names a carrier at a tower owns it. A tower placed by Steve starts
on `steve-mobile`. The carrier screen sets the carrier and the LTE / mmWave channels, issues SIMs
(consumes a blank SIM from the inventory), and lists the carrier's SIMs: which is attached to this
tower on which technology, its uplink signal and CQI, bytes used and the data limit (click a row,
type MB, Set limit; empty = none). Ops have `/ecm cell tower <pos>`, `/ecm cell carriers`,
`/ecm cell carrier <id> [quota <imsi> <bytes|-1>]`.

## PHY profiles (`radio/cellular/CellTech`, `CellPhy`)

Both run on the existing `RadioMedium` (WorldRadioMedium in game), so walls, terrain, height,
interference and fading come from the existing path model; nothing about range is special-cased.

| | LTE | mmWave (NR FR2) |
| --- | --- | --- |
| Band | UHF | MICROWAVE |
| Channels (DL / UL centre) | 1-6: 791-841 / 703-753 MHz, 10 MHz | 1-4: 28.05-28.35 / 27.55-27.85 GHz, 100 MHz |
| Duplex | FDD; uplink split into 4 slices (one per device, slice 0 also random access) | same |
| Tower / device power | 43 dBm (20 W) / 23 dBm | 30 dBm / 23 dBm |
| Tower antenna | 14 dBi sector panel | 20 dBi steered array |
| Device antenna | half-wave whip (2.15 dBi) | 8 dBi array, steered at the tower |
| Noise figure tower / device | 3 / 7 dB | 5 / 9 dB |
| Modulation | CQI 1-15 (QPSK-64QAM, TS 36.213) | CQI 1-18 (adds 256QAM, TS 38.214) |
| Peak rate | 41.7 Mbit/s down, 10.4 up | 607 Mbit/s down, 152 up |
| Behaviour | λ 37 cm: through walls and a few blocks of earth, diffracts over hills | λ 1 cm: one block of earth, stone or water stops it; line of sight only |

Frames are `Emission.frame`s with modulation `CELL-CQI<n>`; `BasicRadioMedium.requiredSinrDb` maps
them to the CQI's 10 % BLER SINR, so packet errors follow the same table link adaptation uses.
Link adaptation keeps a 3 dB margin: downlink from the device's SINR reports (in every uplink frame
and a 1 s keep-alive), uplink by reciprocity from the beacon's RSSI and the tower's advertised power.
Airtime is bits / rate plus one OFDM symbol. Nothing sits in the 1-2.4 GHz or 2.5-5.1 GHz gaps.

## How a modem appears to the kernel

The recommended "extra NIC through `net_get_interface_count`" could not host it cleanly: the
kernel reads the NIC count once at boot (`Net::new`, `phys`), indexes stack interfaces by NIC port,
and the host fixes `networkMacs` when the computer starts, so a modem clicked in at runtime would
need a reboot and would renumber every Interface Block NIC after it. Instead it follows the Wi-Fi
precedent with the smallest ABI that allows hot-plug (abi/host-abi.toml, `cell_*`):

| Function | Meaning |
| --- | --- |
| `cell_present() -> i32` | modems in the bays (modem 0 is `wwan0`) |
| `cell_get_mac(buf6) -> i32` | the modem's MAC (6) or -1 |
| `cell_link() -> i32` | 1 attached (carrier), 0 not, -1 no modem |
| `cell_tx_frame(frame, len) -> i32` | Ethernet frame to the serving tower: 0, -1, -2 queue full |
| `cell_rx_frame(buf, cap) -> i32` | next Ethernet frame: length, 0 none, -1 no modem |

Received frames raise IRQ 3 (network), so `Net::rx` drains `wwan0` with the NICs. The kernel
(`net/cell.rs`) samples presence, MAC and carrier every 250 ms: `wwan0` is added when a modem
appears (and re-created with the new MAC if a different modem is fitted), its carrier follows
`cell_link`, and `network.cfg` lines for `wwan0` are applied when it appears. To the IP stack it is a
plain Ethernet interface, so DHCP, ARP, IP and TCP needed no stack changes. Simulator stubs report
no modem. Java: `ComputerInstance.createCellFunctions` finds the first live `CellRadio` among the
computer's peripherals (like `WifiRadio`).

## How towers bridge, attach and hand over

`TowerLink` (pure, one per technology per tower) is the radio and a layer-2 bridge, exactly like
the Access Point: the tower's RJ45 port is a promiscuous bridge port (`registerBridgePort`) on its
cable segment; uplink Ethernet frames go out through `transmitFromPort` with the device's own MAC
(which the hub learns behind the tower), cable frames for an attached device's MAC (or broadcast)
go down the air. Device-to-device frames on one tower are relayed directly.

Air protocol (`CellFrame`): beacons every 100 ms (carrier, cell key, power, load; staggered within
the tick per tower), ATTACH / ACCEPT / REJECT, DATA_UL (with a downlink SINR report) / DATA_DL,
REPORT (keep-alive), DETACH, RELEASE. Authentication is a challenge proof both ways keyed by the
SIM's secret (HMAC-SHA256, 8 bytes): the device proves it holds the key, the accept proves the
carrier knows it. The `CarrierRegistry` (saved data: carriers, SIMs, keys, quotas, usage) is the
core network every tower asks; it also tracks which tower serves which IMSI.

`ModemLink` (pure) searches the plan's downlink channels (300 ms dwell each), and at the end of a
dwell attaches to the strongest tower of its SIM's carrier. Towers of other carriers are listed by
`cell scan` but never asked, so a wrong-carrier SIM never attaches (and a tower would refuse it).
While connected it measures the beacons of same-carrier towers on its channel; a neighbour 3 dB
stronger for 320 ms gets an attach request with the handover flag while the old link stays up
(make before break). When the new tower accepts, the registry releases the device from the old
tower. The MAC never changes, so ARP entries, addresses and TCP connections survive; the next uplink
frame re-teaches the hub's forwarding table. Losing the serving tower's beacons for 1 s drops back to
search (re-establishment on the best tower, also without changing the MAC). Quotas: every byte up or
down is counted on the SIM; at the limit the tower releases the device and refuses it until the
carrier raises the limit.

## Interfaces for the phone (P5)

* `radio.cellular.SimProvider { SimCard sim(); }`: implement it on the phone's `IModuleHost`
  (the modem picks it up automatically) or call `ModemModule.setSimProvider(p)`. The modem then
  reads the tray every tick; its own slot is ignored (and SIM clicks on the bay are refused).
* `radio.cellular.CellRadio` / `ModemLink`: `state()`, `linkUp()`, `status()` (the map the `status`
  peripheral method and the `cell` program show: carrier, tower, channel, RSSI, SINR, `bars` 0-4,
  CQIs and rates, bytes, handovers), `cells()` for a network list. Use these for the status-bar
  bars and the Settings Cellular pane.
* `SimCardItem.sim(stack)` / `SimCardItem.issued(sim)` read and make SIM stacks for the tray.
* Position: `ModemPose.of(host)` is the one place a modem's position is computed (block centre via
  `SensorSable.toWorld`); once `IModuleHost` exposes a `Pose`, replace its body with
  `host.getPose()`. Modems update their pose every tick, so moving hosts hand over.
* `api.module.IModuleItemTarget` + `IBayUseItem`: a generic way for an installed module to take an
  item clicked onto its bay.

## Tests

| Layer | Test | Command |
| --- | --- | --- |
| JUnit (26.1) | `CellularTest`: channel plans in their bands and clear of the gaps, CQI table and rates, link budgets, antenna patterns, frames, SIM proofs; engines on `BasicRadioMedium`: attach, data both ways, wrong carrier never asked, forged SIM refused, quota release, mmWave out of reach, handover across two towers (frames keep flowing, old tower lets go), lost tower | `scripts\Test.ps1 -JUnit CellularTest -McVersion 26.1` |
| Rust | `net::cell::tests`: `wwan0` appears with the module, follows carrier, ARP/UDP/TCP both ways, TCP survives a 200 ms carrier gap, re-created with a new modem's MAC | `cargo test -p terminal-os cell` |
| GameTests (`ecm_cellular`) | `cell_lte_far`, `cell_mmwave_hill`, `cell_handover`, `cell_wrong_carrier` (below) | `scripts\Test.ps1 -GameTests ecm_cellular` |

The scenarios are player-built and also spawnable with `/ecm scenario spawn <name>`:

* `cell_lte_far`: the tower and server are 300 blocks east of the phone (kept loaded with
  `/forceload`, the far site is a "remote host" of the scenario, outside its footprint). Control: no
  SIM, no carrier, `dhclient` gets no lease. Then: SIM issued at the tower, attach, DHCP lease from
  the server's `dhcpd` over the tower's backhaul, 3/3 pings, the tower screen lists the SIM.
  (The GameTest world doesn't generate the chunks in between; the path there is free space plus
  ground reflection.)
* `cell_mmwave_hill`: one tower, a mmWave computer in line of sight (attaches at 256QAM, pings) and
  one 25 m away behind a 3-block earth ridge: the traced path costs ~12,600 dB more at 28 GHz,
  `cell scan` hears nothing, no ping. Swapped to an LTE modem (new SIM), that computer attaches
  through the ridge (33 dB of earth at 821 MHz) and pings.
* `cell_handover`: towers A and B of one carrier on one cable run with the server between them; the
  phone rides a Sable cart (the movement is scripted) 28 blocks from in front of A to in front of B while an SSH session is
  open; tower B takes over, A lets go, and the same session runs a command afterwards.
* `cell_wrong_carrier`: a cell-blue SIM next to cell-red's tower: `cell scan` lists the red tower,
  the modem keeps searching, no ping, the red tower saw no attach request. A cell-red SIM then
  attaches at once.

## Deviations from the earlier sketches

* `wwan0` with a 5-function ABI instead of an extra `ethN` (reason above).
* mmWave is modelled FDD like LTE (real n257 is TDD) to keep one scheduler.
* Uplink access is one fixed slice per device rather than a scheduler; devices of different
  towers on one channel can still collide on the same slice.
* Handover is intra-frequency (towers of a carrier should share a channel); a tower on another
  channel is reached by re-establishment after the old one fades out.
* The air interface is unencrypted after authentication (keys only authenticate).
* Towers don't draw FE yet (like the Access Point).

## Known gaps

* No rain fade on mmWave yet (the microwave links apply `Atmosphere` themselves; the medium doesn't).
* Tower antenna patterns are not rotated with a Sable ship's orientation (positions are).
* `wwan0` uses one modem; a second modem in the same computer is ignored.
* Carrier screen has no SIM revoke button (`CarrierRegistry.setRevoked` exists) or usage reset.
