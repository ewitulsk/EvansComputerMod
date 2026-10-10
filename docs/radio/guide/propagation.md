# How radio travels: the physics, in player terms

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

Everything that transmits in the mod (Access Points, Wi-Fi Modules, SDRs, Wireless
Controllers, microwave radios, built antennas) goes through one shared **radio medium**, the
`WorldRadioMedium`. It decides, for every pair of radios, how much of the signal arrives and
whether a frame survives. This chapter explains what it models and what that means when you
build. Exact formulas and code locations are in [Developer reference](developer.md).

**Scale:** 1 block = 1 metre, and frequencies are real. A 7 MHz wave is 42.8 blocks long; a
2.4 GHz Wi-Fi wave is 12.5 cm. That is why a dipole for 7 MHz is 20 blocks long, why stone
stops Wi-Fi but not shortwave, and why a hill casts a radio "shadow" at 2.4 GHz but not at 7 MHz.

## 1. The link budget

What a receiver hears is:

```
received dBm = transmit power (dBm)
             + transmit antenna gain toward the receiver (dBi) − its feedline loss
             + receive antenna gain toward the transmitter (dBi) − its feedline loss
             − polarization mismatch (0..20 dB)
             − path loss (free space + walls + diffraction + ground effect, or skywave)
             + fading (random, per link)
```

A frame (Wi-Fi, controller, packet) is decoded if it is above the receiver's **sensitivity**
and its **SINR** (signal to noise-plus-interference) is high enough for its modulation; the
chance of loss rises smoothly near the threshold (PER curves, section 7). SDRs and the
Handheld Radio don't decode frames: they synthesise the actual received waveform (IQ samples)
at the computed level, plus thermal noise.

Units: **dBm** is power relative to 1 mW (0 dBm = 1 mW, 30 dBm = 1 W, 37 dBm = 5 W,
−90 dBm = 1 pW). Every 3 dB is ×2 power; every 10 dB is ×10. See the [glossary](reference.md#glossary).

## 2. What the path loss is made of

`/ecm radio link <x y z> <x y z> <MHz>` (operator command) prints exactly these terms for any two
points. Real output from the `radio_station` scenario (feed point to the listener's SDR, 22
blocks apart at ground level, 11.6 MHz):

```
21.9 m @ 11.600 MHz: total 59.1 dB (GROUND_WAVE, NLOS)
 free space 20.6, walls 42.7 (hulls 0.0), diffraction 0.0, ground -4.2, skywave none
 heights 1.5/1.5 m over stone; noise floor -93.0 dBm in 3 kHz; links cached 4, queued 0
```

| Field | Meaning |
|---|---|
| `total` | The whole path loss in dB, and the **mode** that won: `FREE_SPACE`, `TWO_RAY`, `DIFFRACTION`, `GROUND_WAVE` or `SKYWAVE`; `LOS` (line of sight) or `NLOS` |
| `free space` | Friis loss for the straight-line distance: 20·log10(d·f) − 147.55 dB. Doubling the distance costs 6 dB. Zero inside λ/4π |
| `walls` | Everything solid the straight line crosses (blocks near both ends, plus Sable ships anywhere on the path). Already includes `hulls` |
| `hulls` | The part of `walls` caused by Sable sub-levels (ships) |
| `diffraction` | Loss from terrain (hills, buildings in the heightmap) poking into the path, by the Deygout method |
| `ground` | The ground reflection or ground-wave term relative to free space (negative = a gain: the ground reflection adds up in phase) |
| `skywave` | The ionospheric path loss if one exists (HF/MF only), else `none` |
| `heights` | Each end's height above the ground under it, and the ground material that reflects |
| `noise floor` | The noise a 6 dB noise-figure receiver would see there (in 3 kHz below 1 GHz, 20 MHz at 1 GHz and above) |
| `links cached`, `queued` | How many radio pairs the medium has traced, and how many are waiting |

The command traces one path immediately with a vertical-polarized, 0 dBi view of the world: no
antenna gains, no fading. Frequencies 0.003–100000 MHz are accepted.

### 2.1 Walls (near the two ends)

For the **first and last 32 blocks** of a path (the whole path if it is 64 blocks or shorter)
the medium walks the straight line block by block and adds each block's loss: its loss per
metre at that frequency × the length of line inside it × the block's fill fraction (a slab
counts half, a glass pane a quarter). The two blocks holding the antennas themselves are
skipped. Unloaded chunks count as air.

