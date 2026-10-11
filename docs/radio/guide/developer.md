# Developer reference: how the radio feature is built

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md). Design background:
[RADIO_WIRELESS_SPEC.md](../RADIO_WIRELESS_SPEC.md) (the code wins where they differ, see
[reference](reference.md#spec-vs-implementation)), frozen interfaces:
[CONTRACTS.md](../CONTRACTS.md), performance: [BENCHMARKS.md](../BENCHMARKS.md).

All radio Java code is under `src/main/java/com/example/evanscomputermod/radio/` and is
compiled only for Minecraft **1.21.1** (`//? if <=1.21.1` Stonecutter guards). Data
(recipes, tags, data maps, loot) is in `src/main/resources-mc1.21.1/data/evanscomputermod/`.

## 1. Code map

| Package | Contents |
|---|---|
| `radio` | `RadioContent` (one `register` line per feature, tag keys), `RadioConfig` (server config) |
| `radio.api` | The public API: `Band`, `Channel`, `Pose`, `AntennaPattern`, `Emission`, `RadioEndpoint`, `RadioMedium`, `Reception`, `RadioCapabilities`; `api.event`: `RadioTransmitEvent`, `AntennaOverloadEvent`, `HazardEvent` |
| `radio.phys` | Pure physics: free space, Fresnel, knife edge, Deygout, two-ray, ground wave, skin depth, ionosphere, noise, fading, polarization, BER/PER, 802.11 MCS table, spectral masks, materials |
| `radio.medium` | `WorldRadioMedium` (the in-world medium), `BasicRadioMedium` (reference/fallback), `PathTracer`, `LinkCache`, `RayBudget`, `BandIndex`, `Airwaves`, `RfAttenuation` (data map), `LevelRfWorld`, `SableRf`, `PerModels`, `RadioLinkCommand`, `RadioMediumHooks` (lifecycle, clock), `WorldMediumContent` (registration and event glue) |
| `radio.antenna` | `AntennaGraphWalker`, `AntennaManager` (cache + solver thread), `Antenna`; `graph/` (graph, model builder, analysis, report, conductor/coax specs, `RfDefaults`); `solver/` (thin-wire MoM: mesh, solver, LU, ground, sweep, patterns, heuristic estimates); `tools/` (analyzer packets and screen, `AntennaPeripheral`, SWR plot) |
| `radio.conductor` | Conductor blocks (`ConductorBlock`, copper tiers, mast, insulator, feed point, coax), `Feedline` tracer, `RfConductorData` (data map), `RfWrenchItem` |
| `radio.amp` | Amplifiers, tuner, `AmpModel`, `ChainBudget`, `TransmitChain` (SDR → amp → coax → tuner → feed), `ExciterLink` (an SDR's transmit/receive chain) |
| `radio.power` | Burner Generator, the config recipe condition |
| `radio.hazard` | Thermal model, chain hazards, lightning, RF exposure, RF Meter, gamerules, owners |
| `radio.sdr` | SDR blocks, `SdrRadio` (state, AGC, read/write), `IqSynthesizer`, `/dev/sdr` file descriptors, peripheral |
| `radio.wifi80211` | Byte-accurate 802.11 frames, IEs, EAPOL-Key, WPA2 crypto (PBKDF2, PRF, CCMP, AES key wrap), `AccessPointCore` and a Java `StationCore` (for virtual test stations) |
| `radio.wifi` | Wi-Fi Module (`WifiModule`, `LowMac` SoftMAC, `WifiPhy`), and `wifi.ap`: the Access Point block, GUI, `WifiAirLink`, virtual stations |
| `radio.controller` | Wireless Controller radio, receiver module |
| `radio.handheld` | Handheld Radio item, server receiver/demod, screen, packets |
| `radio.microwave` | Microwave radio, dishes (multiblock), `MicrowaveLink`, dish pattern, atmosphere, bands, modulations |
| `radio.compat` | Sable and Create Aeronautics helpers used by scenarios/tests (assemble, hold, turn, land) |

Rust side (`rust/`): `crates/ecm-wifi` (802.11 station + WPA2 supplicant, used by the kernel),
`crates/ecm-dsp` (DSP), `crates/ecm-radio` (SDR program library), `crates/ecm-host-abi`
(program ↔ host calls), `operating-system/rust/src/net/wifi.rs` (kernel `wlan0`),
`net/radio0.rs` (tun), `wasm-programs/*` (programs), `wasm-programs/python/src/radio_module.rs`
+ `radio.py` (Python).

## 2. The medium

### Lifecycle

- `RadioMediumHooks` creates the medium at `ServerStartingEvent` (seeded with the overworld seed)
  and drops it at `ServerStoppedEvent`. `WorldMediumContent` installs `WorldRadioMedium` as the
  factory (otherwise `BasicRadioMedium`, free space only).
- **Clock**: `RadioMediumHooks.clockMicros()` = overworld game time × 50,000 µs + wall time since
  the tick began (capped at 50 ms), monotonic. Everything (frames, SDR samples, EAPOL timers)
  runs on it, so radio time follows game time and server lag slows it.
- Per server tick (`ServerTickEvent.Post`): chunk load/unload events (unload stores a per-chunk RF
  summary), Sable sub-level motion tracking, then `medium.tick(gameTime, glue)`.
- `BlockEvent.NeighborNotifyEvent` → `onBlockChanged` immediately.
- `DataMapsUpdatedEvent` clears the attenuation cache.

### Threading

| Callable from any thread (lock-free) | Server thread only |
|---|---|
| `register`, `unregister`, `invalidate`, `transmit`, `channelPowerDbm`, `forEachHeard`, `pathGainDb` | `tick` and everything that reads the world (`PathTracer`, `LevelRfWorld`) |

`RadioEndpoint.onReceive` is called on the thread that called `transmit` (Wi-Fi modules transmit
from the computer's thread; APs from the server thread). Receivers must be quick and thread-safe.

### Data structures

- **Shards**: one per `api.Band` + one for "outside every band" (9). Each has an `Airwaves` ring
  (8192 recent emissions; emissions longer than 100 ms in a separate list) and a `BandIndex`
  (64-block x/z grid per tuned channel, with the shard's max antenna gain and min sensitivity for
  culling).
- **`LinkCache`**: per unordered pair × quarter-octave trace bin (`WorldRadioMedium.traceBin`; the
  key is lo(28 bits) | hi(28) | bin(8)): `Link(excessDb, freqHz, gainA, gainB, polDb,
  kLinear, lineOfSight, computedTick, PathTracer.Result)`. 64 open-addressing segments; writes
  are staged and published at the end of each tick.
- Pair requests go through a 65,536-entry lock-free ring with a 4096-slot dedup filter (cleared
  when the ring overflows, so a lost request can be asked again).
- Endpoints never call `invalidate` for movement (only for antenna/channel changes): the medium
  compares poses each tick and applies one rate-limited policy; `RadioEndpoint.turnThresholdScale()`
  lets a narrow beam (microwave dish, 0.25) refresh its gains after smaller turns.

### `transmit` (the hot path: no world reads, no ray casts)

1. Clamp the start time to now; auto-register unknown senders; add the emission (with the spectral
   mask for its modulation) to the shard's airwaves.
2. Only `Emission.Kind.FRAME` is delivered; `IQ` and `ENERGY` exist for interference, channel
   power and SDR synthesis.
3. Culling radius = free-space distance at which `P + peak gains − min sensitivity + 6 dB` is used
   up. Scan the grid cells within it (or the whole grid if smaller).
4. For each candidate: skip self, other dimension, beyond range, not listening, no channel
   overlap. `rssi = P + linkDb + 10·log10(mask weight)`; drop below sensitivity; noise floor
   (receiver noise figure, rural man-made, galactic, atmospheric by time of day, thunderstorm);
   interference = Σ overlapping emissions in time and frequency at this receiver; SINR; PER from
   `PerModels`; one deterministic draw (seed ⊕ start time ⊕ receiver ⊕ payload hash);
   `onReceive(Reception(from, emission, rssi, sinr, end + d/c))`.
5. `linkDb` reads only the cache: gains toward each other − polarization − feed losses −
   (free space at the live distance + traced excess) + fading. Uncached: peak gains − (free space
   + 10 dB) and a trace request.

### The tick: tracing links

1. Set the LOS Rician K from `propagation.realism` (15/9/6 dB).
2. Register/unregister nodes. For each node: `invalidate()` → recompute gains/polarization of its
   cached links immediately and requeue them urgently; moved beyond the Sable thresholds →
   requeue (rate-limited per ship); reindex on channel or cell change; **discovery** queues up to
   64 neighbours within 128 blocks.
3. Every 600 ticks requeue skywave pairs (MUF changes with time of day).
4. Drain requests; then `recompute` urgent pairs first within `propagation.raysPerTick` (cost
   2–14 "rays" per pair by distance; at least one pair per tick).
5. Publish the cache.

`PathTracer.trace`: voxel walk of both 32-block ends (whole path ≤ 64 blocks) with
`RfAttenuation` per block state; Sable sub-levels intersecting the path traced whole in plot
space; below 30 MHz an underground end can go "up and over"; ground under each end; heightmap
profile ≤ 257 samples (end regions clamped, airborne pairs flat); `PathLossModel.evaluate`:
free space, Deygout diffraction, two-ray or Norton ground wave, ionosphere (HF and below, sky
light), obstructions; LOS = not skywave, diffraction < 6 dB, obstruction < 20 dB. Each pair
records the 16³ sections and surface columns it touched; block/chunk/region changes there requeue
it.

## 3. Public API (`radio.api`)

| Type | |
|---|---|
| `Band` | `VLF_LF` 3–300 kHz, `MF` 0.3–3 MHz, `HF` 3–30 MHz, `VHF` 30–300 MHz, `UHF` 0.3–1 GHz, `WIFI_2G4` 2.400–2.4835 GHz, `WIFI_5G` 5.150–5.895 GHz, `MICROWAVE` 10–60 GHz; `contains(hz)`, `of(hz)` (null in the gaps, e.g. 1–2.4 GHz) |
| `Channel(centerHz, bandwidthHz)` | `band()`, `lowHz()`, `highHz()`, `overlaps(c)`, `wifiNumber()`; `Channel.wifi24(n)` (1–13, 2407 + 5n MHz, 22 MHz wide), `Channel.wifi5(n, widthMhz)` (32–177, 5000 + 5n MHz, 20 or 40 MHz) |
| `Pose(dimension, x, y, z, qx, qy, qz, qw)` | world pose with orientation; `at(dim, x, y, z)`, `distanceTo`, `toWorld`, `toLocal`, `movedBeyond(o, metres, radians)` |
| `AntennaPattern` | `gainDbi(lx, ly, lz)`, `polarization(lx, ly, lz)` (unit E vector), `peakGainDbi()`, `feedLossDb()` (default 0); `ISOTROPIC`, `VERTICAL_DIPOLE`. Local frame +Y up, +Z forward |
| `Emission(kind, channel, powerDbm, startMicros, durationMicros, modulation, bitRate, payload, iq, sampleRateHz)` | `Kind {FRAME, IQ, ENERGY}`; factories `frame(...)`, `energy(...)`, `iq(...)` |
| `RadioEndpoint` | `id()`, `pose()`, `antenna()`, `tunedChannel()`, `maxTxPowerDbm()`, `onReceive(Reception)`; defaults `noiseFigureDb()` 6, `sensitivityDbm()` −95, `speedMps()` 0, `listening()` = tuned ≠ null. Return snapshots; may be called off-thread |
| `RadioMedium` | `nowMicros()`, `register`, `unregister`, `invalidate`, `transmit(from, e)`, `channelPowerDbm(at, ch)`, `pathGainDb(a, b, hz)` (NaN if not traced yet), `forEachHeard(rx, within, from, to, sink)` (emissions as heard at rx, for IQ synthesis) |
| `Reception(from, emission, rssiDbm, sinrDb, timestampMicros)` | `payload()` |
| `RadioCapabilities.ENDPOINT` | `BlockCapability<RadioEndpoint, Direction>` `evanscomputermod:radio_endpoint`; provided by SDRs, Access Points, Microwave Radios and Dishes |

Modulation names (for PER and masks): `DSSS-1`, `DSSS-2`, `CCK-5.5`, `CCK-11`, `OFDM-6`…`OFDM-54`,
`HT-MCS0`…, `CHIRP-SF7`…, `AFSK1200`, `FSK`, `BPSK`, `QPSK`, `OOK`, `CTRL`, `MW-BPSK`…`MW-QAM4096`.
802.11 payloads are MPDUs with FCS; receivers accept FCS-less MPDUs too; unicast frames are ACKed
with a 14-byte ACK one SIFS after the frame.

### Adding your own radio (another mod)

1. Implement `RadioEndpoint` (snapshot getters; quick, thread-safe `onReceive`).
2. On your block entity's server tick, fetch `RadioMediumHooks.medium()` (it is replaced per
   server start); if it changed, `unregister` from the old and `register` with the new.
3. Call `invalidate(endpoint)` when the pose, antenna or tuning changes.
4. `unregister` in `setRemoved`.
5. Transmit with `medium.transmit(endpoint, Emission.frame(channel, dBm, medium.nowMicros(), durationµs, "BPSK", bitRate, payload))`
   from any thread.
6. Expose `RadioCapabilities.ENDPOINT` in `RegisterCapabilitiesEvent` so others can find it.
7. On Sable ships, compute the world pose with Sable's projection (the mod uses
   `SensorSable.toWorld`).

### Events (NeoForge bus, all cancellable)

| Event | Fields | Posted |
|---|---|---|
| `RadioTransmitEvent` | `source` (UUID), `pose`, `channel`, `powerDbm`, `kind` (`"sdr"` / `"amplifier"`); `powerWatts()` | Every SDR IQ write chunk (program thread); the first emission of each amplifier burst. Cancel → the SDR write fails ("transmission blocked") |
| `AntennaOverloadEvent` | `level`, `feedPoint`, `powerWatts`, `ratedWatts`, `weakestLink`, `cause` (`wire_current`, `insulator_voltage`, `coax_heat`, `swr`, `tuner_mismatch`) | Before a wire melts, an insulator arcs, coax melts, an amplifier or a tuner burns out. Cancel → no damage this time |
| `HazardEvent` | `level`, `pos`, `kind` (`MELT`, `ARC_FIRE`, `RF_EXPOSURE`, `LIGHTNING`, `AMPLIFIER_BURNOUT`), `victim` (player or null), `detail` | Before every hazard action. Cancel → prevented |

Block destruction additionally posts `BlockEvent.BreakEvent` from a fake player `[ECM Radio]` with
the owner's UUID (claim mods), and arc fires an `EntityPlaceEvent`. KubeJS can cancel these via
NativeEvents (GameTest `radio_transmit_event_can_block` checks the transmit veto).

## 4. Server config

`<world>/serverconfig/evanscomputermod-server.toml` (SERVER type, synced to clients):

| Key | Default | Range | Comment / effect |
|---|---|---|---|
| `propagation.realism` | `REALISTIC` | `ARCADE`, `REALISTIC`, `SIMULATION` | Only changes the line-of-sight fading K (15 / 9 / 6 dB). The promised drift/DC spike/IQ imbalance aren't implemented |
| `propagation.hopCompression` | 500.0 | 1–100000 | Divides real skywave hop distances |
| `propagation.raysPerTick` | 4096 | 64–1048576 | Link-tracing budget per tick (applies live) |
| `sable.recomputeMetres` | 0.5 | 0.05–64 | Ship movement that retraces its links |
| `sable.recomputeDegrees` | 2.0 | 0.1–90 | Ship rotation that retraces its links |
| `sable.minRecomputeTicks` | 4 | 1–200 | At most one retrace per ship per this many ticks |
| `power.wattsPerFePerTick` | 5.0 | 0.01–10000 | DC watts per FE/t |
| `power.burnerGenerator.enabled` | true | | Recipe, creative tab, burning |
| `power.burnerGenerator.fePerTick` | 40 | 1–100000 | |
| `hazards.level` | `EQUIPMENT` | `OFF`, `EQUIPMENT`, `FULL` | Default for gamerule `radioHazards` |
| `hazards.lightningDamage` | true | | Default for gamerule `radioLightningDamage` |
| `sdr.basicMaxRate` | 48000 | 8000–10000000 | |
| `sdr.standardMaxRate` | 250000 | 8000–10000000 | |
| `sdr.advancedMaxRate` | 1000000 | 8000–10000000 | |
| `sdr.chicoryMaxRate` | 250000 | 8000–10000000 | Cap for every tier when computers run on Chicory |

Gamerules: `radioHazards` (int, default −1 = config; 0 off, 1 equipment, 2 full),
`radioLightningDamage` (bool, default true).

## 5. Data maps and tags

| Id | Kind | What |
|---|---|---|
| `evanscomputermod:rf_attenuation` | block data map | material per block (see [propagation](propagation.md#21-walls-near-the-two-ends)); object form `{material, db_per_block, ref_mhz, exponent, ground, metal, fraction}`; mod-conditional entries use per-entry `neoforge:conditions` |
| `evanscomputermod:rf_conductor` | block data map | conductor specs (`radius_mm`, `resistivity`, `current_rating_a`, `corona_kv`, `oxidizes`, `voltage_rating_kv`, `coax_loss_10mhz_db`, `coax_loss_1ghz_db`, `max_power_w`) |
| `#evanscomputermod:rf_conductors` | block tag | blocks that join antennas |
| `#evanscomputermod:rf_good_ground` | block tag | good ground under a feed point (wet ground); any full solid `rf_conductors` block there is metal (perfect) ground |
| `#evanscomputermod:rf_insulators` | block tag | blocks that hold a wire but end it electrically (Insulator, feed points; add more with a `voltage_rating_kv` in `rf_conductor`); wins over `rf_conductors` |
| `#evanscomputermod:rf_coax_ports` | block tag | what coax connects to (amplifiers, tuner, SDRs) |
| `#evanscomputermod:rf_wrenches` | item tag | RF Wrench + `#c:tools/wrench` |
| `evanscomputermod:radio_feature_enabled` | recipe condition | `{"feature": "burner_generator"}` |
| `evanscomputermod:rf_burn` | damage type | RF exposure |

## 6. Performance

From [BENCHMARKS.md](../BENCHMARKS.md) (Ryzen 7 9800X3D, `MediumBench`):

- No ray casts on the transmit path; receiver checks run at ~1.8 M/s per thread; delivery scales
  with threads (4 threads ≈ 3–3.5×).
- Sparse worlds are cheap (28k–3.3M frames/s on one thread).
- Dense worlds: 54,629 frames/s at 100 radios, 5,290 at 1,000, 539 at 10,000 (one thread), every
  frame checked against every co-channel radio.
- Gaps: the dense delivery path allocates ~48 bytes per co-channel radio per frame; 10,000 radios
  appearing at once cost ~56 ms per tick for a few ticks (pair discovery on the server thread).
- Link tracing is server-thread only, bounded by `raysPerTick`.
- Antenna solving: one background thread; ~25 ms for a 200-segment solve; antennas over 200
  segments use the estimate.
- SDR IQ synthesis runs on the reading program's thread; each read synthesises everything heard
  in its window.

Limits in code: 128-block / 64-neighbour discovery; Deygout ≤ 3 edges; profile ≤ 257 samples;
32,768 chunk summaries per dimension; request ring 65,536; airwaves ring 8,192 per band; no cap on
radio count.

## 7. Tests

Run the radio GameTests:

```
pwsh scripts/Test.ps1 -Area radio -GameTests ecm_radio -McVersion 1.21.1 -Aeronautics
```

JUnit (pure Java, run on the 26.1 project):

```
pwsh scripts/Test.ps1 -Area radio-junit -JUnit BasicRadioMediumTest,WorldRadioMediumTest,PathTracerTest -McVersion 26.1 -NoStage
pwsh scripts/Test.ps1 -Area radio-bench -JUnit MediumBench -McVersion 26.1 -NoStage
```

Rust: `cd rust; cargo test -p ecm-radio` (34 tests: blocks, modems, device files, pacing, SigMF,
WAV, scanner, CLI parsing, the radio0 link layer), `cargo test -p ecm-dsp`, `-p ecm-wifi`,
`-p python` (`_radio` with a real interpreter), `-p terminal-os radio0` (two kernels pinging
over the full AX.25/AFSK/NBFM chain). Client render check:
`scripts/Test.ps1 -Area radio-render -ClientChecks -ClientSuite radio -NoStage`.

GameTests in namespace `ecm_radio` (60 on the latest run, all passing):

| Class | Tests |
|---|---|
| `RadioTests` | `medium_delivers_frame_in_range_only` (20 m yes, 200 km no), `bridge_port_learns_client_macs`, `burner_generator_burns_fuel`, `controller_reaches_receiver_by_radio` (5 blocks yes, 3 km no, no receiver), `sdr_blocks_exchange_iq`, `handheld_hears_fm_station`, `handheld_hears_am_station` (3/20/150 blocks), `handheld_scan_finds_station`, `radio_transmit_event_can_block`, `wifi_monitor_captures_probe_requests`, `wifi_wpa2_handshake_and_ping`, `wifi_module_controller_mode`, `scenarios_registered`, `wifi_connect`, `dhcp_lan` |
| `RadioAccessPointTests` | `access_point_bridges_wpa2_station`, `access_point_rejects_wrong_passphrase`, `access_point_open_network_bridges`, `access_point_passphrase_stays_on_server`, `access_point_owner_config_and_wrench_reset`, `wifi_room_scenario` |
| `RadioAntennaTests` | `wires_autoconnect_and_wrench_cuts`, `dipole_resonates_near_design_frequency`, `touching_iron_detunes_dipole`, `insulator_gap_splits_antenna`, `analyzer_reports_dipole_and_no_antenna`, `fine_wire_vhf_dipole`, `antenna_survives_sable_assembly`, `ham_dipole` |
| `RadioAntennaToolsTests` | `antenna_peripheral_reads_dipole`, `antenna_tools` |
| `RadioPowerTests` | `amplifier_raises_far_level_and_browns_out`, `amplifier_draws_fe_only_while_transmitting`, `thin_copper_dipole_melts_at_1kw`, `heavy_cable_dipole_survives_1kw`, `hazards_off_only_warn`, `cancelled_hazard_event_prevents_melt`, `lightning_destroys_unarrested_station`, `lightning_arrestor_protects_station`, `ham_station` |
| `RadioWorldMediumTests` | `stone_wall_attenuates_wifi_by_table_value`, `iron_room_is_a_faraday_cage`, `water_blocks_wifi`, `hf_passes_wall_that_blocks_wifi`, `sable_hull_attenuates_link`, `wifi_walls` |
| `MicrowaveTests` | `microwave_link_bridges_segments`, `dish_multiblock_place_and_break`, `microwave_rain_fade_60ghz`, `microwave_link`, `microwave_dish_follows_sable_ship` |
| `RadioAeroTests` | `aero_rf_data_loads_only_with_its_mods`, `airship_radio`, `ship_rotation_changes_level_by_pattern`, `airborne_link_skips_middle_diffraction`, `ship_round_trip_keeps_radio_state`, `handheld_on_moving_ship_hears_ground_station` |
| `RadioSdrProgramTests` | `sdr_lab`, `radio0_lab`, `radio_station` |

JUnit classes (`src/test/java/.../radio/**`, 271 tests on the last full run): `AmpModelTest`,
`AntennaGraphAnalysisTest`, `AntennaSolverBehaviourTest`, `AntennaSolverReferenceTest`
(Balanis/Kraus/NEC-2 references, incl. a Yagi), `ComplexLuTest`, `FrequencySweepTest`,
`HeuristicAntennaTest`, `AntennaToolsDataTest`, `HandheldDemodTest`, `ThermalModelTest`,
`BasicRadioMediumTest`, `PathTracerTest`, `WorldRadioMediumTest`, `MediumBench`,
`DishPatternTest`, `MicrowaveLinkTest`, the `phys` tests (diffraction, fading/polarization, free
space/Fresnel, ionosphere, link budget, modulation, noise, skin depth/materials, spectral mask,
two-ray), `SdrRadioTest`, `LowMacTest`, `AccessPointHelpersTest`, `AccessPointOverMediumTest`,
`AccessPointCoreTest`, `FrameCodecTest` (golden frames), `Wpa2CryptoVectorsTest` (IEEE 802.11
Annex J and RFC 3394 vectors, shared with `docs/radio/vectors/*.json` and the Rust crate).
