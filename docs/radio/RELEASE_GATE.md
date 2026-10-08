# Radio release gate (1.21.1)

The spec's release gate (Roadmap, "Release gate (1.21.1)"; Sable and Create Aeronautics support,
"Gates") and what proves each item. Every result below comes from a run on branch
`radio/aero-release`; receipts are under `artifacts/` (gitignored, kept locally).

## Runs

| Receipt | Command | Result |
| --- | --- | --- |
| `artifacts/radio-aero-20261008-035655` | `scripts/Test.ps1 -Area radio-aero -GameTests ecm_radio -McVersion 1.21.1 -Aeronautics -NoStage` (Sable 2.0.5, Create 6.0.10, Create Aeronautics + Simulated + Offroad 1.3.2) | DIAGNOSTIC_PASS, all 40 `ecm_radio` tests |
| `artifacts/radio-noaero-20261008-035539` | same without `-Aeronautics` (Sable and Create only) | DIAGNOSTIC_PASS, all 40 `ecm_radio` tests |
| `artifacts/radio-sta-bench-20261008-033055` | `scripts/Test.ps1 -JUnit AccessPointCoreTest,MediumBench -McVersion 26.1 -NoStage` | DIAGNOSTIC_PASS, 16 tests |
| `artifacts/radio-gate-junit-20261008-035816` | `scripts/Test.ps1 -JUnit AccessPointCoreTest,PathTracerTest,MediumBench -McVersion 26.1 -NoStage` | DIAGNOSTIC_PASS, 32 tests (incl. the new beacon-loss test) |

Both GameTest runs load the mod's `rf_attenuation` data map and tags with no `DataMapLoader`
errors: the Aeronautics/Simulated/Create entries carry `neoforge:conditions` /
`neoforge:mod_loaded` per entry (NeoForge 21.1 data maps have no file-level conditions; a missing
block id with no condition only logs an error), and tag entries use `"required": false`.

## Gate items