Loss per block depends on frequency. Each material has a loss at 1 GHz and an exponent k:
`loss(f) = loss(1 GHz) × (f / 1 GHz)^k`. Materials that conduct at low frequency (earth, stone,
water, ice) also have a skin-depth floor, so at LF/VLF they still absorb. Iron and copper are a
flat 80 dB per block at every frequency (an enclosed metal room is a Faraday cage).

Loss per full block, dB (from `phys/Materials.java`; values at frequencies other than 1 GHz are
computed from the formula):

| Material (`rf_attenuation` name) | 7 MHz (HF) | 146 MHz (VHF) | 1 GHz | 2.437 GHz (Wi-Fi ch 6) | 5.5 GHz | 24 GHz |
|---|---:|---:|---:|---:|---:|---:|
| `air` | 0 | 0 | 0 | 0 | 0 | 0 |
| `glass` | 0.003 | 0.18 | 2.35 | 7.75 | 23.1 | 166 |
| `wood` | 0.027 | 0.70 | 5.45 | 14.1 | 33.8 | 163 |
| `leaves` | 0.05 | 0.32 | 1.0 | 1.71 | 2.78 | 6.7 |
| `wool` | 0.004 | 0.07 | 0.5 | 1.22 | 2.75 | 12 |
| `stone` = `concrete` | 0.78 | 7.4 | 33 | 66.1 | 125 | 394 |
| `brick` | 8.9 | 14.5 | 19.7 | 22.7 | 25.9 | 32.8 |
| `dirt` | 2.0 | 2.1 | 14.8 | 66.1 | 259 | 3080 |
| `sand` | 0.09 | 0.09 | 0.14 | 1.34 | 10.4 | 427 |
| `water` (fresh) | 0.55 | 0.96 | 45 | 267 | 1360 | 25900 |
| `sea_water` | 91 | 385 | 676 | 720 | 1360 | 25900 |
| `ice` | 0.009 | 0.009 | 0.05 | 0.12 | 0.28 | 1.2 |
| `snow` | 0.009 | 0.009 | 0.02 | 0.05 | 0.11 | 0.48 |
| `iron`, `copper` | 80 | 80 | 80 | 80 | 80 | 80 |

What this means in play:

- **Wi-Fi** goes through glass, wood, leaves and wool; one block of stone costs ~66 dB and
  stops it; one block of water (~267 dB) or iron (80 dB) stops it dead. The `wifi_walls`
  scenario measured these at 2462 MHz (a 1×3×3 wall for air/glass/wood/leaves, a 3×3×3 cube for
  stone/water/iron, 14 blocks):

  ```
  wifi_walls, /ecm radio link at 2462 MHz (channel 11):
    air     loss   60.4 dB (walls 0.0)
    glass   loss   68.3 dB (walls 7.9)
    wood    loss   74.7 dB (walls 14.3)
    leaves  loss   62.1 dB (walls 1.7)
    stone   loss  343.3 dB (walls 282.7)
    water   loss  468.5 dB (walls 408.0)
    iron    loss  400.0 dB (walls 339.4)
  ```

- **HF** (shortwave, 3–30 MHz) shrugs off walls: a 7 MHz signal passes three blocks of stone
  (GameTest `hf_passes_wall_that_blocks_wifi`).
- **Microwave** links (10–60 GHz) need a clear line: a single block of almost anything is
  enough to stop 24 GHz.

How blocks get a material: the `evanscomputermod:rf_attenuation` data map
(`data/evanscomputermod/data_maps/block/rf_attenuation.json`, 190 entries) names a material, or
an object with overrides (`material`, `db_per_block`, `ref_mhz`, `exponent`, `ground`, `metal`,
`fraction`). Unlisted blocks get a material from their sound type (glass sounds = glass, metal
sounds = iron, wood sounds = wood, wool = wool, sand = sand, gravel/mud/grass-block = dirt,
snow = snow, plants = leaves or air, anything else = stone), from `#leaves`, or air when they
have no collision; waterlogged partial blocks count as water. Notable data entries:

