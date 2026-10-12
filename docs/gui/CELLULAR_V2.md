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
the default since phase 3; the server config `modem.v1Engine = true` (which replaced the phase-1 preview switch
`modem.framedRadio`) loads the v1 engine instead until phase 4 removes it and the setting.

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
5. **Tech Villages get a cell network** (user request, 2026-10-12; after phases 1-4 are done and tested):
   - every Tech Village and sites along the fiber span between villages get a cell site built from
     the primitives (antenna on a mast, SDR in framed mode, computer running `celld-site`) with
     backhaul over the fiber;
   - all sites belong to **one carrier**: one `cored` computer in a seed-chosen village (as the chat
     server's village is chosen), so phones keep their IP between villages;
   - a new **Phone Store** building in each village: ePhones on display fitted with SIMs for that
     network, and a chest of SIM cards **preloaded with the carrier network** (user decision: they work
     as soon as they go in a phone; their keys are provisioned in the core's subscriber database at generation);
   - tested per AGENTS.md world-generation rules: fresh-world natural generation plus a real client
     render check.

## As built: smart card and card writer

Phase 1, second lane (branch `gui/cell2-smartcard`). 1.21.1 only, like the rest of cellular.

### SIM smart card (`card/SmartSim`, `card/SimAuth`, `radio/cellular/SimCardItem`)

A SIM card item is blank (no components) until written. A written card has two components:

| Component | Type | Saved | Sent to clients |
|---|---|---|---|
| `evanscomputermod:sim_card` | `SimIdentity(carrier, imsi)` | yes | yes (tooltip: carrier, IMSI) |
| `evanscomputermod:sim_secret` | `SimSecret(keyHex)`, the 16-byte key K | yes | **no**: its network codec writes zero bytes and every client decodes `SimSecret.HIDDEN` |

"Persistent with no network codec" is not enough in vanilla: `DataComponentType.Builder.build()` falls back
to the persistent codec for the network when no stream codec is given, which would send the key. So the
secret's stream codec is an explicit redaction (`SimSecret.REDACTED_STREAM_CODEC`). Consequences handled:

- **Creative inventory.** A creative client sends whole stacks back (`ServerboundSetCreativeModeSlotPacket`),
  which would replace the key with the placeholder. `card/mixin/CreativeSlotMixin` calls `SimSecretGuard`
  before the server applies such a stack: it restores the key from a card with the same identity in the
  player's inventory, the slot being replaced, or one the player held recently (a move is "empty the old
  slot, fill the new one"); a SIM in an ePhone's tray likewise. A placeholder SIM with no match is stored
  keyless (it answers no challenge).
- **Card Reader/Writer.** Its block entity sends clients no data (`getUpdateTag` is empty); the block state
  shows whether a card is in. Its screen's slot uses the item network codec, so the key stays out.
- **v1 modem's own slot.** A modem's state is custom data on its item (synced, and part of the Terminal's
  update tag), so it now holds only the SIM's identity and a reference into `SimVault` (server saved data:
  `ref = hex(SHA-256("ecm-sim-vault|" + carrier + "|" + imsi + "|" + keyHex))[0..32)` maps to the key).
  It goes away with the v1 modem engine in phase 4.
- Known leak path left: a chat hover that shows the item (`HoverEvent.ItemStackInfo` uses the persistent
  codec), e.g. `/give` feedback. Nothing in the mod puts a written SIM into chat.

API (Java; `SimCardItem.card(stack)` returns a `SmartSim` that changes that stack in place; the logic is
pure, over a `SmartSim.Storage`):

| Method | Meaning |
|---|---|
| `isBlank()` | no identity yet |
| `identity()` | `SimIdentity(carrier, imsi)` or null |
| `respond(byte[16] challenge)` | `SimAuth.Response(res[16], sessionKey[32])`; fails on a blank card or where the key is unknown (a client) |
| `write(imsi, carrier, byte[16] key)` | only on a blank card, one card at a time (a stack of blanks is refused); IMSI 6-15 digits, carrier `[a-z0-9_-]{1,24}` |

No `lock`: a written card is already immutable (it cannot be rewritten), and nothing reads the key back.

**Key derivation (`SimAuth`, version 1).** `cored` must compute exactly this from its database copy of K:

```
K    = 16-byte key, RAND = 16-byte challenge; labels are ASCII without a terminator; || = concatenation
RES  = HMAC-SHA256(K, "ECM-SIM-RES" || RAND)[0..16)     16 bytes, the card's answer
KS   = HMAC-SHA256(K, "ECM-SIM-KS"  || RAND)            32 bytes: KS[0..16) encryption key, KS[16..32) integrity key
```

Vectors (computed independently with Python's `hmac`, checked by `SimCardTest`):

| K | RAND | RES | KS |
|---|---|---|---|
| `00112233445566778899aabbccddeeff` | `000102030405060708090a0b0c0d0e0f` | `4c455ba887affefa1448bea727baa584` | `3a2295de88d4ea04764491a508856d342f71ed47a7415c5bffeabd934308713a` |
| `00112233445566778899aabbccddeeff` | `ffffffffffffffffffffffffffffffff` | `3e316dff329533c535f8b5106208b75c` | `4531753cba8d31b05e36c2c9fd04a80f1baac7d541c8a580b21b2cd17f9cf09e` |
| 16 zero bytes | 16 zero bytes | `7fbf4baffaedda01afa5c9856ba7a9f4` | `027c68857ae22afa9558fa8d747477918e9a90cb23146cb02617eeafabb9dd23` |

The v1 engine still works on the same cards: `SimCardItem.sim(stack)` builds the v1 `SimCard` (carrier, IMSI,
key) from the two components on the server and `SimCardItem.issued(SimCard)` writes both, so cell towers
still issue SIMs, modems still read them and `SimProvider` (the ePhone's tray) is unchanged.

### Card Reader/Writer (`card/CardReader*`, block `evanscomputermod:card_reader`)

A generic one-card slot (cards = item tag `evanscomputermod:cards`, today the SIM card; keycards later).

- **Player:** right-click with a card puts one in (refused if the slot is taken); right-click with an empty
  hand takes it out; with no card in, or sneaking, it opens the screen: the card slot, the card's state
  (No card / Blank SIM / Written SIM; Held for the computer / Released) and the **Auto Eject Sim** button
  (true / false, default false). The setting is saved with the block, synced to the open screen, and set
  through the vanilla menu-button packet.
- **Automation:** the NeoForge item handler capability on every side. Insert: a card, when empty.
  Extract: only once the card is *released* (below), so a hopper underneath doesn't take an unwritten card.
- **Comparator:** 15 while a card is in.
- **Card life cycle.** A card arriving (hand, hopper, pipe) is *held*, and the computer gets `card_inserted`.
  The computer *releases* it with `eject()` or by a successful `write(...)` (writing finishes a card
  writer's job). Released, with Auto Eject Sim = **true**, the reader pushes the card into the first
  adjacent inventory that takes it (order down, north, south, west, east, up; a hopper pointing into the
  reader is skipped, so a feed hopper doesn't loop it); if none takes it, it stays (released) and the push
  is retried every second. With **false** it waits, released, to be pulled out (hopper below, pipe) or
  taken by hand. When it leaves: `card_removed`. A player can always take a card out by hand.

**Peripheral `card_reader`** (every method on the server thread):

| Method | Returns |
|---|---|
| `present()` | bool |
| `identity()` | `{type, blank, imsi?, carrier?}` (`type` = `sim` or the item id), nil with no card |
| `is_blank()` | bool (error: no card / not a SIM) |
| `write(imsi, carrier, key_hex)` | the identity plus `ejected`; errors: not blank, bad IMSI / carrier / key, no card |
| `respond(challenge_hex)` | `{res, ks}` in hex (for testing and inspection) |
| `eject()` | true if Auto Eject Sim pushed it out, false if it waits to be pulled |
| `status()` | `{present, state (empty/blank/written/card), released, auto_eject}` |

Events: `("card_inserted", <attachment>, <type>, <blank>)` and `("card_removed", <attachment>)`.

Python, through the existing `peripheral` module:

```python
import peripheral
r = peripheral.find("card_reader")
if r.present() and r.is_blank():
    print(r.write("310150000000001", "cell-red", "00112233445566778899aabbccddeeff"))
print(r.respond("000102030405060708090a0b0c0d0e0f"))   # {'res': '4c45...', 'ks': '3a22...'}
ev = peripheral.pull_event("card_inserted")             # ('card_inserted', 'right', 'sim', True)
```

Rust: `rust/wasm-programs/simtool` is the example (`ecm_host_abi::peripheral::call(reader, "write", ...)`,
`peripheral::wait_event`).

**`simtool`** (Rust WASI program, in `/bin` with the other programs):

```
simtool [info]                                          the card in the reader
simtool write <imsi> <carrier> [--key <hex>|--random]   write a blank SIM; prints "key <hex>" once
simtool auth <challenge-hex>                            the card's res and ks
simtool eject                                           done with the card
simtool wait [seconds]                                  wait for the next card_inserted / card_removed
--reader <name>                                         pick a reader when there are several
```

`--random` (the default) uses WASI `random_get` (the host's SecureRandom).
(`ecm_host_abi::random::getrandom` returned all zeros in a WASI program, so simtool doesn't use it.)

Model: `models/card_reader.bbmodel` (Blockbench MCP), exported with `py -3 scripts/export-radio-models.py card_reader`
(kind `facing_card`: the cubes named `card_*`, the inserted SIM and the green LED, appear only in `card_reader_card`).
Recipe: `IGI / IPI / IRI` (iron ingots, a gold nugget, paper, redstone).

### Tests

| Layer | Test | Command |
|---|---|---|
| JUnit (26.1) | `SimCardTest`: the secret component's network encoding is empty (no key, hex or raw) and decodes to `HIDDEN` while the save keeps the key; identity round trip; the RES/KS vectors above; write only when blank (not on a stack, not with bad input); a hidden key answers nothing | `scripts\Test.ps1 -JUnit SimCardTest -McVersion 26.1` |
| GameTest `ecm_cards` | `card_writer` (the scenario below); `creative_slot_keeps_key` (SimSecretGuard restores a moved SIM's key; a placeholder SIM nobody held stays keyless) | `scripts\Test.ps1 -GameTests ecm_cards` |
| Client | `radio_closeup_misc` shows the reader with and without a card | `scripts\Test.ps1 -ClientChecks -ClientSuite radio` |
| Regression | the v1 engine and the phone's SIM tray read the new cards | `scripts\Test.ps1 -GameTests ecm_cellular,ecm_phone` |

Scenario `card_writer` (`/ecm scenario spawn card_writer`): a player places a computer, a Card Reader/Writer
against its east side and a chest beyond it. (1) Auto Eject Sim = true in the reader's screen; a blank SIM
clicked in; `simtool write 310150000000001 cell-red --random` prints the key and the SIM lands in the
chest holding exactly that key. Control: that SIM, taken from the chest and clicked back in, can't be
written again (`simtool: the card is not blank`) and is unchanged; `simtool auth` matches the server's own
HMAC of the stored key; `simtool eject` pushes it back into the chest. (2) Auto Eject Sim = false and a
hopper under the reader: a new blank SIM stays in the reader (control: the hopper doesn't take it), is
written with a given key and the hopper pulls it; its network encoding (what a client gets) has no key.
(3) A hopper on top fed by a chest: a blank SIM put into that chest reaches the reader, `simtool wait`
prints `event card_inserted sim blank=true`, the card is written and pulled out below.
NeoForge's FakePlayer refuses `openMenu`, so `ScenarioPlayer.openMenu` right-clicks the block (its own use
code runs) and then builds the server-side menu from the block's MenuProvider, as `ServerPlayer.openMenu`
would; buttons go through `clickMenuButton` and chest items through shift-clicks, as their packets do.

## As built: ecm-cell protocol

Phase 2 (branch `gui/cell2-protocol`). The protocol is the crate `rust/crates/ecm-cell` (`#![forbid(unsafe_code)]`,
`no_std` + `alloc` without its default `std` feature, so it builds for the kernel and for WASI programs alike); the
simulator runs it end to end on a simulated framed radio medium with real kernels, sockets, `tun` and routers.
No programs or UIs yet: phase 3 wraps the three state machines below in `celld`, `celld-site` and `cored`.

### Layout

| Module | What |
|---|---|
| `ue` | the phone (`celld`): cell search, attach, SIM challenge, network authentication, measurements and gaps, handover, re-attach, idle/paging, detach |
| `site` | the cell site (`celld-site`): system information beacon, random access, attach relay, air security, batched uplink scheduler, handover (source and target), tunnel relay, idle release, paging |
| `core` | the carrier core (`cored`): site registration, challenges, sessions, address pool, tunnels, path switch, usage/quota, revocation, paging, admin commands |
| `air` | air messages: plain messages and protected items (catalogue below) |
| `backhaul` | site <-> core control messages (TCP) and the tunnel header (UDP) |
| `simauth`, `crypto` | SimAuth v1 (vectors above), the `Sim` trait, key derivation, ChaCha20-Poly1305 air protection, replay windows |
| `phy` | the framed PHY record API as types (`TxReq`, `RxFrame`, `TxStatus`, `PhyCmd`, `PhyEvent`) and the `FramedPhy` trait; with feature `framed`, `impl FramedPhy for ecm_radio::framed::Framed` (the in-game device file) |
| `db` | the core's subscriber database file |
| `sched` | the uplink scheduler |
| `seg`, `ip` | segmentation/reassembly/dedup over the air; tun Ethernet/ARP helpers |
| `medium` (std) | a simulated framed radio medium (tests, simulator) |

Every machine is sans-IO: inputs are method calls (`on_phy`, `on_ctl`, `on_bytes`, `on_tunnel`, `on_lan_packet`,
`send_ip`, `tick`) with the time as `Now { ms, slot }`; outputs queue up and the driver drains them with `take_out()`
(`UeOut`, `SiteOut`, `CoreOut`). Nothing blocks, nothing reads a clock, nothing opens sockets.

### Air interface

A frame payload (inside a framed PHY TX/RX record) is one plain message, or a protected frame (`0x10`) whose
plaintext is a list of items `[type u8][len u16][body]` (several per frame: data with a buffer report, a grant with
downlink...). Big-endian; every decoder is strict (truncation, trailing bytes, bad values and unknown types are
errors, never panics). Radio addresses (the PHY's 32-bit frame addresses) identify a phone at a site.

| Plain message | Dir | Fields | Notes |
|---|---|---|---|
| `0x01 SysInfo` | site beacon | version, carrier, cell id, area, flags (`core up`, `barred`), profile, channel, tx power, `ul_mask`, `rach_sub`, neighbours (cell, profile, channel, ul_mask) | beacon offload, every 100 ms, CQI 1 |
| `0x02 RachRequest` | UE | nonce, cause (attach / service) | on the random-access sub-channel (`rach_sub` = 4), CQI 1, HARQ |
| `0x03 RachResponse` | site | nonce, ue id, grant | the grant is the random-access sub-channel for 10 s |
| `0x04 AttachRequest` | UE | IMSI, carrier | wrong carrier or a revoked IMSI: refused by the site itself |
| `0x05 AuthChallenge` | site | RAND (from the core) | |
| `0x06 AuthResponse` | UE | RES (from the SIM) | |
| `0x07 AttachReject` | site | cause | |
| `0x08 SecurityModeCommand` | site | epoch, MAC (computed by the core) | the phone checks the MAC: network authentication |
| `0x09 Paging` | site broadcast | TMSIs | idle phones with downlink waiting |
| `0x0A ServiceRequest` | UE | TMSI, count, MAC | an idle phone resumes |

| Protected item | Dir | Fields |
|---|---|---|
| `0x20 SecurityModeComplete` | UE | |
| `0x21 AttachAccept` | site | ip, prefix, gateway, dns, mtu, TMSI, session |
| `0x22 AttachComplete` | UE | |
| `0x23 Grant` | site | from/to slot, period, offset, len, sub-channel, CQI |
| `0x24 BufferStatus` | UE | bytes waiting |
| `0x25 MeasurementReport` | UE | serving cell first, then neighbours: cell, profile, channel, RSSI x10, SINR x10, their `ul_mask` |
| `0x26 HandoverCommand` | source site | target cell, address, profile, channel, epoch, ue id, grant |
| `0x27 HandoverConfirm` | UE to target | (protected with the target's key) |
| `0x28 Detach` / `0x29 DetachAccept` | UE / site | cause |
| `0x2A Release` | site | cause (quota exhausted, SIM revoked, core unavailable, inactivity, ...) |
| `0x2B Data` | both | tunnel seq, segment idx/count, bytes (an IP packet in 1..255 segments) |
| `0x2C MeasGap` | UE | from/to slot: the phone is on another channel then (the site holds its downlink) |
| `0x2D Keepalive` | both | |

Causes: unknown subscriber, wrong carrier, authentication failed, SIM revoked (these four are permanent: the
phone stops trying until its SIM changes or the user asks), quota exhausted (60 s back-off), core unavailable,
congestion, detached, radio link lost, site shut down, handed over, released by the operator, session lost,
inactivity, handover failed, protocol error.

**Attach.**

```
UE                         site                                    core
 |-- RachRequest(nonce) ---->|                                        |
 |<- RachResponse(id,grant) -|                                        |
 |-- AttachRequest(IMSI) --->|-- UeAttach(ue, cell, IMSI) ----------->|  carrier? subscriber? revoked? quota?
 |                           |<- UeChallenge(RAND) -------------------|  RES, KS = SimAuth(K, RAND)
 |<- AuthChallenge(RAND) ----|                                        |
 |  SIM: RES, KS             |                                        |
 |-- AuthResponse(RES) ----->|-- UeAuthResponse(RES) ---------------->|  RES ok? address, TMSI, epoch 1
 |                           |<- UeAccept(session, ip, K_AS, SMC MAC) |
 |<- SecurityModeCommand ----|  (the site holds only K_AS)            |
 |  check MAC, derive K_AS   |                                        |
 |== SecurityModeComplete ==>|                                        |
 |<= AttachAccept + Grant ===|                                        |
 |== AttachComplete ========>|-- UeActive(session) ------------------>|  tunnel up
```

**Handover** (make-before-break). The phone reports (every second, and at once when a neighbour is 3 dB
stronger); the source site decides (neighbour > serving + 3 dB), asks the core (`HoRequired`); the core derives the
target's `K_AS` (new epoch) and asks the target (`HoRequest`); the target admits the phone with a grant
(`HoRequestAck`); from then the core **bicasts** downlink to both sites; the source sends `HandoverCommand`; the
phone retunes if needed and sends `HandoverConfirm` to the target (HARQ, protected with the new key); the target
sends `PathSwitch`; the core moves the tunnel and releases the source (`UeRelease(handed over)`). Downlink packets
carry the core's tunnel sequence number to the phone, which drops the duplicates the bicast produced, and the
phone keeps the source's key until the switch, so source downlink still arrives meanwhile. Uplink sequence numbers
are per site: the core keeps a replay window per sending site and starts a fresh one for every new radio context.
**Inter-frequency**: while the serving cell is below -75 dBm and its system information lists neighbours on other
channels, the phone announces a measurement gap (`MeasGap`, 160 ms every 2 s), retunes, collects beacons and comes
back; the site holds its downlink during the gap. **Failure**: no beacon from the serving cell for 1.5 s (or a
handover not confirmed in 1.5 s) is a radio link failure: search and re-attach; the address is sticky at the core,
so it comes back unchanged.

**Idle and paging** (minimal). A site with `idle_after` releases a quiet phone (`Release(inactivity)`, `UeIdle`);
the core keeps the session and address. Downlink for an idle phone is buffered (32 packets) and paged at every site
(again every second); the phone (or its own uplink data) sends `ServiceRequest(TMSI, count, MAC)`; the core checks
the MAC and that the count grew (anti-replay), bumps the epoch and resumes the same session through a new security
mode command.

### Crypto

| Key | Derivation | Who holds it |
|---|---|---|
| K | the SIM's key | the SIM (server side, never exposed) and `cored.db` |
| RES, KS | SimAuth v1 (above) | the SIM's answer; the core computes the same |
| `K_AS` | `HMAC-SHA256(KS_enc, "ECM-CELL-AS" + cell_id u32 + epoch u32)` | the phone, the core, the one site serving that cell |
| SMC MAC | `HMAC-SHA256(KS_int, "ECM-CELL-SMC" + RAND + cell_id + epoch + ue_addr u32)[0..16)` | computed by the core, checked by the phone |
| Service MAC | `HMAC-SHA256(KS_int, "ECM-CELL-SR" + tmsi u32 + count u32)[0..16)` | the phone; checked by the core |
| Site proof | `HMAC-SHA256(site_secret, "ECM-SITE-AUTH" + transcript)`, transcript = the `CoreHello` body + the `SiteAuth` body up to the proof (both nonces, site id, cells, tunnel port) | the site; checked by the core |
| Backhaul keys | `K_BH = HMAC-SHA256(site_secret, "ECM-BH-KEY" + core_nonce + site_nonce + site_id)`; control `HMAC(K_BH, "ECM-BH-CTL")`, tunnels `HMAC(K_BH, "ECM-BH-TUN")` | the site and the core (phase 3, below) |

(`+` = concatenation, labels ASCII without terminator, integers big-endian.) Air protection: ChaCha20-Poly1305 (the
RustCrypto crate the SSH code already uses; there is no AES-GCM in the workspace), key `K_AS`, frame
`[0x10][epoch u32][counter u32][ciphertext + tag]`, nonce `dir u8, 0 0 0, counter u64` (1 up, 2 down), associated
data `0x10, epoch, counter, src u32, dst u32, dir` (a frame can't be replayed to another radio or turned around);
a 64-frame replay window per direction. Every new radio context (attach, handover, resume) has a new epoch, so
counters restart without nonce reuse. User data and control after the security mode command are both protected
(one AEAD gives encryption and integrity; `KS_int` authenticates the network and the resume). Before it, only the
unprotected attach messages above cross the air. A site never sees K or KS. The backhaul is sealed too since phase 3
("As built: phase 3 network side", below).

### Uplink scheduling

The site schedules every 100 ms (`site::SCHED_PERIOD_MS`) and grants each connected phone the next 400 ms (renewed
every batch). Sub-channels 1-3 carry data, 4 random access. A phone alone on a sub-channel may use every slot; `m`
phones sharing one take turns in runs of 4 slots (LTE; 16 for mmWave): period `4m`, offset `4j`. The order is
stable (by address) and a changed pattern starts 10 ms later, so a grant that arrives late never overlaps a
neighbour's. Grants carry the CQI the site measures on that phone; the phone forces it on its TX records and sizes
its frames to end inside its run. Neighbour coordination: sub-channels that same-channel neighbour cells use (their
`ul_mask`, as phones report it) are taken last, and the first choice rotates with the cell id. Sites learn their
neighbours from the reports (automatic neighbour relations) and list them in their system information.

### Site <-> core

Control: one TCP connection per site to the core (port 7700), messages `[u32 len][u8 type][body]` (at most 64 KiB),
heartbeats every second both ways (3.5 missed = down). User plane: UDP, core port 7701, one tunnel per phone (its
session id), header `[0xE1][flags][0 u16][session u32][seq u32]`; a packet bigger than one datagram can carry
(1460 bytes of IP) travels as two (MTU handling: the phone keeps a 1500-byte MTU).

| Message | Dir | Meaning |
|---|---|---|
| `CoreHello(version, carrier, nonce)` | core | first message on a connection |
| `SiteAuth(site_id, cells, tunnel_port, proof)` | site | registration with the site credential |
| `SiteAccept(heartbeat_ms, revoked IMSIs)` / `SiteReject(cause)` | core | |
| `Heartbeat(seq)` | both | |
| `UeAttach(ue, cell, IMSI, carrier)` / `UeService(ue, cell, tmsi, count, mac)` | site | relays |
| `UeChallenge(ue, RAND)` / `UeAuthResponse(ue, RES)` | core / site | |
| `UeAccept(ue, session, ip config, tmsi, epoch + K_AS, SMC MAC)` / `UeReject(ue, cause)` | core | |
| `UeActive(session)` / `UeIdle(session)` | site | |
| `UeRelease(session, cause)` | both | end the radio context (core: quota, revoked, operator; site: lost, detached) |
| `HoRequired(session, target cell)` | source | |
| `HoRequest(ho, session, ue, target cell, ip config, tmsi, epoch + K_AS)` / `HoRequestAck(ho, ue id, grant)` / `HoFailure(ho, session, cause)` | core / target | |
| `HoCommand(ho, session, cell, addr, profile, channel, epoch, ue id, grant)` | core to source | |
| `PathSwitch(ho, session)` / `PathSwitchAck(session)` | target / core | |
| `Page(tmsi)`, `Usage(session, used, quota)`, `Revoked(IMSI)` | core | |

### Core: subscriber database and operations

`cored` owns one text file (`ecm_cell::db`; strict parser with line numbers, rewritten whole): `version 1`,
`carrier`, `pool a.b.c.d/len`, `gateway` (the core's own address on its tun), optional `dns` and `mtu`,
`site <id> <secret-hex>` lines, and `sub <imsi> key=<hex> quota=<bytes, 0 = none> used=<bytes> revoked=<0|1>
ip=<sticky address>`. A phone's address is assigned the first time it attaches and written to the file, so it
survives re-attaches, handovers and core restarts. Usage is counted at the core on every packet both ways and saved
at most every 5 s (and on every state change). Admin commands (`Core::admin`, for the Carrier Console and `cored`'s
admin socket): `list`, `sites`, `sessions`, `add <imsi> <key> [quota]`, `revoke`, `unrevoke`, `quota <imsi>
<bytes>`, `reset-usage`, `site-add <id> <secret>`, `release <imsi>`.

### What the phase-3 programs call

| Program | Loop |
|---|---|
| `celld` (phone) | `Ue::new(UeConfig)`, `start(&mut sim)` with a `Sim` over the phone's SIM slot or a `card_reader` (`identity()`, `respond(challenge)`). Every 5-10 ms: radio records to `on_phy`; frames the kernel sends on `wwan0` through `ip::from_kernel` (answer ARP, `send_ip` the IPv4); `tick`; apply `UeOut::Phy` to the modem's framed radio (`phy::apply` over `FramedPhy`), write `UeOut::Ip` with `ip::to_kernel`; on `UeEvent::Attached` create (or keep) the tun with the address/prefix and add `default via gateway` (netlink `RTM_NEWROUTE`). `detach()` for the `cell` command, `status()` for the status bar. |
| `celld-site` | `Site::new(SiteConfig)`, `start`; connect TCP to the core while `wants_core()` (then `core_connected`); bytes through `backhaul::Framer` to `on_ctl`, EOF/error `core_disconnected`; a UDP socket bound to `tunnel_port` to `on_tunnel`; radio records to `on_phy`; `tick`; apply `SiteOut::Phy`, send `Ctl` (`encode()`), `CloseCore`, `Tunnel` (to the core's tunnel port). `status()` for the Cell Site app. |
| `cored` | parse the database file; `Core::new(db, cfg, entropy)` (WASI `random_get`); a TCP listener: `on_connect(id, peer)`, `on_bytes`, `on_disconnect`; the UDP tunnel socket: `on_tunnel(from, bytes)`; a tun with the pool's gateway address and IP forwarding on (the router service with `ip routing`), frames through `ip::from_kernel(frame, gateway)` to `on_lan_packet`; `tick`; apply `CoreOut` (`Send`, `Close`, `Tunnel`, `ToLan` via `ip::to_kernel`, `SaveDb` = write the file, `Log`). `admin(line)`, `sessions()`, `sites()` for the Carrier Console. The LAN routes the pool to the core (a static route, or BGP in Tech Villages). |

The simulator's agents (`rust/simulator/src/cell.rs`) are these loops over the kernel's socket IPC and are the
reference for the programs.

### Simulator

`rust/simulator` gained a `[radio]` section (the medium: path-loss exponent, walls), a `[node.cell]` section (an
agent on that computer: `role = "site" | "core" | "ue"` and its settings), and the steps `cell start|stop|restart|
status|mark NODE`, `cell admin NODE "command"`, `move NODE x,y,z [over 10s]`, `expect_cell NODE /re/ [within T]` and
`expect_cell_not NODE /re/ [for T]` (see `rust/simulator/README.md`, "Cellular"). The medium (`ecm_cell::medium`)
models log-distance path loss plus walls, FDD channels, slots, CQI link adaptation and airtime, collisions per
sub-channel by SINR, HARQ (3 retransmissions, 2 CQI steps lower) and beacon offload. Agents run as processes of
their node's kernel (own pid, real TCP/UDP sockets, tun, netlink route) on the virtual clock; runs are
deterministic (two runs give identical transcripts).

| Scenario | Shows |
|---|---|
| `30_cell_attach` | attach, the address from the core, ping both ways; control: an unassigned pool address does not answer |
| `31_cell_handover_same_channel` | an SSH session survives a handover on one channel, IP unchanged; a ping across the way back loses at most 1 of 25 |
| `32_cell_handover_channels` | the same across channels 1 and 3, with measurement gaps |
| `33_cell_routed_sites` | sites behind two routers in different subnets, the core in a third: SSH and the address survive; control: village B's uplink cut stops the traffic |
| `34_cell_site_failure` | a site loses power and backhaul: the phone re-attaches through the other site with the same address; the site re-registers when back |
| `35_cell_core_restart` | `cored` restarts: the sites notice and release, the database is reloaded, the phone gets its address back |
| `36_cell_wrong_carrier` | a cell-blue SIM never transmits to cell-red cells (`no service (heard cell-red)`); control: a cell-red phone attaches |
| `37_cell_forged_sim` | right IMSI, wrong K: rejected (authentication failed), no session, no retries; control: a genuine SIM |
| `38_cell_quota` | a 1500-byte quota: released in the middle of a ping, attach refused, usage in `cored.db`; the operator lifts it and the phone is back |
| `39_cell_revoked` | a revoked SIM refused by the site; revoking an attached SIM releases it for good (`revoked=1` in the file) |
| `40_cell_idle_paging` | idle after 3 s, paged by downlink, resumed by uplink, same address |
| `41_cell_celld_cli` | (phase 3) the phone agent is the real `celld`; the `cell` program typed on the phone: status, scan, ping, handover, down/up (see "As built: phase 3 phone side") |

Each takes well under a second of wall time (6-62 s virtual).

### Tests

| Layer | Command |
|---|---|
| Unit: codecs (round trips, every truncation, trailing bytes, bad values, 20 000 random and bit-flipped inputs), SimAuth vectors (the table above), derivations, AEAD and replay, scheduler (no overlap within a cell, neighbour steering), segmentation/dedup, database parse/write/errors/sticky addresses, medium (range, channels, HARQ, collisions per sub-channel, walls, motion) | `cargo test -p ecm-cell --lib` |
| End to end on the medium (ideal backhaul through the byte encodings): attach and data both ways, handover on one channel and across channels with steady traffic (>= 95 % / 90 % delivered, no downlink duplicates), wrong carrier, forged SIM, unknown IMSI, quota, revocation, core restart, site failure, idle and paging, six phones in one cell, a site with a bad credential, garbage on every input of a running network | `cargo test -p ecm-cell --test e2e` |
| Simulator scenarios above | `scripts\Test.ps1 -Rust ecm-cell,terminal-simulator -Scenarios _cell_` |

### Simplifications and open questions

- **Backhaul security.** Done in phase 3: control and tunnels are sealed (see "As built: phase 3 network side").
- **HARQ ACKs** in the simulated medium always come back when the frame got through (the game's PHY sends real ACK
  frames that can be lost). Uplink retransmissions land 8 slots later whatever the grant says (as in the game's
  PHY); on a shared sub-channel they can meet another phone's run.
- **Idle mode** is minimal: no tracking areas (paging goes to every site), no idle reselection signalling.
- **One cell per site** in `SiteConfig`; `SiteAuth` already carries a list of cells.
- **Phone MTU** stays 1500: the air segments packets, the tunnel splits large ones in two datagrams.
- **Neighbour coordination** is by sub-channel preference only; two same-channel cells loaded at one edge still
  share sub-channels.
- Decided in phase 3: the phone's `wwan0` is a tun named `wwan0` created and owned by `celld` (below); `cored`'s
  admin socket belongs to the network lane.

## As built: phase 3 phone side

Branch `gui/cell2-phone`. The phone runs its side of the protocol as a program, like `wpa_supplicant` for Wi-Fi:
`celld` drives the modem's framed radio with the `ecm-cell` UE machine, authenticates through the SIM, and owns
`wwan0`. The network side (`cored`, `celld-site`, their apps) is the other phase-3 lane.

### Modems are framed radios by default

| | |
|---|---|
| Default | LTE / mmWave Modem modules load as `FramedModemModule` (peripheral `framed_radio`, `/dev/frames.<bay>`, `/dev/framesctl.<bay>`): `RadioConfig.modemFramedRadio()` is true unless the server config `modem.v1Engine = true`. The key is new (it replaced `modem.framedRadio`) so an existing config that had turned the preview off doesn't pin v1. |
| v1 kept until phase 4 | With `modem.v1Engine = true` (or `RadioConfig.overrideModemFramedRadio(false)`), modules load as the v1 `ModemModule` (Cell Tower attach, the kernel's `wwan0` through the `cell_*` host ABI). The four v1 scenarios (`cell_lte_far`, `cell_mmwave_hill`, `cell_handover`, `cell_wrong_carrier`) pin it for their run with the decor `CellularScenarios.V1_ENGINE` (set in `terrain`, before any modem is fitted; cleared with the scenario). |
| No fight over `wwan0` | The kernel's v1 `wwan0` exists only while a v1 modem is present (`cell_present` looks for a `CellRadio`; a framed module isn't one), so with framed modems the name is free for celld's tun. If a v1 modem made it earlier in the same boot, the tun can't take the name and celld logs `wwan0: can't create the interface ... (is the v1 modem engine using wwan0?)`. |
| SIM slot | On a Terminal the framed modem keeps the v1 SIM slot (right-click its bay with a written SIM; the same `sim` entry in the module's saved state, so both engines read it) and exposes it as the companion peripheral `<bay>_sim` (`IComputerModule.companions()`, attached by `PeripheralHub.setModule`). On an ePhone the SIM tray is the SIM: the phone sets itself as the modem's `SimProvider` before attaching it, so the modem brings no slot of its own. |
| Fixed on the way | `FramedModemModule` built its antenna before its technology was set (a NullPointerException: in phase 1 the module never loaded in game). |

### SIM access: peripheral `sim`

`card/SimPeripheral` (version-neutral): how the phone's own software asks the SIM to authenticate, as a modem asks
its SIM over the card interface. On an ePhone it is the SIM tray, attached as `sim` (a wireless attachment, always
present; `present()` says whether a card is in). On a Terminal it is a framed modem's slot, `<bay>_sim`.

| Method | Returns |
|---|---|
| `present()` | bool |
| `identity()` | `{carrier, imsi}` (IMSI as 15 digits), nil with no card |
| `respond(rand_hex)` | `{res, ks}` hex: RES (16 bytes) and KS (32 bytes) of SimAuth v1 (above); error `no SIM card` |

The card answers on the server (`SimAuth.respond` with the key from the item's server-only component, or
`SimVault` for a modem's slot); **K never leaves the server**: no method returns it, and `SimPeripheralTest`
calls every method with every kind of argument and checks that K is in no answer (as Java values or as the bytes
a program receives). **KS does reach the phone's own software, and that is correct**: KS is the UE's session key
material (the phone derives `K_AS` from it to protect the air link), as in LTE where the USIM hands CK/IK to the
terminal. The core has KS too; a cell site never sees it. Apps need the `cellular` permission
(`ProcessSandbox.peripheralPermission("sim")`); celld, started by the phone or typed in a Terminal, is unconfined.

### `celld` (rust/wasm-programs/celld)

`celld [--phone] [--modem NAME] [--sim NAME] [--quiet]`. The library is the daemon (the simulator's phone agent
runs it, below); `host.rs` is the computer.

| Part | What |
|---|---|
| Driver loop (`Daemon::step`, every 5 ms) | radio records to `Ue::on_phy` (beacons are also kept for `cells`); frames the kernel sent on `wwan0` through `ip::from_kernel` (ARP answered, IPv4 to `Ue::send_ip`); every 2 s the SIM is looked at again (taken out: the session is dropped; swapped: a new attach; put in: attach); `Ue::tick`; then the UE's outputs: `PhyCmd` to the radio, downlink IP to the tun (`ip::to_kernel`), events handled and logged. |
| Radio | `/dev/frames.<bay>` through `ecm_radio::framed::Framed`. A retune sends only the control lines that changed (`channel N`): a `phy` line would reset the radio's peers and HARQ. The slot clock is read from the control file every 250 ms and extrapolated in between. |
| SIM | `HostSim`: `identity()` / `respond(rand_hex)` on the `sim` peripheral (`--sim`, `sim =` in the config, `<modem>_sim`, else the first `sim`). |
| `wwan0` | On `Attached`: a tun `wwan0` with the core's address and prefix (MAC `02:ce:<radio address>`), `default via <gateway>` by netlink (`RTM_NEWROUTE`, the kernel picks the tun: the gateway is in its subnet) and the core's DNS server as the resolver (`RTM_ECM_DNS`, below). Kept across handovers and re-attaches (the address is sticky at the core; a changed gateway gets a new route); removed on detach, a permanent release / reject, or the SIM going. |
| Config | `/etc/celld.toml` (strict, errors name the line): `modem`, `sim`, `carrier` (a carrier lock: another carrier's SIM gives state `locked`), `channels = [1, 3]`, `roaming` (stored and shown; v2 cores have no partner networks, so a phone only uses its SIM's carrier either way), `autoconnect` (false: wait for `cell up`). |
| Control socket | UDP `127.0.0.1:7710` (loopback only), one request per datagram: `status` (`key value` lines: state, message, modem, profile, sim_carrier, imsi, carrier, cell, channel, rssi, sinr, cqi, bars, ip, gateway, dns, handovers, attaches, tx/rx bytes and packets, roaming, cause, uptime), `cells` (cells heard in the last 10 s: carrier, cell, profile, channel, RSSI, SINR, serving, why not usable; `none` when empty), `up`, `down`, `roaming on|off` (saved to the config). A second celld finds the port taken and exits. |
| Status bar | With `--phone`: `/dev/uictl cell <bars> <carrier|->` whenever bars or carrier change and every 5 s. Bars: 0 off a cell, else by the serving cell's RSSI (>= -80 dBm 4, -90 3, -100 2, -110 1). |
| Log | Every line to `/var/log/celld.log` (rotated at 64 KB); the interesting ones (not the search loop, no repeats) to stdout. |

States: `no_sim`, `locked`, `down`, `searching`, `waiting`, `attaching`, `connected`, `handover`, `idle`,
`detaching`, `rejected`, each with a sentence (`message`) for people.

**`cell`** (rust/wasm-programs/cell) talks to celld: `cell` (status: state, SIM, carrier and cell id, signal
bars, RSSI, SINR, CQI, address, traffic, handovers, roaming), `cell scan`, `cell up|down`, `cell roaming on|off`.
On a computer whose modem runs the v1 engine (only `lte_modem` / `mmwave_modem` peripherals) it shows the v1
pages as before; with a framed modem and no celld it says how to start it.

**Kernel primitive: `RTM_ECM_DNS`** (`0x7E11`, private netlink, `[ip 4 bytes]`): sets the resolver
(`Stack::set_dns_server`), not persisted; `net_config::dns_set` (a stub before) uses it.

### Phones

- `PhoneApps.tick` -> `PhoneUi.cellular(want)`: while a framed modem is in a module slot and a written SIM is in
  the tray, `PhoneUi` runs `celld --phone` (spawned detached, unconfined; again 5 s after it exited) and stops it
  when either goes (events `celld start pid N`, `celld stop pid N`). Terminals: the user runs `celld &`, or puts a
  `celld &` line in `/services.cfg` (the kernel's startup commands, ROUTER.md) to start it at boot.
- Status bar (the shell's existing source, `/dev/uictl phone`): `cell <bars>` now comes from celld's report for a
  framed modem (0 until it reports, stale after 12 s; v1 modems as before) and a new line `carrier <name|->`; the
  phone shell draws the carrier's name before the bars. A framed modem on a cell counts as an active radio for
  the battery.
- Settings is a built-in phone app (`dev.ecm.settings`, the `settings` program; `network`, `radio`, `cellular`,
  `packages` allowed once at install, revocable).

### Settings -> Cellular (ecm-apps)

On phones and Terminals: with celld running, the modem and the serving carrier with bars, State (celld's
sentence), SIM (carrier and IMSI, nothing secret), Cell, Signal (RSSI, SINR, CQI), Address, Data (sent,
received, handovers), a **Data roaming** switch (`roaming on|off` to celld), the cells heard (serving marked,
other carriers noted) and **Turn off / Turn on** (`down` / `up`). A framed modem without celld says to start it;
v1 modems show the v1 rows. `Sys::cell_command` sends celld requests (`MockSys` records them for tests).
Goldens `settings_cellular` (640x400) and `settings_cellular_compact` (320x200); `phone_home`, `phone_sheet` and
`phone_dialog` were re-recorded with a carrier in the status bar.

### Simulator: the phone agent is celld

`rust/simulator/src/cell.rs` runs `celld::Daemon` as the phone agent (`SimIo`: the node kernel's tun, netlink and
UDP sockets, the simulated medium; the SIM a `SoftSim`), with its control socket on the node's 127.0.0.1:7710, so
the real `cell` program typed at the node's prompt talks to it. Scenarios 30-40 pass unchanged on it (`cell admin
phone "attach|detach"` are `up` / `down`). New: `41_cell_celld_cli`: celld attaches, makes `wwan0` with a default
route and the core's DNS server; `cell` shows connected, the SIM, cell 1, signal with CQI and bars, the address
and DNS; `cell scan` lists the serving cell; ping; a walk to the second site hands over (same address, `cell`
counts it); `cell down` removes `wwan0` (control: the ping fails); `cell up` brings the same address back.

### Scenarios (in game)

| Scenario | |
|---|---|
| `cell_phone_celld` (GameTest `ecm_phone`) | A SIM written at a Card Reader/Writer with `simtool` and taken out by hand; a new ePhone; its slot screen (sneak-right-click) gets an LTE Modem and the SIM. The phone starts celld; its attachments are `module_2` (framed_radio) and `sim` (the tray), with no SIM slot of the modem's own; the status bar has `cell 0` and `carrier -`. In the phone's Terminal `cell` shows the SIM's identity (read through `sim`) and `searching`, `cell scan` hears nothing (no network is built). Control: the SIM out of the tray stops celld and `cell` says it isn't running. About 5 s. |
| `cell_phone_attach` (GameTest `ecm_cellular`) | The whole network, see "As built: phase 3 network side". |

### Tests

| Layer | Command | Covers |
|---|---|---|
| Rust | `scripts\Test.ps1 -Rust ecm-cell,celld,cell,terminal-simulator,ecm-apps -Scenarios _cell_` | celld config, control protocol, the daemon end to end on the medium with a site and a core (attach, `wwan0` + route + DNS, ARP, packets both ways, handover keeps `wwan0`, down/up keeps the address, no SIM -> card in -> card out, carrier lock, autoconnect off, roaming saved), `cell` arguments and pages, Settings -> Cellular (from celld's answers, roaming and power), simulator scenarios 30-44 |
| JUnit (26.1) | `scripts\Test.ps1 -JUnit SimPeripheralTest,UiGoldenTest -McVersion 26.1` | the `sim` peripheral never exposes K, answers the SimAuth vectors, empty tray; the goldens |
| GameTests | `scripts\Test.ps1 -GameTests ecm_phone,ecm_cellular,ecm_cards` | `cell_phone_celld` and the other phone tests; the v1 cellular scenarios on the pinned v1 engine; the card writer |

### What phase 4 deletes or rewires

- Delete the v1 engine: `ModemModule`, `ModemLink`, `TowerLink`, `CellTowerBlock*`, `CarrierRegistry`, `CellPackets`,
  the carrier screen, `CellRadio`, `SimVault` (move the modem slot's SIM to its own server-only storage), the kernel's
  `net/cell.rs` + `cell_*` host ABI (and `ComputerInstance.cellRadio()` / `createCellFunctions`), the v1 page of
  `cell` (`wasm-programs/cell/src/v1.rs`) and of Settings (`CellState` rows without `daemon`), `PhoneApps.cellBars`'s
  v1 branch and `PhoneHost.activeRadios`'s `CellRadio` case.
- Remove `modem.v1Engine`, `RadioConfig.overrideModemFramedRadio`, `CellularScenarios.V1_ENGINE`; rewrite the four
  v1 scenarios on cored / celld-site (`cell_lte_far`, `cell_mmwave_hill`, `cell_wrong_carrier`, `cell_roaming`,
  `cell_chat` as listed above).
- `ModemModuleItem` then always creates `FramedModemModule`; its sneak-use SIM removal keeps working on the same
  `sim` state entry.

## As built: phase 3 network side

Branch `gui/cell2-network`. The cell site and the carrier core are programs on computers, each with a CLI, a config
file, a loopback admin socket and an ecm-ui app; the site <-> core backhaul is sealed.

### Programs

| Piece | Where | What |
|---|---|---|
| `celld-site` | `rust/wasm-programs/celld-site` (lib `celld_site`: `config`, `daemon`; `prog.rs` is the WASI loop) | drives an Advanced SDR in framed BS mode (`ecm_radio::framed`, `phy::apply`) with the `ecm_cell` site machine; TCP 7700 to the core, UDP 7701 tunnels; the radio's slot clock is read twice a second and counted forward in between; framed status cached for `status` |
| `cored` | `rust/wasm-programs/cored` (lib `cored`: `config`, `daemon`) | the `ecm_cell` core machine; TCP 7700 listener, UDP 7701, tun `cell0` (address = the pool's gateway), the database file (rewritten on every `SaveDb`), SIM provisioning through the `card_reader` peripheral |
| `ecm-cell-net` | `rust/crates/ecm-cell-net` | the programs' socket plumbing: `AdminServer` / `AdminClient` / `admin_call`, TCP/UDP helpers, `WasiEntropy` (WASI `random_get`) |
| apps | `rust/crates/ecm-cell-ui` (`site`, `carrier`, `fake`, `testkit`), programs `cellsite`, `carrier` | Cell Site and Carrier Console |

**`celld-site` CLI**

```
celld-site config set <key> <value> [<key> <value>...]   writes /etc/celld-site.toml (every value checked)
celld-site config | config keys                            the file / every key with its meaning
celld-site start                                           run the site (background it: celld-site start &)
celld-site status | log [n] | restart | stop               through the admin socket
celld-site enable | disable                                `celld-site start &` in services.cfg (boot)
```

`/etc/celld-site.toml` (strict `key = value`, an unknown key or bad value is an error with its line): required
`site_id`, `secret` (32 hex), `carrier`, `core` (IPv4), `cell_id`; optional `profile` (lte | mmwave), `channel` (1-6 /
1-4), `power` (dBm or `auto` = the profile's BS power), `area`, `radio` (empty = the first SDR; `sdr_1`, an attachment
or a `/dev` path), `address` (hex, `auto` = `a0000000` + cell id), `core_port` 7700, `core_tunnel_port` 7701,
`tunnel_port` 7701, `neighbours` (`cell:channel[:profile],...`), `idle_after` (s), `admin_port` 7710.
Config changes apply on `restart` (a new site machine, radio reconfigured, phones released).

**`cored` CLI**

```
cored init <carrier> <pool> [--gateway IP] [--dns IP] [--force]   a new /etc/cored.db (gateway = the pool's first address)
cored start | stop                                       run (background it: cored start &) / stop the core
cored status | sites | subs | sessions | log [n] | handovers
cored site add <id>                                      a site with a random credential; prints the celld-site line to type
cored site remove <id> | site-add <id> <secret>
cored sim new [--label L] [--quota BYTES]                write the blank SIM in the Card Reader/Writer
cored add <imsi> <key> [quota] [label] | revoke | unrevoke | quota <imsi> <bytes> | reset-usage | remove | label | release
cored config | config set <key> <value> | config keys    /etc/cored.toml
cored enable | disable
```

With the core running, every command goes to it over the admin socket; without it, database commands (`add`,
`revoke`, `sim new`, `site add`, ...) run on the file directly (the same code: `cored::daemon::Daemon` over the parsed
database, the `SaveDb` output written back). `/etc/cored.toml` (all optional): `db` (/etc/cored.db), `listen` 7700,
`tunnel_port` 7701, `tun` cell0, `admin_port` 7711, `imsi_prefix` 00101 (MCC 001 MNC 01, the test network),
`reader` (empty = the first Card Reader/Writer). The database format gained an optional `label=` per subscriber
(1-32 of `a-z A-Z 0-9 . _ -`); admin commands gained `remove`, `label`, `site-remove`.

**SIM provisioning** (`cored sim new`, the console's Write SIM): the reader must hold a blank SIM (else: no card / already
written / not a SIM); a random IMSI under `imsi_prefix` not in the database and a random K (WASI `random_get`) are
added to the database first, then `card_reader.write(imsi, carrier, K)` (which releases the card: Auto Eject Sim
pushes it out, otherwise it waits to be taken). If the write fails the subscriber is removed again. K is never printed
or sent to the apps: it exists only in the database and on the card.

**Routing.** Phones' traffic enters the core's kernel on `cell0`; to reach the LAN the core forwards (the router
service: `router`, `configure terminal`, `ip routing`, interface addresses, `end`) and the LAN routes the pool to the
core (a static route, e.g. `ip route add 100.65.2.0/24 via <core> dev eth1`, or BGP). For NAT instead of a route,
the same router CLI's NAT commands on the core's LAN interface (as in the router scenarios); not exercised here.

### Admin socket

TCP on 127.0.0.1 only (`celld-site` 7710, `cored` 7711), line-based text (`ecm_cell::admin`): a request is one
command line; a reply is `ok` or `err <message>`, body lines, and `.` alone (a body line starting with `.` gets one
more, as SMTP). Any number of requests per connection. Bodies are `key value` lines plus list lines:

| Daemon | Command | Body |
|---|---|---|
| celld-site | `status` | `site carrier cell profile channel power address radio core link link_error uptime_s phones ul_bytes dl_bytes backhaul_rejects ul_mask radio_*`, then `ue <addr> <imsi> <state> <ip> <rssi> <cqi> <sub> <bytes>`, `nb <cell> <profile> <channel> <ul_mask>` |
| celld-site | `config`, `set <key> <value>`, `restart`, `stop`, `log [n]`, `radio` | the file; `saved <key>`; ... |
| cored | `status` | `carrier pool gateway db listen tunnel_port tun uptime_s subscribers sites_configured sites_connected sessions tunnel_rejects`, then `site <id> <peer> <cells> <up s> <hb ms ago> <rejected>`, `session <id> <imsi> <ip> <state> <site> <cell> <used> <quota>` |
| cored | `subs` / `sites` / `sessions` | `sub <imsi> <ip> <used> <quota> <revoked> <label> <online>` / `site <id> <connected> <peer> <cells> <hb> <up> <rejected>` per configured site |
| cored | `sim-new [label] [quota]`, `site-new <id>`, `log`, `handovers`, the database commands, `stop` | `imsi/label/ejected`; `site/secret`; ... |

Parsers for the apps: `celld_site::daemon::StatusView`, `cored::daemon::{parse_subs, parse_sites, parse_sessions}`.

### Backhaul security as built

Version 2 of the site <-> core protocol (`backhaul::VERSION`):

1. core -> site `CoreHello(version, carrier, core_nonce)` (plain);
2. site -> core `SiteAuth(version, site_id, cells, tunnel_port, site_nonce, proof)` (plain); the proof is
   `HMAC(site_secret, "ECM-SITE-AUTH" + CoreHello body + SiteAuth body up to the proof)` (`auth_transcript`), so a
   box in between can change none of it (cells, port, nonces, the core's hello);
3. both derive `K_BH` from the secret and both nonces; from here every control message both ways is
   `[u32 len][0x7F][counter u64][ChaCha20-Poly1305(type + fields)]`, associated data `0x7F + counter + dir`, nonce
   `dir u8, 0 0 0, counter u64` (dir 1 site -> core, 2 core -> site); TCP is in order, so the counter must be exactly
   the next one (a replayed, dropped or reordered message closes the connection);
4. the core's first sealed message is `SiteAccept`: it proves the core holds the credential (a site that can't open
   it says "the core failed to prove the site credential" and backs off 10 s). Only `SiteReject` stays plain (the core
   has no key for an unknown site or a bad proof);
5. tunnels: `[0xE2][0][0 u16][counter u64][ChaCha20-Poly1305(the 0xE1 tunnel packet)]` under the tunnel key, a
   64-datagram replay window per direction (UDP reorders); the core finds the connection by the sender's address and
   only accepts a session's packets from its serving (or handover target) site's key, then checks the inner source
   address. 28 more bytes per datagram: one carries 1432 bytes of IP (bigger packets go in two, as before).

Failures are counted (`SiteInfo.rejected`, `Core::tunnel_rejects`, `SiteStatus::backhaul_rejects`) and logged at most
every 2 s. `BhChannel` / `Backhaul` are in `ecm_cell::crypto`; the machines take and give bytes (`Site::on_bytes`,
`SiteOut::Send`, `CoreOut::Send(conn, bytes)`), and `Site::with_entropy` takes the source of the site nonce.

### Apps

Installed as **system apps** (`SystemApps`), but only on the computers that run the service (`App.requires`): Cell Site
(`dev.ecm.cellsite`, program `cellsite`) once `/etc/celld-site.toml` exists, Carrier Console (`dev.ecm.carrier`,
program `carrier`) once `/etc/cored.db` or `/etc/cored.toml` exists. Both declare `network` (allowed at install) for
the loopback admin socket. `celld-site config set` and `cored init` ask the app manager to reindex, so the app appears
at once. Icons: `assets/make_icons.py` in ecm-cell-ui.

- **Cell Site**: header with the core link (up / registering / down) and phone count; Overview (radio: profile,
  channel, power, cell, address, frames, beacons; core link: address, state, last error, rejected; traffic: phones,
  uplink, downlink, uptime, sub-channels), Phones (each phone's IMSI, state, address, RSSI, CQI, sub-channel, bytes;
  the neighbours heard), Settings (site id, carrier, core, cell id, profile, channel, power, credential; Save sends
  `set` for each change, Restart applies them). Compact: tabs; medium: tabs, overview cards two to a row; wide: all
  three side by side.
- **Carrier Console**: Subscribers (New SIM: label, quota, Write SIM; the list with label, IMSI, address, usage of
  quota, revoked / online / over quota; for the chosen one Revoke or Reinstate, Set quota, Release), Sites (connected
  or not, from where, cells, last heartbeat, uptime, rejected; Add site prints the credential and the `celld-site`
  line), Phones (live sessions with site and cell), Handovers (the core's handover log), Core (settings and counters).
  Compact and medium: tabs; wide: a sidebar.

Golden images (looked at): `cellsite_compact` (320x480, overview), `cellsite_medium` (640x400, phones),
`cellsite_wide` (1024x640), `carrier_compact` (subscribers), `carrier_medium` (sites after Add site),
`carrier_wide` (subscribers after Write SIM, one chosen). Recorder:
`cd rust; cargo run -p ecm-cell-ui --example record_cell_sessions -- ../src/test/resources/ui-sessions`.

### Scenarios

| Scenario | Player steps |
|---|---|
| `cell_site_core` (GameTest `ecm_cellular`, ~15 s) | Built: server and core cabled at their bottoms (10.0.0.0/24), core and site at their tops (10.0.1.0/24), a Card Reader/Writer on the core's east side, beside the site and a probe computer an Advanced SDR with coax up a two-block mast and an LTE Sector Antenna on top. Typed: `ifconfig` on each; `cored init cell-red 100.65.2.0/24`; `cored site add site-a` (the credential read off the screen); `cored start &`; **control**: `celld-site config set ...` with the credential's last digit mistyped, `celld-site start &`: `celld-site status` says `core refused this site: authentication failed`, `cored log` says `bad credential`; the right credential, `celld-site restart`: `link up`, `cored sites` lists site-a connected; **control**: `cored sim new --label alice` with the reader empty says no card; a blank SIM clicked into the reader, `cored sim new --label alice` writes it (the card's key is the database's), taken out by hand; the probe: `framed config --phy lte --role ue --channel 3`, `framed recv` hears beacons from `a0000001` carrying `cell-red`, ~10 a second; `gui`, Apps, Carrier Console: subscriber alice, Sites: site-a connected. |
| `cell_phone_attach` (GameTest `ecm_cellular`, ~10 s) | Built: core cabled (bottom) to a cell site and (top) to a server, a Card Reader/Writer on the core, the site's SDR, coax and LTE panel. A new ePhone into the off hand (it boots meanwhile). Typed: server address and `ip route add 100.65.2.0/24 via 10.0.41.1 dev eth1`; on the core the router CLI (addresses, `ip routing`); `cored init`, `cored site-add site-a <secret>`, `cored start &`; `celld-site config set ...`, `celld-site start &`, `link up`; a blank SIM in the reader, `cored sim new --label phone`, taken out; the phone in hand, 17 blocks east of the antenna, LTE Modem and SIM fitted in its slot screen: celld attaches, the status bar shows cell-red, `cell` says connected, the phone pings the server through the core, `cored status` lists the phone's session at site-a cell 1. |

### Tests

| Layer | Command | Covers |
|---|---|---|
| Rust | `scripts\Test.ps1 -Rust ecm-cell,celld-site,cored,ecm-cell-ui,terminal-simulator -Scenarios _cell_` | sealed control in order (replay, reorder, tamper, wrong credential), tunnels (reorder ok, replay and tamper refused), the proof's transcript, the admin framing and config lines; e2e `tampered_backhaul_is_rejected` (tampered and forged tunnel datagrams, a tampered control message: closed, the site re-registers, same address); celld-site config/status/commands; cored init, `sim-new` (blank / no card / written / failed write leaves nothing), sites, database commands; both apps over the real daemon logic and scripted ones at every size; simulator `43_cell_backhaul_tamper` (a box flips a bit in every tunnel datagram with checksums fixed: pings fail, the core counts them; then in the control connection: closed, re-registered, the phone back with its address) and `44_cell_cored_program` (the real `cored` WASI program typed at a simulated computer: init, site-add, add, start; a site and a phone attach, ping through cell0; `cored sites/subs/status`; control: revoke releases the phone). `link set ... tamper=N tamper_port=P` is the new simulator fault. |
| JUnit (26.1) | `scripts\Test.ps1 -JUnit UiGoldenTest -McVersion 26.1` | the six goldens above |
| GameTests | `scripts\Test.ps1 -GameTests ecm_cellular,ecm_cards` | `cell_site_core`, `cell_phone_attach`, the v1 cellular tests, the card writer |

### For phase 4

- Rewrite `cell_lte_far`, `cell_mmwave_hill`, `cell_wrong_carrier` on `cored` / `celld-site` (the two scenarios
  above show the building blocks: `CellNetScenarios.buildStation`, the router CLI steps, `cored sim new`).
- Tech Villages (phase 5): one `cored` per carrier (`cored init`, `cored site-add` for every village's site, SIMs
  preloaded with `cored add <imsi> <key> 0 <label>` at generation) and `celld-site` with a written
  `/etc/celld-site.toml` + `celld-site enable`.
- `cored` hands out the pool's DNS (`cored init --dns`); celld sets it with `RTM_ECM_DNS`.
- Not done: NAT on the core is documented, not tested; one cell per site; the site's TCP connect blocks up to the
  kernel's 10 s when the core's host doesn't answer (beacons go on, offloaded to the radio).
