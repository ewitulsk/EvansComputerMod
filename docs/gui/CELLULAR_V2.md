# Cellular v2: cell networks built from primitives

Status: approved design (2026-10-12), to be built in the GUI/ePhone PR. Replaces the
block-based design described in [CELLULAR.md](CELLULAR.md), which never shipped.

## Why

AGENTS.md "Build devices from primitives": a router is a computer running a routing program,
not a router block. The v1 cell tower was a Java block that ran the whole base station, kept a
world-wide subscriber table (`CarrierRegistry`) and issued SIMs. In v2:

- a **cell site** is an antenna and a radio, driven by a computer running a cell-site program;
- a **carrier core** is a program on another computer that owns subscribers, authentication and
  phone IP addresses;
- the **phone** runs its side of the protocol in software, as Wi-Fi does with `wpa_supplicant`.

## Architecture

```
 ePhone / Terminal                Cell site (player-built)                    Carrier core (player-built)
 ┌─────────────────────┐  air   ┌──────────────────────────────────────┐ any IP ┌───────────────────────────────┐
 │ Modem module        │ ~~~~~~ │ LTE / mmWave antenna on a mast       │ network│ Computer running `cored`      │
 │  (framed PHY radio) │        │   │ coax                             │ ─────► │  subscriber DB (a file)       │
 │ kernel wwan0        │        │ Advanced SDR in framed PHY mode      │        │  SIM challenge / keys         │
 │ `celld` (UE daemon) │        │   │ peripheral                       │        │  phone IP pool, NAT optional  │
 │ SIM (smart card)    │        │ Computer running `celld-site`        │        │  tunnel per phone             │
 └─────────────────────┘        │  beacons, scheduler, relay, tunnels  │        │  handover = path switch       │
                                │  UI: "Cell Site" app                 │        │  UI: "Carrier Console" app    │
                                └──────────────────────────────────────┘        │  Card Reader/Writer: SIMs     │
                                                                                └───────────────────────────────┘
```

A phone's IP address lives at the core. A cell site relays the phone's packets to the core in a UDP
tunnel. On handover the new site tells the core, which switches the tunnel, so the phone keeps its
address across sites anywhere the core can reach over IP (other villages, behind routers, over BGP).

## Primitives

