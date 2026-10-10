# Power, amplifiers and hazards

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

An SDR (Standard or Advanced) transmits at most **5 W (37 dBm)**. For more you add an
**amplifier**, which needs **FE** (Forge Energy). More power means more heat in the antenna,
coax and amplifier, and mistakes have consequences, always with a warning stage first.

Power units: **1 FE/t = 5 W** of DC input (server config `power.wattsPerFePerTick`, default 5.0).
Amplifiers are 50% efficient, so each watt of extra RF costs 2 W of DC.

## 1. The Burner Generator

| | |
|---|---|
| Fuel | Anything that burns in a furnace (coal, charcoal, wood, blaze rods, lava buckets...). One slot |
| Output | **40 FE/t** while burning (`power.burnerGenerator.fePerTick`) |
| Buffer | 40,000 FE |
| Pushes | up to 400 FE/t into **every** touching block that accepts FE, all six sides |
| Coal | 1600 ticks × 40 = 64,000 FE (80 s) |

- Right-click with fuel to insert it (creative players keep their stack). Hoppers work too.
- Right-click empty-handed: action bar `Burner Generator: <stored> / 40000 FE, <n>s of fuel left, 40 FE/t while burning`
  (real, before fuelling: `Burner Generator: 0 / 40000 FE, 0s of fuel left, 40 FE/t while burning`).
- It takes new fuel only while the buffer isn't full; when full, the current item keeps
  burning and its output is wasted.
- `lit` blockstate: light 13, flame and smoke, furnace crackle.
- Breaking it drops the fuel; stored FE is lost.
- **Disabling it** (`power.burnerGenerator.enabled = false` in `evanscomputermod-server.toml`):
  the recipe disappears (recipe condition `evanscomputermod:radio_feature_enabled`), it's not in
  the creative tab, existing generators stop burning and say
  `Burner Generator: disabled by server config`, but they still push out FE they had stored. Use
  any other mod's FE generator instead (amplifiers accept FE from anything).

Note: the recipe condition reads a server config, and NeoForge loads datapacks before server
configs on a world's first load, so the recipe may still be present until `/reload`.

## 2. Amplifiers

### Tiers

| | Power Amplifier (100 W) | (1 kW) | (10 kW) |
|---|---|---|---|
| Rated output | 100 W | 1,000 W | 10,000 W |
| Gain (rated ÷ 5 W drive) | ×20 (13 dB) | ×200 (23 dB) | ×2000 (33 dB) |
| FE/t while transmitting at full output | 40 | 400 | 4,000 |
| FE buffer | 16,000 | 160,000 | 1,600,000 |
| FE accepted per transfer | 160 | 1,600 | 16,000 |
| High-SWR protection | **none** | folds back | folds back |

### Wiring it

```
SDR (Standard/Advanced)  ─ touching, or coax ─  Amplifier  ─ coax ─  [Antenna Tuner] ─ coax (with a Lightning Arrestor) ─ Feed Point
                                                    ↑
                                         FE from a Burner Generator (touching) or any FE cable
```