| Entries | Material |
|---|---|
| `#base_stone_overworld`, `#base_stone_nether`, cobblestone, bricks of stone/deepslate, polished stones, sandstone, end stone, obsidian, bedrock, blackstone, purpur, quartz, prismarine | stone |
| `#terracotta`, all 16 concrete colours | concrete |
| bricks, mud bricks, nether bricks | brick |
| `#dirt`, dirt path, farmland, clay, soul soil, netherrack, gravel | dirt |
| `#sand`, soul sand | sand |
| `#logs`, `#planks`, bookshelf, chest, barrel, crafting table, note block, jukebox; slabs 0.5, doors 0.2, trapdoors 0.2 | wood |
| glass, tinted and stained glass; `glass_pane`, white and black stained panes at 0.25 | glass |
| iron block/door/trapdoor, anvils, netherite, **gold block**, raw iron/gold blocks, cauldron, hopper, iron bars (1.0), chain (0.1) | iron |
| copper blocks (all states), lightning rod, copper grate (0.5) | copper |
| `#wool`, `#wool_carpets` | wool |
| barrier, structure void, light | air |
| `#aeronautics:envelope` (Aeronautics loaded) | wool at 0.3 dB/block at 1 GHz |
| levitite | glass |
| Aeronautics propellers / bearings / burners; Simulated parts; Create girder (0.4), casings, scaffolding (0.15), ladders (0.1) | wood or iron with fractions, only when those mods are loaded |

No entry uses `sea_water`, so oceans count as fresh water in the medium (the antenna solver
does use biome-based sea water for ground under an antenna).

### 2.2 Terrain and diffraction (the middle of long paths)

Beyond the 32-block ends the medium does not walk blocks; it samples the **heightmap** (the top
motion-blocking block of each column) along the path, up to 257 samples, and computes
knife-edge diffraction over the worst one to three edges (Deygout, ITU-R P.526, flat earth).
Diffraction depends on wavelength: a hill that blocks Wi-Fi by 30+ dB costs HF only a few dB
("HF bends round, Wi-Fi doesn't").

- If **both ends are more than 1 block above every middle sample** (airships, tall masts over
  flat land) the middle is treated as clear: no diffraction.
- Unloaded chunks use a per-chunk summary (heights plus the lossiest block near the surface)
  saved when the chunk unloads, so long links keep working when the middle isn't loaded.

### 2.3 Ground: two-ray and ground wave

- **Two-ray**: on line of sight the direct wave and the ground reflection add or cancel. The
  ground's permittivity and conductivity come from the block under the transmitter (short
  paths) or under the midpoint (long paths): sea water, fresh water, wet ground, average
  ground, dry ground, dry sand, ice, or metal (a metal deck is a perfect reflector). Higher
  antennas push the first null further out. `ground -4.2` in the example above means the
  reflection *helped* by 4.2 dB.
- **Ground wave** (Norton/Terman): at MF and low HF a vertically polarized wave hugs the ground
  and beats free space plus obstructions over moderate distances; the medium uses it when it
  gives less loss (`GROUND_WAVE` mode).
- **Underground (below 30 MHz)**: if a radio is buried (more than 1 block below the surface,
  under earth/stone/water/ice), the medium compares the direct path with "straight up through
  the column and then over the top" and uses the cheaper one. Because stone and earth absorb
  little at VLF/LF, a VLF transmitter can reach a buried or underwater receiver that Wi-Fi never
  could.

### 2.4 Skywave: the ionosphere (MF and HF)

Below 30 MHz a signal can bounce off the ionosphere and land thousands of blocks away. The
model (`phys/Ionosphere.java`) is driven by the **Minecraft time of day**:

- Sun angle: `cos χ = sin(2π · dayTime / 24000)` (so noon, tick 6000, is overhead).
- Critical frequency `foF2 = 4 + 6·√max(0, cos χ)` MHz: **10 MHz at noon, 4 MHz at night**.
- The maximum usable frequency for a hop is `MUF = foF2 · sec φ` (φ = angle of incidence on the
  F layer at 300 km). Signals above the MUF pass through and are lost.
- **Skip zone**: near the transmitter a skywave can't come down (the angle is too steep for the
  frequency); beyond it the signal returns.
- D-layer absorption (90 km) eats low frequencies by day and vanishes at night, so 3.5 MHz is
  short-range by day and long-range at night; 14–28 MHz open by day.
- Up to 6 hops, 1 dB per hop plus 2 dB per ground reflection, minimum elevation 2°.
- **Distances are compressed**: real hops (2,000–4,000 km) are divided by the server config
  `propagation.hopCompression` (default **500**), so one hop lands a few thousand blocks away.
  At the default the longest single hop is about 6,830 blocks.
- **No skywave without a sky**: dimensions without skylight (Nether, End) have none.
- Skywave arrives with a random polarization per pair.

Approximate figures at the default compression (computed from the model, not measured in game):

| | Noon (foF2 10 MHz) | Night (foF2 4 MHz) |
|---|---|---|
| MUF on the longest hop | 33.5 MHz | 13.4 MHz (14 MHz and up: no skywave) |
| MUF at 1,000 blocks | 12.9 MHz | — |
| Skip distance | 14 MHz: ~1,200 blocks; 21 MHz: ~2,430; 28 MHz: ~3,960; ≤10 MHz: none | 5 MHz: ~910; 7 MHz: ~1,820; 10 MHz: ~3,230 |