| Primitive | Status | Notes |
|---|---|---|
| Radio medium, antennas, masts, coax, antenna solver | exists | LTE sector and mmWave array antennas stay (passive blocks). |
| **SDR framed PHY mode** | built (`gui/cell2-framed-phy`) | See below and [SDR_PROGRAMS.md](../radio/SDR_PROGRAMS.md#framed-phy-mode-advanced-sdr) (interface, record format, what is modelled). LTE Sector / mmWave Array antennas now take coax (bottom jack). |
| **SIM smart card** | new | The SIM item holds a secret key that never leaves the server and is never synced to clients (a persistent data component with no network codec). It answers challenges; it cannot be read. |
| **Card Reader/Writer** | new | Generic block: 1-slot card inventory, hopper/pipe input and output, **Auto Eject Sim** setting (true / false), peripheral `card_reader` for the computer next to it. Reusable for keycards later. Model made in Blockbench. |
| Sockets (TCP/UDP), `tun`, packet sockets, routing, NAT | exists | Core and cell site use them unchanged. |
| `wwan0` kernel interface | exists | Driven by `celld` instead of a Java modem engine. |
| UI toolkit, app platform, App Store | exists | Cell Site and Carrier Console are ecm-ui apps installed as system apps. |

### SDR framed PHY mode (addition to the SDR blocks)

Raw IQ (today's SDR mode) is right for experimenting with modulation, listening to broadcast radio and
DSP programs, but a Wasm program cannot do LTE signal processing in real time. Real base stations
offload the PHY to dedicated silicon; the framed mode is that silicon.

- The computer chooses a **PHY profile** (`lte`, `mmwave`; later others), channel, bandwidth, transmit
  power, and gets back frames with measurements. Modulation per frame follows the CQI table that is
  already in the v1 PHY profiles (`CellTech`, `CellPhy` move here).
- **Transmit** a frame now or in a scheduled slot of the medium's airtime clock; **receive** frames with
  RSSI, SINR, CQI and timestamp; **beacon** offload (repeat a frame every N ms without the computer
  sending each one); the radio handles retransmission timing and timing advance.
- Tier gating: the Advanced SDR (and the modem modules' built-in radio) can run framed LTE / mmWave.
- Interface: peripheral methods for configuration and a device file for frames (`/dev/sdrN.frames`,
  same style as `/dev/audio`), so programs can `poll` it with their sockets.
- **When to use which mode**: IQ mode for anything where the program is the modem (SDR programs,
  experiments, broadcast reception); framed mode for protocols whose PHY would be silicon in real
  life (cellular). This guidance goes into docs/radio/SDR_PROGRAMS.md and the SDR block tooltip.

The modem modules (LTE Modem, mmWave Modem) become the same framed radio, built into a module, with no
protocol logic of their own. Built as `FramedModemModule` (peripheral `framed_radio`, `/dev/frames.<bay>`),
loaded instead of the v1 engine while the server config `modem.framedRadio` is true (default false until
phase 4 removes the v1 engine and the flag).

## Software

1. **`ecm-cell` crate** (Rust, no_std-friendly where cheap):
   - Air interface (simplified LTE): system information beacon (carrier id, cell id, area, neighbour
     list with channels), random access and attach, SIM challenge-response, session keys and
     encryption of user traffic, scheduling grants (uplink slots), measurement reports, handover
     command (same channel and other channels), detach.
   - Site ↔ core protocol: control over TCP (attach, auth, path switch, release, quotas), user plane
     over UDP tunnels (one per phone).
   - Tested in the Rust simulator (many sites and phones, virtual clock, packet loss, a site going
     down, core restart, wrong carrier, forged SIM).
2. **`celld-site`** (cell-site daemon) + **Cell Site** app: beacons, uplink scheduler (slots
   coordinated with same-carrier neighbours it hears), relays air ↔ tunnel, connects to its configured
   core. UI: attached phones and their signal, channel and power, neighbours, core link, traffic.
3. **`cored`** (carrier core) + **Carrier Console** app: subscriber database file on that computer
   (IMSI, key, quota, usage, revoked), challenges and keys, IP pool and tunnels, path switch on
   handover, routing of phone traffic onto its LAN (NAT optional via the router code). UI: write
   SIMs through the Card Reader/Writer, subscribers, quotas, revoke, live sites and phones, handover
   log.
4. **Phone side**: `celld` (UE daemon, like `wpa_supplicant`) drives the modem's framed radio and
   `wwan0`; the `cell` command, the phone status bar and a Settings → Cellular pane talk to it.

## Removed

`cell_tower` block and block entity, `TowerLink`, `CarrierRegistry`, the Java attach / handover / HMAC
engines in the modem modules, the carrier screen. Cellular has not shipped, so there is no migration.

## Performance

Every phone packet passes through Wasm programs on the site and the core, as it already does through
router computers. Scheduling runs in batches (about 10 Hz), not per millisecond; the framed PHY does
the per-frame work. Throughput will be lower than v1's Java tower; it is measured and written down
here when built.

## Phases and done-when

1. **Primitives** (two lanes in parallel)
   - SDR framed PHY mode: frames over the medium between two SDRs at LTE and mmWave profiles, beacons,
     scheduled slots, measurements; docs on when to use it.
   - SIM smart card + Card Reader/Writer: hopper in/out, Auto Eject Sim true/false, a computer writes
     a blank SIM through `card_reader`, the key never reaches a client.
2. **`ecm-cell` + simulator scenarios**: attach, handover (both kinds), site failure, core restart,
   wrong carrier, forged SIM.
3. **Programs and UIs**: `celld-site`, `cored`, phone `celld`, Cell Site and Carrier Console apps,
   Settings → Cellular.
4. **Delete v1 and rewrite scenarios** (real player builds per AGENTS.md):
   - `cell_lte_far`, `cell_mmwave_hill`, `cell_wrong_carrier`;
   - `cell_roaming`: two sites behind **different routers**, one core, the phone keeps its IP and an
     SSH session while walking between them;
   - `cell_chat`: a cell network plus a package server; the phone installs the chat app from the App
     Store over LTE and messages a server.