Amplifiers have no input/output faces: any face works; the chain is found by walking from the
SDR (see [Antennas: the transmit chain](antennas.md#the-transmit-chain)).

### How it behaves

- **Output** = min(rated, drive × gain), where drive is the SDR's power after any coax between
  SDR and amplifier. With `tx_tone ... --power 37` (5 W) a 100 W amp gives 100 W; with
  `--power 30` (1 W) it gives 20 W.
- **FE only while transmitting**: it draws `amplified watts ÷ 0.5 ÷ 5` FE per tick, scaled by
  the fraction of each tick actually on air. Idle = 0 FE. Measured: 1 s at 100 W drew 800 FE;
  the `ham_station` 2 s transmission drew 1560 FE (40 FE/t).
- **Brownout**: with too little stored FE the output drops in proportion; with none it passes
  the 5 W drive straight through (**bypass**) and draws nothing. Never harmful. Status shows
  `· BROWNOUT 0% supply`.
- **Foldback (1 kW and 10 kW)**: if the reflected power seen at the amplifier would exceed 10%
  of its rating, it cuts its output so that it doesn't (1 kW into 3:1 SWR gives 400 W out, 100 W
  reflected; 10 kW into an open line folds back to 1 kW). These tiers can't overheat. Status
  shows `· FOLDBACK`.
- **100 W has no protection**: into a bad match it heats up. Heat = 1 W per watt amplified
  (50% efficiency) plus the reflected power, against a 125 W heat rating. Into an open line it
  reaches its failure temperature after ~29 s of continuous key-down; at 3:1 SWR or better at
  the amplifier it never fails. It warns first (smoke above 97 °C), then burns out at 130 °C into
  Melted Scrap. Put a tuner in front of it.
- **Arc foldback**: if an insulator arcs, protected amplifiers limit their output for 200 ticks.
- A transmission burst ends after 100 ms with nothing sent.
- **Events**: each burst first posts `RadioTransmitEvent` (kind `amplifier`); cancelling it
  blocks the whole burst (the SDR's write fails with "transmission blocked").

### Right-click status (chat)

```
Amplifier 100 W: out 100.0 W (drive 5.0 W) · 23 °C · 2280 / 16000 FE
SWR 1.0:1 · reflected 0.0 W · 40 FE/t while transmitting
Antenna takes 85.7 W of 227.6 W rated (antenna wire)
```

(real, from `ham_station`; the tuner made the amplifier's SWR 1.0:1). Before any fuel:

```
Amplifier 100 W: out 5.0 W (drive 5.0 W) · 20 °C · 0 / 16000 FE
SWR 1.0:1 · reflected 0.0 W · 0 FE/t while transmitting · BROWNOUT 0% supply
Antenna takes 4.3 W of 227.6 W rated (antenna wire)
```

Idle: `Amplifier 100 W: idle · 20 °C · 2200 / 16000 FE`. Other lines that can appear: `· FOLDBACK`,
`Unprotected: burns out after 29 s more at this SWR`, `No exciter: connect an SDR (touching, or through coax)`,
`Feedline ends open: no antenna`, `No lightning arrestor in the feedline`,
`Antenna takes X of Y rated (copper wire) → wire fails after 1.7 s continuous`, `Last hazard: ...`,
`lightning grounded by the arrestor`.

### The `amplifier` peripheral

A computer touching the amplifier gets peripheral type `amplifier` (read-only):

| Method | Returns |
|---|---|
| `status()` | `{tier_w, transmitting, output_w, drive_w, reflected_w, swr (≤999), foldback, temperature_c, fe_per_tick (actual, fractional), energy, capacity, supply (0..1), antenna_w, bursts}` |
| `warnings()` | list of the chain's warning strings, or `["no exciter attached"]` |

The tuner has no peripheral and uses no FE.

## 3. Hazards

### Levels

The gamerule `radioHazards` (integer) picks the level; its default `-1` means "use the server
config" `hazards.level` (default `EQUIPMENT`).

| Value | Level | What can happen |
|---|---|---|
| `0` | off | Warnings, particles and sounds only; nothing breaks |
| `1` | equipment (default) | Wires melt, coax melts, amplifiers and tuners burn out, insulators arc, lightning destroys radios |
| `2` | full | Also: arcs can start fires (needs `doFireTick`), RF exposure hurts players |

`radioLightningDamage` (boolean, default true) separately enables lightning damage; it is off
whenever the level is 0 or config `hazards.lightningDamage` is false.

Every destruction goes through events so protection mods can stop it: a cancellable
`HazardEvent`, then a vanilla `BlockEvent.BreakEvent` posted by a fake player named
`[ECM Radio]` carrying the **owner's UUID** (whoever placed the feed point, else the amplifier's
placer). Claim mods that protect the owner's land therefore block it. Arc fires post an
`EntityPlaceEvent`. Every destroyed block drops **Melted Scrap**.

### The thermal model

Parts heat toward a steady temperature set by their load (power ÷ rating) with a time constant,
exactly: `θ' = L + (θ − L)·e^(−0.05/τ)` per tick (θ = 0 ambient, 1 = failure). Warning effects
start at θ = 0.7. Time to failure = τ·ln((L − θ₀)/(L − 1)); a load at or below 1 never fails.

| Part | τ | Failure | Warning effects (from θ 0.7) |
|---|---|---|---|
| Wire | 20 s | 200 °C | orange glow particles at the hottest segment + sizzle sound (at 146 °C) |
| Insulator | 2 s | arcs at 100% of rating | electric sparks + crackle |
| Coax run | 30 s | 100 °C | smoke at the run's first block |
| Amplifier | 30 s | 130 °C | large smoke + hiss (97 °C) |
| Tuner | 30 s | 120 °C | smoke (90 °C) |

### What fails, and when

| Hazard | Cause | Result |
|---|---|---|
| **Wire melts** | Antenna accepting more power than its wire rating (duty-cycle averaged) | The hottest wire block is destroyed (`copper wire melted at 845 W (rated 68 W)`), dropping Melted Scrap. Event cause `wire_current` |
| **Insulator arc** | Voltage at an insulated end above its rating | Sparks; at level 2 a fire on a flammable block within 2 blocks; amplifiers fold back. Cause `insulator_voltage` |
| **Coax melts** | Power in a run above its rating at that frequency | The first block of the run melts. Cause `coax_heat` |
| **100 W amplifier burns out** | Sustained high SWR | Amplifier destroyed. Cause `swr` |
| **Tuner burns out** | More than 600 W of mismatch for long | Tuner destroyed (`tuner burnt out absorbing ... W of mismatch`). No `AntennaOverloadEvent` |
| **RF exposure** | Standing in a strong field (level 2, survival/adventure only) | Damage `rf_burn`, up to 2 hearts per half second; death message "%1$s was cooked by RF from an antenna" |
| **Lightning** | A bolt within 3 blocks of an antenna or its feed point, no arrestor | The device(s) between the antenna and the first arrestor are destroyed |

Real numbers from the GameTests: a 1 kW amplifier into a 2 × 10 Copper Wire dipole: "out 1000 W,
antenna takes 845 W, wire limit 68.1 W → load 12.41, 1.7 s to fail (34 ticks)"; the wire melted
after exactly 34 ticks. The same dipole in Heavy Cable: load 0.35, θ 0.048 after 3 s, survives.
With hazards off the status shows `hazards off: copper wire would melt at 845 W (rated 68 W)`.

### Lightning

- Any lightning bolt (natural, `/summon`, Channeling) within **3 blocks** of a transmit chain's
  feed point or any wire of its antenna counts as a strike. Antenna tops don't attract lightning
  beyond what vanilla does (a vanilla lightning rod touching the antenna is a conductor and part
  of it).
- The surge runs down the feedline from the antenna: the first coax run containing a
  **Lightning Arrestor** stops it (`lightning grounded by the arrestor`). Every device before
  that run (tuner, amplifier, and the **SDR** at the end) is destroyed:
  `lightning destroyed 2 device(s): no arrestor`. The feed point, wires and coax survive.
- With lightning damage off: `lightning struck the antenna (lightning damage is off)`.

### RF exposure and the RF Meter

- Sources: SDR transmissions (through their antenna chain, or the bare SDR's whip).
- Field at distance d: E = √(30·P·G)/d (d ≥ 0.5 m), summed in power over sources.
- Limit (ICNIRP general public): 610 V/m below 1 MHz; 610/f(MHz) from 1–10 MHz; 61 V/m from
  10–400 MHz; 3·√f(MHz) from 400–2000 MHz; 137 V/m above 2 GHz.
- Every 10 ticks, for each player: at ≥ 50% of the limit a beacon-like hum plays (any level, any
  game mode); at ≥ 100% **and level 2** and not creative/spectator, damage of min(4, E/limit).
- The **RF Meter** (hold it) shows `RF meter: <V/m> V/m · <dBm> dBm (0 dBi) · <MHz> MHz · limit <V/m> V/m`
  in green / gold (≥ 50%) / red (≥ limit), or `RF meter: no transmitter in range`.

### What is tested where

| Hazard | Tested in a running world (GameTest) | Unit-tested only (maths) |
|---|---|---|
| Wire melting at 1 kW, heavy cable surviving, hazards off, cancelled event | yes (`thin_copper_dipole_melts_at_1kw`, `heavy_cable_dipole_survives_1kw`, `hazards_off_only_warn`, `cancelled_hazard_event_prevents_melt`) | |
| Lightning with and without an arrestor | yes (`lightning_destroys_unarrested_station`, `lightning_arrestor_protects_station`, `ham_station`) | |
| Brownout, FE only while transmitting | yes (`amplifier_raises_far_level_and_browns_out`, `amplifier_draws_fe_only_while_transmitting`) | |
| Amplifier foldback, 100 W burnout, tuner heat | | `AmpModelTest` |
| Thermal maths | | `ThermalModelTest` |
| Insulator arc, arc fire, coax melting, tuner burnout, RF exposure damage, RF Meter readings, `radioLightningDamage false`, claim-mod cancellation, a disabled Burner Generator | not tested | |

## 4. Server config (`evanscomputermod-server.toml`)

| Key | Default | Range | Meaning |
|---|---|---|---|
| `power.wattsPerFePerTick` | 5.0 | 0.01–10000 | DC watts per 1 FE/t (amplifier draw and buffer sizes) |
| `power.burnerGenerator.enabled` | true | | Recipe, creative tab and burning |
| `power.burnerGenerator.fePerTick` | 40 | 1–100000 | Output while burning |
| `hazards.level` | EQUIPMENT | OFF / EQUIPMENT / FULL | Default for `radioHazards` |
| `hazards.lightningDamage` | true | | Default for `radioLightningDamage` |

All other radio options are in the [Developer reference](developer.md#4-server-config).