Known quirk: a pair that was traced while no skywave existed is only re-checked when something
else invalidates it (blocks change, a radio moves or re-registers); only pairs that are already
in skywave mode are re-traced every 600 ticks (30 s). See [limitations](reference.md#limitations-and-quirks).

### 2.5 Sable ships and Create Aeronautics airships

- Every Sable sub-level (ship) whose box touches the path is traced block by block in its own
  frame, wherever it is on the path: a **hull between two radios attenuates** like any other
  blocks (the GameTest measured −221 dB through a hull versus −61 dB in the open).
- On a short path, a ship's deck under an antenna acts as its ground (a metal hull is a
  counterpoise).
- Built antennas and dishes on a ship **turn with the ship** (pattern and polarization). Fixed
  hardware with a built-in antenna (Access Point, Wi-Fi Module, Controller Receiver, the SDR's
  whip, the Handheld Radio) follows the ship's position but its pattern does not tilt.
- Links are retraced when a ship moves more than `sable.recomputeMetres` (0.5 m) or turns more
  than `sable.recomputeDegrees` (2°), at most once per `sable.minRecomputeTicks` (4) ticks per
  ship; in between, the free-space term follows the live distance.

## 3. Antennas, gain and polarization

- **Gain** (dBi) is how much an antenna favours a direction compared with an isotropic one. A
  half-wave dipole has 2.15 dBi broadside and nulls off its ends; a 1.2 m dish at 24 GHz has
  47 dBi in a 0.7° beam. Built antennas get their real pattern from the antenna solver
  ([Antennas](antennas.md)).
- **Feedline loss** (coax length, mismatch) is subtracted from the antenna gain.
- **Polarization**: the medium projects both antennas' electric-field directions across the
  path; the mismatch costs `−20·log|cos θ|`, capped at 20 dB. A horizontal dipole and a vertical
  whip lose up to 20 dB to each other. Rotating a ship with a dipole on it changes what the
  ground hears by the pattern plus polarization change (GameTest
  `ship_rotation_changes_level_by_pattern`: −8.18 dB measured vs −8.22 dB computed).

Built-in antennas:

| Hardware | Pattern | Polarization |
|---|---|---|
| SDR with nothing attached | vertical half-wave dipole (2.15 dBi), 0.6 block above the SDR | vertical |
| Access Point, Wi-Fi Module, Controller Receiver Module | see [Wi-Fi](wifi.md) / [Handheld, controller, microwave](handheld-controller-microwave.md) | vertical |
| Handheld Radio | vertical dipole with a whip loss: AM band −25 dB, SW −12 dB, VHF −2 dB | vertical |
| Wireless Controller | −2 dBi everywhere | horizontal (see the quirk in [limitations](reference.md#limitations-and-quirks)) |
| Dishes | parabolic: gain 0.55·(πD/λ)², beamwidth 70λ/D | vertical |
| Built antennas (feed point + wires) | solved by the method of moments | from the wire directions |

## 4. Noise

The noise floor a receiver sees is thermal noise (−174 dBm/Hz + 10·log10(bandwidth) + its noise
figure) plus external noise from ITU-R P.372:

- **Man-made noise** (always the "rural" level): strong at LF/MF, falling ~28 dB per decade.
- **Galactic noise** above 10 MHz.
- **Atmospheric noise** at LF/MF/HF, much higher at night; by day it is reduced (most at low
  frequencies).
- **Thunderstorms**: when the world is thundering, the night-time atmospheric level + 15 dB is
  added, everywhere in that dimension.

Rough noise floors (6 dB noise figure, computed): 3.5 MHz in 3 kHz: −85 dBm at noon, −73 dBm at
night, −58 dBm at night in a thunderstorm; 14 MHz: −99 / −97 / −83 dBm; 146 MHz in 12.5 kHz:
−122.6 dBm; 2.437 GHz in 20 MHz: −95 dBm. VLF/LF is very noisy at night (around +16 dBm in
3 kHz at 10 kHz).

Rain does not affect the shared medium; rain and oxygen absorption are applied to **microwave
links** only ([microwave](handheld-controller-microwave.md#3-microwave-links)).

## 5. Fading

Each link gets a random fade on top of the average:

- **Line of sight**: Rician fading with K = 9 dB (server config `propagation.realism`:
  `arcade` 15 dB, `realistic` 9 dB, `simulation` 6 dB). Small wobbles.
- **No line of sight**: Rayleigh fading. Occasional deep fades.
- How fast it changes depends on how fast the ends move (coherence time 0.423·λ/speed). Two
  **stationary** radios keep one fixed draw: a given static NLOS link can sit permanently a few
  dB up or down (about 10% of such links are ≥10 dB down). Breaking and replacing a radio
  re-rolls it.

The `realism` option changes only K. Its config comment also promises oscillator drift, a DC
spike and IQ imbalance for `simulation`; none of those exist in the code.

## 6. Interference and channels

- Every transmission occupies a channel (centre + bandwidth) and has a **spectral mask**
  (DSSS Wi-Fi: −30 dB beyond ±0.5 B, −50 dB beyond B; OFDM Wi-Fi: 0 to ±0.45 B, −20 at 0.55 B,
  −28 at B, −40 at 1.5 B; everything else: 0 to ±0.5 B, −25 at 0.6 B, −40 at B, −60 at 1.5 B).
  A receiver collects the fraction of each transmitter's power that falls inside its own
  channel.
- Overlapping Wi-Fi channels (1 vs 3, say) interfere; 1, 6 and 11 don't. Wireless Controller
  frames (2 MHz wide at a 2.4 GHz channel centre) and Wi-Fi interfere with each other where
  they overlap.
- Interference adds to the noise for SINR: two transmitters on the same channel at the same
  time collide.

## 7. When is a frame decoded?

Packet error rate from SINR, per modulation (code: `medium/PerModels.java`, `phys/WifiMcs.java`):

| Modulation | SINR for ~10% loss of a 1000-byte frame |
|---|---|
| 802.11b DSSS 1 / 2 / CCK 5.5 / 11 Mb/s | 0 / 3 / 6 / 9 dB |
| 802.11a/g OFDM 6 / 9 / 12 / 18 / 24 / 36 / 48 / 54 Mb/s | 4 / 5 / 7 / 9 / 12 / 16 / 20 / 21 dB |
| 802.11n HT MCS0–7 (one stream) | 4 / 7 / 9 / 12 / 16 / 20 / 21 / 23 dB |
| AFSK1200, FSK, BPSK, QPSK, OOK | from the bit-error rate at Eb/N0 (≈15 dB for AFSK, 12 dB FSK/OOK, 8 dB BPSK/QPSK at 100 bytes) |
| LoRa-style chirp SF7 / SF9 / SF12 | about −7 / −13 / −21 dB |
| Wireless Controller (`CTRL`) | 2 dB (logistic curve) |
| Microwave MW-BPSK … MW-QAM4096 | 4, 7, 13, 19, 25, 31, 37 dB |

Longer frames need slightly more SINR. The draw is deterministic per frame, so a test is
repeatable.

## 8. Why there is no "range" number

No radio in the mod has a range setting. A link works when the budget closes:

- Wi-Fi: the `airship_radio` test measured −75 dBm at 14 m and −85 dBm at 40 m from a **0 dBm**
  AP (lost at 500 m, where the medium predicts −136.5 dBm against the module's −92 dBm
  sensitivity). At the default 20 dBm that is 20 dB more, which works out (estimate) to full
  54 Mb/s out to several tens of metres and the slowest rates to a few hundred metres in the
  open. A single stone block wall ends it.
- A 5 W SDR on VHF with whips reaches kilometres over flat ground; a hill shadows it.
- HF with a resonant dipole reaches the horizon by ground wave and thousands of blocks by
  skywave when the ionosphere cooperates.
- The medium only considers receivers within the **free-space** distance at which the
  strongest possible signal would fall below the most sensitive receiver on that channel
  (plus 6 dB), so a sparse world is cheap.

## 9. How fresh are the numbers? (caching)

The medium never casts rays when a frame is sent. Each pair of radios (per band) has a cached
link, traced on the server thread within a ray budget (`propagation.raysPerTick`, 4096 per
tick, at least one pair per tick). A pair is retraced when:

- a block changes in any 16×16×16 section its path crossed, or a surface column it sampled;
- a chunk on the path loads or unloads;
- either radio moves (0.5 m / 2° by default) or re-registers (placed, re-loaded, settings
  changed);
- a ship moves through the path;
- every 600 ticks, for pairs currently using skywave.

Until a pair has been traced, the medium assumes free space + 10 dB with peak antenna gains.
New radios look for up to 64 neighbours within 128 blocks immediately. Changing the *ground*
under an antenna (outside the ray) does not by itself retrace a link.
