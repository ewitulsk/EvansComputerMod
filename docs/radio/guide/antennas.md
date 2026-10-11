# Antennas and feedlines

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md). Block recipes are in
[Every block and item](blocks-and-items.md#antenna-building).

You build antennas out of blocks. The mod finds the conductors connected to a **Feed Point**,
turns them into a wire model and solves it with a real antenna solver (thin-wire method of
moments). The result (resonant frequency, impedance, SWR, efficiency, gain pattern,
polarization, power rating) drives everything: how well your SDR hears and transmits, whether
the amplifier is happy, and whether the wire melts.

## 1. The pieces

| Piece | Role |
|---|---|
| **Feed Point** | The antenna's terminals. Two faces along its axis are the antenna terminals (wire arms attach); the other four faces are its coax port |
| **Wires** (Copper Wire, Antenna Wire, Heavy Cable, Antenna Rod, Lattice Mast) | The radiating elements. Thicker = more power, slightly broader bandwidth |
| **Fine Wire** | Thin routed wire on a feed point's lugs, for small VHF/UHF antennas |
| **Insulator** | Holds a wire end mechanically but stops the antenna electrically. End insulators set the voltage rating at the tips. Anything in `#evanscomputermod:rf_insulators` (the Insulator and feed points; a datapack can add more, with a voltage rating from the `rf_conductor` data map) acts the same way |
| Metal blocks (`#rf_conductors`) | Join the antenna if they touch a wire, and detune it |
| **Coax Cable / Hardline / Lightning Arrestor** | The feedline from the feed point's coax side to the radio |
| **Antenna Tuner** | Inline matching unit |
| **Antenna Analyzer**, **RF Wrench**, `antenna` program | Measure, cut, inspect |

### How blocks connect

Each conductor block has six arms. Two touching blocks connect when:

- both are radio conductors and the touching sides are the same kind: **bare** (wires, rods,
  masts, insulators, feed-point terminals) to bare, or **coax** (coax, hardline, arrestor,
  feed-point side faces) to coax;
- and at least one of the two is a real conductor: insulator–insulator, insulator–feed point
  and feed point–feed point bare sides don't join;
- or a bare side touches a block in `#evanscomputermod:rf_conductors` (iron bars, lightning rod,
  chain, iron/copper/gold blocks, iron door/trapdoor, cut copper, waxed copper block; Create
  girders, casings and metal blocks; the Aeronautics smart propeller);
- or a wire's bare side touches a block in `#evanscomputermod:rf_insulators` (held, not joined);
- or a coax side touches an SDR, amplifier or tuner (`#evanscomputermod:rf_coax_ports`);
- except that a vertical feed point's lower lug does not join a full metal block under it: that
  block is the antenna's ground plane (see the ground-mounted vertical below);
- and the side isn't **cut** with the RF Wrench (on either block).

Wire arms show the connections. Blocks float, so you can string wire in mid-air between posts.

### How the antenna is found (the graph)

From a feed point the game walks every connected conductor from its two terminals:

- Each wire block becomes a 1 m wire between block centres, so **an arm of N blocks ends in the
  centre of its last block**. A dipole with N blocks per side measures exactly **2N m** tip to
  tip (1 m of feed point + 2 × (N − 0.5)).
- Mixed blocks keep their own properties (thickness, resistance, oxidation, water) half-block
  by half-block.
- The walk **stops at an insulator** (anything in `#rf_insulators`, feed points included) and records an insulated end there.
- Touching `#rf_conductors` blocks are added as thick metal (up to 64 of them).
- Fine Wire is followed only from the feed point's own lugs (up to 256 wire pieces).
- At most 1024 blocks are walked. Past any of these limits only part of the antenna is analysed,
  and the analyzer's details and the `antenna` program say so ("only part of the antenna was
  analysed: more than 64 touching metal blocks").
- The walk never crosses from a ship into the world or back.

**Ground**: the first block below the **feed point** (one ground plane for the whole antenna)
sets the ground the solver uses: water (sea water in ocean biomes, else fresh water), a full
metal block (anything solid in `#rf_conductors`: iron, copper, gold, Create casings; "metal", a
perfect ground), other `#rf_good_ground` blocks such as mud/clay ("wet ground"), sand/sandstone
("dry sand"), anything else solid ("average soil"). Nothing below (airship, void) = free space.
The analyzer prints it, e.g. `ground: average soil, 10 m below`.

### When it is (re)solved

- The game caches one result per feed point and watches every antenna block, its neighbours and
  the column under the feed. Placing/breaking/cutting/weathering/scraping marks it dirty; Fine
  Wire changes are picked up by a re-walk at most every 20 ticks.
- It only re-solves when the geometry, materials, ground or insulators actually changed.
- The solver runs on one background thread ("ECM antenna solver"). While it works, the analyzer
  and radios use a quick **estimate** (prefixed `≈` in the analyzer). A typical 200-segment solve
  takes ~25 ms.
- Antennas that need more than 200 segments are not solved: you get the estimate, marked
  `(estimate: too large for the solver (over 200 segments))`.

### What the solver does (player terms)

1. Splits the wires into segments of at most λ/20 at the design frequency (λ/10 if that would
   be more than 200), at least 4 wire radii long; straight runs of the same material are merged.
2. Solves for the current on every segment with a delta-gap source at the feed point, including
   the resistance of each block (skin effect) and the ground (image method; real grounds use a
   reflection coefficient).
3. Finds the **lowest resonance** (where the reactance crosses from capacitive to inductive)
   between 0.45× and 1.25× the half-wave frequency c/(2L). If there is none, it analyses at the
   best-SWR frequency.
4. Computes impedance, SWR against 50 Ω, efficiency, the 3-D gain pattern (5° grid) and
   polarization at that frequency, a ±20% sweep for the **2:1 SWR band**, and sweeps over every
   band from 0.45× resonance up to 3.2× resonance (capped at 30 MHz for block-wire antennas,
   500 MHz for Fine-Wire-only antennas, but never below 1.25× resonance).
5. Works out the **power rating**: the current in the hottest segment against that block's
   current rating, and the voltage at every insulated end, bare tip and the feed point against
   their voltage ratings. The lower one wins and is named: `limited to 61 W by copper wire`.

Off resonance the antenna uses the same pattern shape, scaled by its efficiency and mismatch at
that frequency. Outside the swept bands it counts as useless (−30 dB, SWR ∞). Example: a 7 MHz
dipole works on its 3rd harmonic (21 MHz, inside 3.2×) but reads "no reading" at 28 MHz.

**Below the antenna's horizon.** Over ground, the upper half of the pattern includes the ground
reflection (the image of the currents in the ground); below the antenna's horizontal the pattern
is the direct radiation of the solved currents, which is what a receiver nearby but lower than
the antenna gets (the ground reflection on that path is the propagation model's two-ray term).
So a dipole 10 blocks up is heard by a receiver 20 blocks away on the ground (27° below its
horizon) at about its free-space gain in that direction: a horizontal dipole broadside ~2 dBi, a
vertical one ~1 dBi. Straight down a vertical's axis there is still nothing (its null).

## 2. Measuring an antenna

### Antenna Analyzer (item)

- **Right-click a Feed Point**: three chat lines. The `ham_dipole` scenario (2 × 10 Copper Wire,
  10 blocks up):

  ```
  Resonant at 7.3 MHz · 2:1 SWR band 7.1–7.4 MHz · rated 61 W (copper wire) / 2.1 kW (insulators)
  dipole · Z 79.5 - j1.5 Ω · SWR 1.6:1 · efficiency 74% · gain 6.0 dBi · 20.0 m of wire · 12 segments · ground: average soil, 10 m below · limited to 61 W by copper wire
  ```

  The third line appears when coax leaves the feed point:
  `Feedline: 8 blocks, 0.29 dB at 7.2 MHz · lightning arrestor fitted` (or `· no lightning arrestor`,
  and `· ends open` if the coax goes nowhere).

  Other summaries: `No resonance found · best SWR 39.4:1 at 13.6 MHz · ...`;
  `... · SWR 1.6:1 (no 2:1 band)`; `No antenna: nothing conductive on the feed point's arms`;
  `Antenna can't be analysed: feed is on an open wire end` (or `wire N is below the ground plane`,
  `solver error: ...`). Kinds in the details line: `dipole`, `monopole`, `loop`, `long wire`,
  `wire antenna` (anything branched).
- **Sneak + right-click a Feed Point**: the "Antenna Analyzer - SWR" screen: a log SWR plot
  (1:1 to 10+:1, gridlines 1.5/2/3/5) over ±15% around resonance with 61 points, the 2:1 line in
  gold and the 2:1 band shaded green, a white resonance line and a red marker at the best point
  (`1.62:1 @ 7.20 MHz`), then the summary and power-limit text. No controls; Esc closes it. It
  refreshes when the solve finishes.

### The `antenna` program

On a computer **touching the Feed Point** (it can touch any face). Full card in
[Programs](programs.md#antenna). Real output from the `antenna_tools` scenario:

```
/ > antenna swr 6e6 8e6 21
SWR 6.000 MHz .. 8.000 MHz (21 points, 50 ohms)
 10+ |^^^^^^^^^^^^^^^^^^*
 8.1 |                   **
 6.6 |                     **                                  ****
 5.3 |                       **                             ***
 4.3 |                         **                        ***
 3.5 |                           **                   ***
 2.8 |                             **              ***
 2.3 |                               **          **
 1.9 |---------------------------------***----***------------------
 1.5 |                                    @***
 1.2 |
 1.0 |
     +-------------------------------------------------------------
      6.000                       7.000                       8.000 MHz
antenna: min SWR 1.62:1 at 7.200 MHz; resonant at 7.255 MHz; 2:1 band 7.091 MHz - 7.389 MHz

/ > antenna limits --amp 1k
Limited to 61 W by copper wire at (-13483882, -49, 368171); insulation rated 2.1 kW; amp 1.0 kW -> will overheat
weakest link: copper wire (evanscomputermod:copper_wire)
wire limit 61 W, insulation limit 2.1 kW
transmitter: amp 1.0 kW -> will overheat
```

and next to a bare feed point: `No antenna: nothing conductive on the feed point's arms`.

## 3. Materials and power ratings

| Material | Radius | Metal | Current rating | Tip voltage rating (bare end) | Typical rating as a 7 MHz dipole |
|---|---|---|---|---|---|
| Fine Wire | 0.25 mm | copper | 0.27 A | 0.8 kV | ~5 W (measured 5.4 W on a 68 MHz dipole) |
| Copper Wire | 1.0 mm | copper | 0.85 A | 1.5 kV | 61–70 W (measured) |
| Antenna Wire | 1.6 mm | copper | 1.65 A | 2.5 kV | 228 W (measured) |
| Heavy Cable | 5.0 mm | copper | 5.2 A | 5 kV | ~2 kW |
| Antenna Rod | 12.5 mm | aluminium (crafted from iron) | 11.7 A | 10 kV | ~10 kW |
| Lattice Mast | 200 mm | iron | 26 A | 40 kV | ~50 kW |
| Lightning rod (vanilla) | 4 mm | copper | 8 A | 4 kV | — |
| Iron bars | 30 mm | iron | 20 A | 8 kV | — |
| Chain | 10 mm | iron | 10 A | 5 kV | — |
| Other `#rf_conductors` metal blocks | 200 mm | iron (also gold and copper blocks) | 100 A | 40 kV | — |
| Insulator | — | — | — | 4 kV | 2.1–2.2 kW on a 7 MHz dipole (measured) |
| Feed Point gap | 3 mm | copper | 30 A | 3 kV | — |

- Ratings are for continuous key-down; the thermal model lets you exceed them briefly
  ([hazards](power-and-hazards.md#3-hazards)).
- **Copper oxidation** multiplies resistance by 1.6 / 2.6 / 4.0 (exposed / weathered / oxidized):
  efficiency drops and the copper runs hotter. Wax it with honeycomb; scrape with an axe.
- **Water**: a waterlogged conductor has 20× the resistance.
- Without end insulators the bare tips have the wire's own corona rating (copper wire 1.5 kV),
  shown as `bare end of copper wire`.
- All of these numbers are data-driven: the `evanscomputermod:rf_conductor` data map
  (`data_maps/block/rf_conductor.json`) sets `radius_mm`, `resistivity`, `current_rating_a`,
  `corona_kv`, `oxidizes`, `voltage_rating_kv`, and for feedlines `coax_loss_10mhz_db`,
  `coax_loss_1ghz_db`, `max_power_w`. Fine Wire is fixed in code.
- There are no iron or gold wire tiers.

## 4. How long? Cutting an antenna for a frequency

**The rule of thumb the game itself uses for its first estimate:** `f ≈ 143.9 / L` MHz, where L
is the antenna's electrical path length in metres (0.96 × the free-space half-wave). The solver
usually lands within ±5% of it.

| Antenna | Path length L | Estimate | Blocks for a target f (MHz) |
|---|---|---|---|
| Dipole, N blocks per arm | 2N | **f ≈ 72 / N** | N ≈ 72 / f |
| Ground-mounted vertical, N blocks up (feed on the ground) | 2(N + 0.5) (the ground's mirror image doubles it) | **f ≈ 72 / (N + 0.5)** | N ≈ 72 / f − 0.5 |

Then check it with the analyzer and trim: **a longer antenna resonates lower**. One block more
per arm of a 10-block dipole moves it about 0.65 MHz down.

Worked numbers (N = blocks per arm or blocks up):

| Band | Target | Dipole N | Vertical N |
|---|---|---|---|
| 160 m | 1.9 MHz | 38 | 37 |
| 80 m | 3.6 MHz | 20 | 19 or 20 |
| 40 m | 7.1 MHz | 10 (measured 7.2–7.3) | 10 |
| 25 m broadcast | 11.6 MHz | 6 | 6 (measured 11.6) |
| 20 m | 14.2 MHz | 5 | 5 (est.) |
| 15 m | 21.2 MHz | 3 (est. 24; measured 24.9 low over a deck) or 4 | 3 |
| 10 m | 28.5 MHz | 2 or 3 | 2 |
| VHF | above ~70 MHz | below one block per arm: use **Fine Wire** (section 6) or the SDR's built-in whip | — |

Every measured build in the test logs:

| Build | Solved result |
|---|---|
| Copper Wire dipole 2 × 10 + insulators, 10 blocks up over average soil | 7.2–7.3 MHz, 2:1 band ≈ 7.0–7.4 MHz, Z ≈ 79–90 − j1.5 Ω, SWR 1.6–1.8, efficiency 74–78%, 6 dBi, 12 segments, rated 61–70 W (copper wire) / 2.1 kW (insulators) |
| Same in Antenna Wire | 7.2 MHz, band 7.0–7.4, Z 78.2 − j1.4 Ω, SWR 1.6:1, efficiency 75%, rated 228 W |
| Same in free space | 2.08 dBi broadside, nulls off the ends |
| Same with one iron block touching an end | 6.11 MHz (clean: 7.28 MHz) |
| Same with an insulator replacing block 5 of one arm (arms 10 + 4) | above 8.6 MHz (> 1.15 × design) |
| Same with one arm cut off by the wrench at the feed point | `No resonance found · best SWR 39.4:1 at 13.6 MHz`, Z 963.7 − j985.1 Ω |
| Copper dipole 3 + 3, 0.5 block above a stone deck | 24.9 MHz, SWR 2.7, Z 18.7 Ω, efficiency 44% |
| Copper dipole 3 + 4 on a plank deck (airship) | 21.3 MHz, SWR 3.0:1, rated 12 W |
| **Vertical: feed point on grass + 6 Copper Wire up + insulator on top** | **11.6 MHz, band 11.5–11.9, Z 26.6 − j0.5 Ω, SWR 1.9:1, efficiency 43%, 1.4 dBi, rated 22 W (copper) / 789 W (insulators)** |
| Vertical: feed point + 9 Antenna Rods over a perfect (metal) ground (unit test only, see the bug in section 8) | 7.4 MHz, band 7.2–7.8, Z 33.5 Ω, rated 5.1 kW |
| Fine Wire dipole, 1 m on each lug of a bare feed point | 68.3 MHz, band 66.4–69.9, Z 73.3 − j1.2 Ω, rated 5.4 W |

A ground-mounted vertical's impedance is about half a dipole's (≈ 25–35 Ω), so SWR to 50 Ω is
about 1.5–2:1; low efficiency over average soil (43% in the example) is real: a ground-mounted
vertical needs good ground (water, or radials).

## 5. Build recipes

Coordinates: the feed point is at the origin; +x east, +y up.

### Half-wave dipole (the classic; the `ham_dipole` scenario)

1. Two posts 22 blocks apart (e.g. fences at x = −11 and x = +11, 9 high).
2. An **Insulator** on top of each post (y = 10).
3. **Copper Wire** from each insulator inward: x = −10 … −1 and x = 1 … 10 at y = 10 (2 × 10
   blocks).
4. A **Feed Point** at (0, 10, 0), clicked into the gap last: with wire on both sides along x it
   takes the X axis.
5. **Coax** from the feed point's side (north, south, top or bottom face) down to your SDR or
   amplifier. Or put a computer directly against a side face to use the `antenna` program.
6. Right-click the feed point with the Antenna Analyzer: `Resonant at 7.2–7.3 MHz`.

Block count: 20 Copper Wire (2 crafts of 3 copper ingots → 32), 2 Insulators, 1 Feed Point,
coax. Rated ~60 W: use Antenna Wire (rated ~228 W) with a 100 W amplifier, Heavy Cable (~2 kW)
with a 1 kW amplifier, Antenna Rods with 10 kW.

Pattern: horizontal polarization, strongest broadside (perpendicular to the wire), nulls off the
ends; gain 6 dBi at 10 m over soil (the ground reflection adds to the free-space 2.15 dBi).

### Inverted-V

Same as the dipole but with the arms sloping down from a single central mast (feed point on the
mast top, each arm stepping one block down every few blocks, insulators at the low ends). Any
bent geometry is solved exactly; resonance rises slightly and gain drops a little against a flat
dipole (the estimate subtracts 0.5 dB). Keep the ends off the ground: a wire touching a
`#rf_conductors` block or the ground plane changes the antenna.

### Quarter-wave vertical on the ground (the `radio_station` scenario)

1. Place a **Feed Point on top of a ground block** (grass, dirt, stone, sand, water...). Clicking
   the top face gives it the vertical (Y) axis.
2. Stack **N Copper Wire straight up** from it (N ≈ 72 / f − 0.5; 6 blocks for 11.6 MHz).
3. An **Insulator** on top.
4. Coax from a side face of the feed point along the ground to the SDR.

The analyzer reports `monopole` and `ground: average soil, 1 m below`. It radiates along the
ground in all directions (vertical polarization), which is what receivers at ground level need.

Requirements for the game to treat it as a monopole: axis Y, nothing connected to the feed
point's bottom face, no Fine Wire on the lower lug, and the block directly under the feed point
is ground. A full metal block (iron, copper, gold, a Create casing: anything solid in
`#rf_conductors`) under the feed point is the best ground of all: the lower lug doesn't join it
and the analyzer reads `ground: metal`. Water, mud or clay are good too. Thin metal (iron bars,
a chain, a lightning rod) under the feed point is not a ground: it joins as a lower element.

### Ground plane (raised vertical with radials)

1. Build a support column (fence, wood, or a Lattice Mast topped with an **Insulator** so the
   mast doesn't join the antenna).
2. On top: one **wire block** (the radial hub), then a vertical **Feed Point** on it (axis Y).
3. Radiator: N blocks of wire straight up from the feed point's top lug, insulator on top.
4. Radials: 3–4 horizontal wire runs of about N blocks each out of the hub block (insulators at
   their ends).

The solver handles it (kind `wire antenna` in the details, because it branches). It is a valid
low-angle antenna for distant stations, and receivers lower than it nearby hear its direct
radiation (see "Below the antenna's horizon" above).

### Off-centre fed dipole, long wire

- Put the feed point anywhere along a wire: unequal arms raise the impedance (and SWR); a
  tuner handles it.
- **Long wire**: a wire at least one wavelength long fed within 10% of one end (one arm long,
  the other one block or a short counterpoise). Kind `long wire`; impedance ≈ 73 + 69·log10(2L/λ) Ω
  in the estimate. Needs a tuner.

### Loops

Run wire from one lug of the feed point around a closed path and back into the other lug (both
arms must end on the feed point). A full-wave loop (circumference ≈ 1.06 λ, e.g. a square of
about 11 blocks a side for 7 MHz) is resonant near 125 Ω with ~3.1 dBi; smaller loops are
"small loops" with low radiation resistance. Kind `loop`.

### VHF/UHF with Fine Wire

Block wire can't make an arm shorter than one block, so the shortest block dipole (1 + 1) is
about 72 MHz. For higher frequencies:

1. Place a Feed Point with **nothing** attached to its bare faces (the "compact feed": its lugs
   then count as 1/16 m each).
2. Route **Fine Wire** from each lug outward, in opposite directions: click the lug, then block
   faces, then end it.
3. The tested build (1 m per lug) resonates at **68.3 MHz** (rated 5.4 W). Shorter runs go higher:
   total length ≈ 143.9 / f metres (≈ 1 m total for 146 MHz). Fine Wire is routed via block
   faces, so exact short lengths are fiddly; check with the analyzer.

For receiving VHF/UHF the SDR's own built-in whip (no coax attached) is often easier: it acts as
an ideal vertical half-wave dipole at every frequency.

### Yagi (not supported)

Parasitic elements (directors and reflectors that are not electrically connected) **are not
modelled**: the antenna is only what is electrically connected to the feed point. Unconnected
elements are ignored; connecting them makes them part of the driven element. (The solver itself
can solve Yagis, as its reference tests show, but the game never gives it unconnected wires.)

For directional links at microwave, use a dish.

### Dishes

Dishes are a separate system for 10/24/60 GHz microwave links; see
[Microwave links](handheld-controller-microwave.md#3-microwave-links).

## 6. Things that change an antenna

| Change | Effect |
|---|---|
| Longer / shorter arms | Lower / higher resonance (f ∝ 1/L) |
| Iron block, iron bars, lightning rod, chain, gold/copper block touching a wire | Joins the antenna as thick metal: detunes it (an iron block on the end of a 7.28 MHz dipole: 6.11 MHz). Only **touching** blocks count; nearby but not touching does nothing |
| Insulator in the middle of an arm | Cuts the arm there: the outer part is no longer antenna, resonance rises |
| RF Wrench on an arm | Cuts that connection (click again to restore). A dipole with one arm cut: no resonance, SWR ~40:1 |
| Oxidation | Higher resistance: less efficiency, lower power rating. Wax to stop it |
| Waterlogging | 20× resistance |
| Different ground under the feed point | Changes impedance, efficiency and pattern (metal/water best) |
| Height | Over ground the reflection shapes the pattern (6 dBi at 10 m over soil for a 7 MHz dipole) |
| Putting it on a ship | It turns with the ship; the deck under it becomes its ground |

## 7. Feedline

### Coax vs hardline

| | Coax Cable | Hardline | Lightning Arrestor (one block) |
|---|---|---|---|
| Loss per 10 blocks at 3.5 / 7 / 14 / 30 MHz | 0.29 / 0.41 / 0.58 / 0.87 dB | 0.012 / 0.017 / 0.024 / 0.035 dB | 0.003 / 0.004 / 0.006 / 0.009 dB per block |
| at 146 / 440 MHz / 1 GHz / 2.4 GHz | 2.08 / 3.96 / 6.6 / 11.8 dB | 0.08 / 0.15 / 0.23 / 0.39 dB | |
| Power rating (≤ 10 MHz, falls as √(10 MHz / f) above) | 600 W (507 W at 14 MHz, 157 W at 146 MHz, 60 W at 1 GHz) | 20 kW | 5 kW |

(Loss per 10 blocks follows a·√f + b·f through the 10 MHz and 1 GHz points.)

- HF: coax is fine for tens of blocks. VHF/UHF: coax loss adds up fast; use hardline or keep the
  SDR near the antenna.
- Overloaded coax heats and melts its first block ([hazards](power-and-hazards.md)).

### The transmit chain

```
SDR ─(touching or coax)─ [Amplifier] ─ coax ─ [Tuner] ─ coax (+ Lightning Arrestor) ─ Feed Point ─ antenna
```

- Devices can touch each other directly or be joined by coax/hardline runs; touching devices
  are preferred over coax.
- At most **8 hops** (a hop = a coax run, possibly empty, plus the device it ends at: amplifier,
  tuner, feed point, or an open end). Each run is followed up to 512 blocks, first branch only.
- Only the first amplifier counts: a second one passes the signal through unchanged.
- The chain is re-checked every 4 ticks (immediately if an amplifier or tuner is broken).
- Total feedline loss and the antenna's mismatch reduce both transmitted and received signal.

### Antenna Tuner

- Matches any SWR from 1:1 up to **30:1** (as seen at the tuner, after the coax loss beyond it):
  upstream of the tuner the amplifier sees a perfect match.
- The antenna still only accepts (1 − reflection) of the power: the mismatch is absorbed as heat
  in the tuner (rated **600 W** of mismatch). A short or badly mismatched antenna stays
  inefficient.
- Above 30:1 (or an open line) the tuner does nothing and the reflection goes back to the
  amplifier.

### Lightning Arrestor

A coax block with a ground. One anywhere in the run between the feed point and the nearest
device protects everything behind it from a strike on the antenna
([lightning](power-and-hazards.md#lightning)).

## 8. Antenna quirks (summary)

- Only a full metal block is a metal ground under a vertical feed point; thin metal joins as an element.
- No proximity detuning (only touching metal), no Yagis, no iron/gold wire tiers.
- Past the walker's limits (1024 blocks, 64 metal blocks, 256 Fine Wire pieces) only part of the
  antenna is analysed (reported).
- Breaking a wire drops a fresh item (oxidation, wax and wrench cuts are lost).
- The `antenna` program's default sweep is 41 points; the peripheral's `sweep()` default is 51.
