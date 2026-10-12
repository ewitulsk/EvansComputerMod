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
| **SDR framed PHY mode** | new | See below. Documented as an addition to the SDR blocks, with when to use it. |
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
protocol logic of their own.

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
