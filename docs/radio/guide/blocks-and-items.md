# Every radio block and item

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

The radio feature adds **30 blocks and items**: 29 with recipes and 1 drop-only item (Melted
Scrap). All are in the mod's creative tab ("Evans Computer Mod") except Melted Scrap, and the
Burner Generator when it is disabled by config. Every radio block is mined with a pickaxe and
drops itself (block settings such as SDR tuning, an Access Point's configuration, a wire's
oxidation or wrench cuts are **not** kept on the dropped item).

Recipes use common tags where they can (`c:ingots/copper` accepts any mod's copper ingot, and so
on). In the recipe grids below, `·` is an empty slot.

| Group | Blocks / items |
|---|---|
| [Computer-side radios](#computer-side-radios) | SDR (Basic / Standard / Advanced), Wi-Fi Module, Controller Receiver Module |
| [Wi-Fi](#wi-fi) | Wi-Fi Access Point |
| [Antenna building](#antenna-building) | Copper Wire, Antenna Wire, Heavy Cable, Antenna Rod, Lattice Mast, Insulator, Feed Point, Fine Wire |
| [Feedline](#feedline) | Coax Cable, Hardline, Lightning Arrestor |
| [Transmit power](#transmit-power) | Power Amplifier (100 W / 1 kW / 10 kW), Antenna Tuner, Burner Generator |
| [Tools](#tools) | RF Wrench, Antenna Analyzer, RF Meter |
| [Microwave](#microwave) | Microwave Radio, Dish (0.6 m / 1.2 m / 2.4 m) |
| [Handheld](#handheld) | Handheld Radio |
| [Leftovers](#leftovers) | Melted Scrap |

How blocks connect to a computer: a computer sees **peripheral blocks that touch one of its six
faces** (named by side relative to the computer's facing: `front`, `back`, `left`, `right`,
`top`, `bottom`) and **modules in its bays** (`left_bay_1`, `left_bay_2`, `right_bay_1`,
`right_bay_2`). Run `peripherals` to list them. Nothing is found through cables.

---

## Computer-side radios

### SDR (Basic), SDR (Standard), SDR (Advanced)

`sdr_basic`, `sdr_standard`, `sdr_advanced` · block · block entity `sdr` · peripheral type `sdr`

A software-defined radio: the computer gets raw IQ samples of whatever the world's radio medium
delivers at its antenna, and (Standard/Advanced) can transmit any waveform it writes. Every SDR
program and the Python `radio` module work through it. Full protocol: [SDR](sdr.md).

| | Basic | Standard | Advanced |
|---|---|---|---|
| Tuning range | 0.5 – 1700 MHz | 10 kHz – 6 GHz | 1 kHz – 6 GHz |
| Max sample rate | 48 kS/s | 250 kS/s | 1 MS/s (250 kS/s when computers run on the Chicory interpreter) |
| ADC | 8-bit | 12-bit | 16-bit |
| Noise figure | 8 dB | 5 dB | 5 dB |
| Transmit | **receive only** | up to 37 dBm (5 W) | up to 37 dBm (5 W) |

- **Place** it touching a computer (any face of the SDR, any face of the computer). Facing does
  nothing functionally.
- **Right-click**: action-bar status `<name>: <MHz> MHz, <rate> S/s, gain <dB> dB`, e.g.
  `SDR (Standard): 146.5200 MHz, 48000 S/s, gain 30 dB`.
- **Antenna**: on its own it uses a built-in vertical whip (half-wave dipole pattern, 0.6 block
  above the block). If a **Coax Cable/Hardline** run, an **amplifier**, a **tuner** or a **Feed
  Point's coax side** touches any face of the SDR, it uses that chain instead, both to receive
  and to transmit: the antenna is then the built antenna at the end of the feedline
  ([Antennas](antennas.md), [Power](power-and-hazards.md)). A chain that ends in nothing
  ("open feedline") hears almost nothing.
- **Power**: none. Only amplifiers in the chain use FE.
- **Saved**: frequency, sample rate, gain and AGC survive saves and ship assembly; bandwidth,
  sample format, transmit on/off and transmit power do not.
- **Recipes** (3×3):

  | Basic | Standard | Advanced |
  |---|---|---|
  | `iron copper iron` / `redstone quartz redstone` / `iron iron iron` | `gold copper gold` / `redstone SDR(Basic) redstone` / `gold amethyst gold` | `diamond copper diamond` / `redstone SDR(Standard) redstone` / `diamond ender_pearl diamond` |

  (tags: `c:ingots/iron`, `c:ingots/copper`, `c:dusts/redstone`, `c:gems/quartz`,
  `c:ingots/gold`, `c:gems/amethyst`, `c:gems/diamond`, `c:ender_pearls`)
- **Ships**: works on Sable ships; the whip follows the ship.

### Wi-Fi Module

`wifi_module` · item, stacks to 1 · bay module · peripheral type `wifi`

The computer's `wlan0`: a 2.4/5 GHz 802.11 radio with a SoftMAC (ACKs, retries, CSMA/CA), driven
by the computer's kernel and the programs `wifi`, `iw`, `wpa_supplicant`, `wpa_cli`, `tcpdump`.
It can instead act as a Wireless Controller receiver. Details: [Wi-Fi](wifi.md).

- **Install**: right-click a side of a computer (not the screen) with a **Module Expansion
  Card** first (opens a two-slot bay on that side), then right-click the same side with the
  module. Without the card: "Install a Module Expansion Card first".
- Antenna: vertical dipole (2.15 dBi) at the computer's centre; sensitivity −92 dBm; transmit
  power 0–20 dBm.
- Peripheral methods: `get_mode()`, `set_mode("wifi"|"controller")`, `get_channel()`,
  `set_controller_channel(1..13)`, `set_max_power(0..20)`, `get_mac()`, `stats()` (see
  [Programming](programming.md#wi-fi-module-wifi)).
- Recipe: `· lightning_rod ·` / `gold_nugget comparator gold_nugget` / `iron_ingot quartz iron_ingot`
  (plain items).

### Controller Receiver Module

`controller_receiver_module` · item, stacks to 1 · bay module · peripheral type `controller_receiver`

A 2.4 GHz receiver for the Wireless Xbox Controller. A computer needs one (or a Wi-Fi Module in
controller mode) before a controller can drive it. Details:
[Handheld, controller, microwave](handheld-controller-microwave.md#2-wireless-controller).

- Install like the Wi-Fi Module (expansion card first).
- Peripheral: `get_channel()`, `set_channel(1..13)` (paired controllers follow), `stats()`.
- Recipe: `· lightning_rod ·` / `gold_nugget comparator gold_nugget` / `iron ender_pearl iron`
  (`c:ingots/iron`); like the Wi-Fi Module's, with the Wireless Controller's ender pearl.

---

## Wi-Fi

### Wi-Fi Access Point

`access_point` · block · block entity + menu `access_point` · radio endpoint

An 802.11a/g access point with WPA2-PSK (or open) that bridges wireless clients onto the network
cable it sits on (a layer-2 bridge, like a real AP). Full chapter: [Wi-Fi](wifi.md).

- **Place** touching a **Network Cable** (or an Internet Gateway). It joins the first touching
  cable segment (checked down, up, north, south, west, east). The block's lit LEDs (blockstate
  `active=true`) mean only "cabled into a segment", not "clients connected".
- **Right-click**: opens the GUI (Settings and Status tabs) if you may configure it (owner, an
  operator, a creative player, or nobody owns it yet); otherwise "This access point belongs to
  <name>". You must be within 8 blocks.
- **Sneak + right-click with an RF Wrench** (or any `c:tools/wrench`): factory reset to an open
  network named `ECM-XXXX` (last two bytes of its BSSID); the passphrase is erased and you
  become the owner.
- Built-in antenna: vertical omni, 5 dBi at the horizon, nulls straight up/down; 0.3 block above
  the block centre. Transmit power 0–20 dBm (GUI steps 0, 3, 6, 10, 13, 15, 17, 20). Sensitivity
  −94 dBm.
- No FE needed.
- Recipe: `stick · stick` / `quartz copper quartz` / `iron redstone iron` (`c:rods/wooden`,
  `c:gems/quartz`, `c:ingots/copper`, `c:ingots/iron`, `c:dusts/redstone`).
- Ships: its settings, passphrase and cable segment survive Sable assembly, flight, landing and
  reload (GameTest `ship_round_trip_keeps_radio_state`).

---

## Antenna building

All of these are **conductor blocks**: they connect on all six sides to neighbours of the right
kind, can float in mid-air, and are waterloggable. They join into an antenna only through a
**Feed Point**. Full chapter, with build recipes: [Antennas](antennas.md).

| Block | Recipe (output) | Thickness | Collision | Electrical model | Rated power (resonant dipole, approx.) |
|---|---|---|---|---|---|
| **Copper Wire** `copper_wire` | `copper copper copper` → **16** | 2 px | none (walk through) | 1.0 mm radius copper, 0.85 A | ~50 W |
| **Antenna Wire** `antenna_wire` | 8 Copper Wire around 1 iron nugget → **8** | 3 px | none | 1.6 mm copper, 1.65 A | ~200 W |
| **Heavy Cable** `heavy_cable` | `copper×3` / `iron_nugget×3` → **4** | 5 px | yes | 5 mm copper, 5.2 A | ~2 kW |
| **Antenna Rod** `antenna_rod` | 3 iron ingots in a column → **4** | 7 px | yes | 12.5 mm aluminium, 11.7 A | ~10 kW |
| **Lattice Mast** `lattice_mast` | iron in an X → **4** | 15 px, four corner legs; climbable | legs only | 200 mm iron, 26 A | ~50 kW |
| **Insulator** `insulator` | `· iron_nugget ·` / `white_terracotta×3` → **4** | 6 px | yes | joins wires mechanically, **not** electrically; 4 kV rating | — |
| **Feed Point** `feed_point` | `copper_wire insulator copper_wire` / `· coax_cable ·` → **1** | 8 px + lugs | yes | where the radio connects: a 3 mm gap, 3 kV, 30 A | — |

- **Copper oxidizes**: Copper Wire, Antenna Wire and Heavy Cable weather like copper blocks
  (random ticks: unaffected → exposed → weathered → oxidized), raising their resistance ×1.6,
  ×2.6, ×4.0 (lower efficiency, lower power rating). **Honeycomb** waxes them (stops it); an
  **axe** removes wax, then one oxidation stage per use.
- **Waterlogged** conductors have ×20 resistance.
- Blockstate: six connection booleans + `waterlogged`; copper tiers add `oxidation` 0–3 and
  `waxed`; the Feed Point adds `axis` (x/y/z).
- Other conductive blocks: anything in `#evanscomputermod:rf_conductors` that touches a wire
  joins the antenna (lightning rod, iron bars, chain, iron/copper/gold blocks, iron door and
  trapdoor, cut copper, waxed copper block; with Create, girders, casings and metal blocks; with
  Aeronautics, the smart propeller). They become part of the antenna and **detune** it.

### Feed Point

`feed_point` · block · block entity `rf_conductor` · peripheral type `antenna`

- It has an **axis** (shown by its lugs). The two faces along the axis are the antenna
  terminals ("bare" sides: wire arms attach here); the other four faces are its **coax port**
  (Coax Cable, Hardline, an amplifier, a tuner, an SDR or a computer attach here).
- When placed, the axis is chosen: the clicked face's axis if a wire block is already on either
  side along it, else X, Z, Y, whichever has a wire neighbour; with no wire neighbours, the
  clicked face's axis (like placing a log: clicking the top of a block gives a vertical feed).
- **A vertical (Y) feed point sitting directly on the ground** (a solid non-conductor or water
  block right under it, nothing connected below) feeds a vertical wire **against the ground**:
  a quarter-wave monopole.
- **Fine Wire** attaches to its two lugs (terminal 0 = negative-axis side, 1 = positive), for
  small VHF/UHF antennas.
- A computer touching it gets the `antenna` peripheral; the Antenna Analyzer reads it.
- Breaking it breaks any Fine Wire attached to it.

### Fine Wire (formerly Sensor Wire)

`sensor_wire` · item (places a wire entity) · shared with the sensor feature

Thin routed wire (0.25 mm copper, ~5 W). Click a connector (a feed point lug, a sensor or
module connector), then block faces to route it, then another connector or wire; sneak-click
the air to cancel. Costs half an item per metre. For radio it is used to build VHF/UHF dipoles
on a feed point's lugs; it **joins block wires only at a feed point**. Recipe: shapeless copper
ingot + redstone + string → 8.

---

## Feedline

Coax-type blocks connect only to each other's coax sides and to coax ports (Feed Point's four
side faces, amplifiers, tuner, SDRs: tag `#evanscomputermod:rf_coax_ports`). They never join an
antenna's bare wires.

| Block | Recipe (output) | Loss per 10 blocks at 7 MHz / 146 MHz / 1 GHz | Power rating (≤10 MHz; falls as 1/√f above) |
|---|---|---|---|
| **Coax Cable** `coax_cable` | `wool copper wool` → **6** | 0.41 / 2.08 / 6.6 dB | 600 W (157 W at 146 MHz) |
| **Hardline** `hardline` | 8 copper in a ring → **4** | 0.017 / 0.08 / 0.23 dB | 20 kW |
| **Lightning Arrestor** `lightning_arrestor` | `· lightning_rod ·` / `coax iron coax` → **1** | 0.04 / 0.25 / 1.0 dB | 5 kW |

- A **Lightning Arrestor** is a grounded coax block: put one anywhere in the run between the
  antenna and the first device and a lightning strike on the antenna is grounded instead of
  destroying the radio ([Power and hazards](power-and-hazards.md#lightning)).
- One run is traced up to 512 blocks; at a fork only the first branch counts.

---

## Transmit power

### Power Amplifier (100 W), (1 kW), (10 kW)

`amplifier_100w`, `amplifier_1kw`, `amplifier_10kw` · block · block entity `amplifier` · FE storage · peripheral type `amplifier`

Goes between an SDR (the 5 W "exciter") and the antenna: touching the SDR or connected by coax,
with coax onward to a tuner or feed point. Amplifies only while the SDR transmits and only then
draws FE. Full chapter: [Power and hazards](power-and-hazards.md).

| | 100 W | 1 kW | 10 kW |
|---|---|---|---|
| Gain at full 5 W drive | ×20 (13 dB) | ×200 (23 dB) | ×2000 (33 dB) |
| FE/t at full output (1 FE/t = 5 W, 50% efficient) | 40 | 400 | 4,000 |
| FE buffer | 16,000 | 160,000 | 1,600,000 |
| Max FE input per receive | 160 | 1,600 | 16,000 |
| SWR protection | **none** (can burn out) | folds back | folds back |
| Recipe | `iron quartz iron` / `copper redstone copper` / `iron iron iron` | `gold quartz gold` / `amp100 amp100 amp100` / `copper_block redstone_block copper_block` | `diamond gold_block diamond` / `amp1k amp1k amp1k` / `copper_block redstone_block copper_block` |

- Right-click: status lines in chat (output, temperature, FE, SWR, reflected power, warnings).
- Accepts FE on any face from any FE source (Burner Generator, other mods' generators/cables).
  Other blocks cannot pull FE out of it.

### Antenna Tuner

`antenna_tuner` · block · block entity `antenna_tuner` · no peripheral, no FE

Inline in the coax between amplifier and antenna. Matches any antenna whose SWR (as seen at the
tuner) is between 1:1 and 30:1, so the amplifier sees ~1:1; the mismatch power becomes heat in
the tuner (rated 600 W of mismatch). It does not make a short antenna efficient. Right-click:
`Antenna Tuner: antenna SWR 1.8:1 matched to 1.0:1, absorbing 6 W · 20 °C (rated 600 W of mismatch)`.
Recipe: `iron quartz iron` / `coax_cable redstone coax_cable` / `iron iron iron`.

### Burner Generator

`burner_generator` · block · block entity `burner_generator` · FE source

Burns any furnace fuel for **40 FE/t** (config `power.burnerGenerator.fePerTick`), stores
40,000 FE and pushes up to 400 FE/t into every touching FE receiver on all six sides.

- Right-click with fuel: inserts the stack (one slot; hoppers work too). Right-click empty-handed:
  `Burner Generator: 0 / 40000 FE, 0s of fuel left, 40 FE/t while burning` on the action bar.
- One coal = 1600 ticks × 40 = 64,000 FE. Fuel is only taken while the buffer isn't full.
- Blockstate `lit` (light level 13, flame and smoke particles, furnace crackle).
- Config `power.burnerGenerator.enabled = false`: no recipe (recipe conditions run before the
  per-world server config loads, so the recipe is also removed once the server has started:
  already on a world's first load), not in the creative tab, existing generators stop burning
  (and say `Burner Generator: disabled by server config`) and deliver no FE at all, stored FE
  included (neither pushed nor extractable).
- Recipe: `iron iron iron` / `iron furnace iron` / `copper redstone copper` (with the condition
  `evanscomputermod:radio_feature_enabled`).

---

## Tools

### RF Wrench

`rf_wrench` · item, stacks to 1 · in `#evanscomputermod:rf_wrenches` (with `#c:tools/wrench`)

- **Right-click a conductor's arm** (or the face of its centre box) to **cut** that connection;
  click again to **restore** it. Action bar: `Cut the east connection` / `Restored the east connection`.
  The cut is stored in the block (survives saves and ship assembly) and both blocks drop the arm.
  Works on every wire, rod, mast, insulator, feed point, coax, hardline and arrestor.
- **Sneak + right-click an Access Point**: factory reset.
- Recipe: `iron · iron` / `· copper ·` / `· iron ·`.

### Antenna Analyzer

`antenna_analyzer` · item, stacks to 1

- **Right-click a Feed Point**: chat summary, details and feedline line. Example (the
  `ham_station` dipole):

  ```
  Resonant at 7.2 MHz · 2:1 SWR band 7.0–7.4 MHz · rated 228 W (antenna wire) / 2.2 kW (insulators)
  dipole · Z 78.2 - j1.4 Ω · SWR 1.6:1 · efficiency 75% · gain 6.0 dBi · 20.0 m of wire · 12 segments · ground: average soil, 10 m below · limited to 228 W by antenna wire
  Feedline: 8 blocks, 0.29 dB at 7.2 MHz · lightning arrestor fitted
  ```

  While the solver is still working it first prints an estimate prefixed `≈` and ending in
  `(estimate: solving)`, then the solved result when it lands.
- **Sneak + right-click a Feed Point**: the SWR plot screen ("Antenna Analyzer - SWR").
- On another conductor: `Use the analyzer on a feed point`.
- Recipe: `glass_pane quartz glass_pane` / `copper redstone copper` / `iron iron iron`.

### RF Meter

`rf_meter` · item, stacks to 1

Hold it (main or off hand): every half second the action bar shows the field strength from
transmitting SDR chains at your eyes, e.g.
`RF meter: 3.21 V/m · -18.4 dBm (0 dBi) · 7.100 MHz · limit 86 V/m` (green under 50% of the
exposure limit, gold from 50%, red at or above), or `RF meter: no transmitter in range`.
Only SDR transmissions (with or without an amplifier chain) count; Access Points, Wi-Fi Modules
and controllers are not shown. Recipe: `· copper ·` / `gold redstone gold` / `gold iron gold`.

---

## Microwave

Full chapter: [Handheld, controller, microwave](handheld-controller-microwave.md#3-microwave-links).

### Microwave Radio

`microwave_radio` · block · block entity `microwave_radio` · peripheral type `microwave_radio`

A point-to-point Ethernet bridge on 10, 24 or 60 GHz: place it touching a Network Cable (joins
that segment) and touching a Dish. Two radios on the same band, width and channel whose dishes
see each other bridge their two cable segments.

- Right-click: action bar `Microwave radio: <band> GHz ch <n> (<width> MHz), dish connected|no dish, <RSSI> dBm, <modulation>, <rate> Mbit/s`
  (or `no link` at the end). The value in parentheses is the channel **width**, not the frequency.
- Sneak + right-click: next channel; after the last channel, the next band (default width, ch 0).
- Defaults: 24 GHz, 56 MHz, channel 0, 20 dBm (range −40 to +30 dBm via the peripheral).
- Recipe: `iron copper iron` / `redstone quartz redstone` / `iron gold iron`.

### Dish (0.6 m), Dish (1.2 m), Dish (2.4 m)

`dish_small` (1×1), `dish_medium` (2×2), `dish_large` (3×3) · multiblock · block entity `dish` · peripheral type `dish`

- Placed as a vertical square facing the way you look; all parts or none (fails silently if
  any space is blocked). Breaking any part removes the whole dish; it drops one item by the
  usual rules (a pickaxe in survival; nothing bare-handed or in creative).
- Right-click: action bar `Dish (1.2 m): yaw -90.00°, pitch 0.00° (<radio>)` where `<radio>` is
  `no radio`, `<band> GHz ch <n>, RSSI <x> dBm` or `<band> GHz ch <n>, no link`.
- Sneak + right-click near an edge of the face: nudge the aim that way (half a beamwidth when a
  radio is attached, 1° otherwise).
- The model does not turn with the aim (the beam does).
- Recipes: small `iron iron iron` / `iron copper iron` / `· iron ·`; medium `small iron small` /
  `iron · iron` / `small iron small`; large `· medium ·` / `medium iron_block medium` / `· medium ·`.

---

## Handheld

### Handheld Radio

`handheld_radio` · item, stacks to 1

A receive-only AM / shortwave / VHF FM radio you hold. Right-click: on/off. Sneak + right-click:
tuning screen (frequency box, dial buttons, band, scan, volume, squelch). Audio plays through
the **Jukebox/Note Blocks** volume slider. Works only while held (either hand). Details:
[Handheld](handheld-controller-microwave.md#1-handheld-radio).

Recipe: `· lightning_rod ·` / `iron note_block iron` / `copper quartz copper` (`c:ingots/iron`,
`c:ingots/copper`, `c:gems/quartz`).

---

## Leftovers

### Melted Scrap

`melted_scrap` · item · not in the creative tab

Dropped where a hazard destroys something: a melted wire, melted coax, a burnt-out amplifier or
tuner, or a radio killed by lightning. Smelts into 1 iron nugget (0.1 XP).

---

## Related non-radio blocks used here

| Block / item | Why it matters for radio |
|---|---|
| Terminal (computer) | runs the programs; SDRs and feed points touch it; modules go in its bays |
| Module Expansion Card | needed before any bay module (Wi-Fi Module, Controller Receiver) |
| Network Cable | Access Points and Microwave Radios sit on it |
| Speaker | `rx_fm`/`rx_am`/`rx_ssb` and Python `radio.speaker()` play through it |
| Screen | `waterfall --screen`, `antenna ... --screen` |
| Wireless Xbox Controller | the 2.4 GHz controller (recipe `stone_button ender_pearl stone_button` / `iron redstone iron`) |