| Gate item | Proved by | With Aeronautics | Without Aeronautics |
| --- | --- | --- | --- |
| **`airship_radio`**: an airship with an AP and a computer flies away from a ground station; ping holds within range, RSSI and bitrate fall with distance, the link drops and re-associates as the ship leaves and returns | scenario `airship_radio` (`/ecm scenario spawn airship_radio`, `testing/scenario/AirshipScenarios.java`), GameTest `airship_radio` (`RadioAeroTests`) | PASS (5.3 s). Ship assembled by Create Simulated's `SimAssemblyHelper` (super glue over the ship, as a player would), real Aeronautics envelopes. A real booted computer on the ship pings the phone through the AP: before assembly, on the ship, at 14 m and 40 m. Phone hears the AP at -66.5 dBm (medium predicts -68.9) at 14 m and -76.6 dBm (predicts -79.0) at 40 m; the phone's rate 54 -> 24 Mb/s. At 500 m (predicted -133 dBm) the phone loses the beacons, the ping gets no reply; back at 14 m it re-associates and pings return. Landed with Simulated's `disassembleSubLevel`; AP settings, passphrase and the pc's cable segment unchanged; ping after landing. | PASS (11.3 s). Same script; wool instead of envelopes, Sable's assembler: -68.7 / -79.1 dBm (predicted -69.0 / -79.3), rate 54 -> 24 Mb/s, drop at 500 m, re-association, landing. |
| **Ship rotation**: rotate a ship carrying a dipole 90 degrees; the received level changes by the computed pattern and polarization loss | GameTest `ship_rotation_changes_level_by_pattern` | PASS. Copper-wire dipole (21.3 MHz) on a ship 20 blocks up; receiver 30 m away. Yaw 90: level -8.18 dB, pattern+polarization computed from Sable's orientation -8.22 dB (medium antenna terms -8.22). Pitch 90 (wire upright): level -7.28 dB = pattern+polarization -13.27 dB + path ground term +6.01 dB. Tolerance 0.5 dB. | PASS (same numbers; Sable assembler) |
| **Assemble -> fly -> disassemble -> reload** keeps AP config, conductor graphs, cable networks (and SDR settings) | GameTest `ship_round_trip_keeps_radio_state`; landing part of `airship_radio` | PASS. Checked before, in flight (20 up, 12 east, turned 30 degrees), landed, and after reload: AP id, settings (WPA2, ch 36, 7 dBm) and server-side passphrase; two APs on one cable segment; the dipole's 8 blocks (wrench cut kept) and 21.252 MHz resonance; SDR 145.5 MHz / 250 kS/s / 20 dB / AGC off. | PASS (Sable assembler) |
| **Hull attenuation**: a ship hull between two ground stations attenuates their link | GameTest `sable_hull_attenuates_link` (`RadioWorldMediumTests`, unchanged) | PASS: hull lane -221 dB (sub-level blocks 160 dB) vs open lane -61 dB; frame lost through the hull, delivered in the open | PASS |
| **Handheld on a moving ship** hears a ground station | GameTest `handheld_on_moving_ship_hears_ground_station` | PASS: handheld on a deck flying at 10 m/s (moved 8.5 m during reception) decodes a ground SDR's 1 kHz FM tone (peak ratio 1324); off-frequency control silent (ratio 0.2) | PASS |
| **Aeronautics RF data** (envelopes nearly transparent, metal frames reflect / counterpoise; optional data) | GameTest `aero_rf_data_loads_only_with_its_mods` | PASS: all 32 `#aeronautics:envelope` blocks resolve to fabric (0.3 dB/block at 1 GHz); a 2-block envelope wall costs 1.48 dB at 2.4 GHz (table 1.46), stone 134 dB (control); levitite is glass; smart propeller a conductor; Create metal girder is a 0.4-fraction metal reflector in `rf_conductors`, industrial iron in `rf_good_ground` | PASS: Aeronautics assertions logged as `skipped: Aeronautics not loaded` and replaced by "the blocks are absent"; Create entries and the in-medium wool wall (2.47 dB vs table 2.45) still checked |
| **Above-heightmap rule** for airships | GameTest `airborne_link_skips_middle_diffraction`; JUnit `PathTracerTest.airborneEndsSkipMiddleDiffraction` (ridge control) | PASS: two radios 300 m apart, 60 up: diffraction 0, two-ray off the terrain | PASS |
| **Medium benchmarks at 100 / 1,000 / 10,000 radios** | JUnit `MediumBench`; numbers in [`BENCHMARKS.md`](BENCHMARKS.md) | 26.1 JUnit (pure medium, no mods), pass | n/a |

## What is not proven here

- **Flight is kinematic.** Ships are held at scripted poses every tick (Sable pipeline teleport,
  velocity cancelled). Aeronautics' lift and propellers are not exercised; the gate is about radio on
  a moving sub-level, which this does exercise (poses, link cache invalidation, hull tracing).
- **Reload is a save/load round trip of the block entities** (`saveWithFullMetadata` into a fresh
  instance via `BlockEntity.loadStatic`) plus a forced re-walk of the antenna from the landed blocks,
  not a server restart or chunk unload of a ship in flight.
- **The airship pinging partner is a virtual phone** (802.11 + WPA2 station with a tiny IPv4 host),
  not a second computer with a Wi-Fi module.
- **Ship-to-ship links, SDR receiving on an airship, dish re-aiming on a ship** are not part of this
  lane (the last is `microwave_dish_follows_sable_ship`, passing in both runs).
- **Real-game client rendering** of ships carrying radio blocks was not checked.
- **Medium gaps** (see BENCHMARKS.md): the dense delivery path allocates ~48 B per co-channel radio
  per frame, and a burst of 10,000 radios registering at once costs ~56 ms per tick for discovery.
