# Limitations, troubleshooting, glossary and spec differences

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

## Limitations and quirks

An honest, consolidated list. Most are also mentioned in the chapter they belong to.

### Scope

- Radio is **Minecraft 1.21.1 only**. On 26.1 none of these blocks exist and the Wireless
  Controller works at any range without a receiver.
- The **Handheld Radio** and the **Controller Receiver Module** have no crafting recipe (creative
  tab or `/give`). Melted Scrap only comes from hazards.
- No Jade/WTHIT tooltips or JEI/EMI info pages. A disabled Burner Generator is hidden by having no
  recipe and no creative-tab entry (not by the `c:hidden_from_recipe_viewers` tag).
- Breaking a radio block drops a plain item: SDR tuning, AP settings, wire oxidation/wax and
  wrench cuts are not carried on the item.

### Propagation

- Raised antennas have **zero gain more than 5° below their own horizon** (the antenna solver
  computes only the upper hemisphere over ground). A raised dipole is barely heard by lower
  receivers nearby; use a ground-mounted vertical for ground-level listeners.
- One path trace per band is reused for every frequency in that band (a link traced at 3.5 MHz is
  reused at 28 MHz until something retraces it).
- A pair traced while no skywave existed is not re-checked when the ionosphere changes (only
  pairs already using skywave are re-traced every 30 s); it updates when anything else
  invalidates the pair (a block change on the path, a radio moving or being replaced).
- Static links keep one random fade forever (fading only changes with movement); replacing a
  radio re-rolls it.
- `realism` only changes fading K; no oscillator drift, DC spike or IQ imbalance exist.
- Rain doesn't affect the shared medium (only microwave links); man-made noise is always "rural";
  thunderstorm noise is dimension-wide; no lightning crashes.
- Oceans count as fresh water in the medium.
- Changing the ground under an antenna (outside the ray path) does not by itself retrace links.
- Very short VLF/LF links can show a small negative path loss (ground reflection gain inside the
  near field).
- Fixed-pattern hardware (AP, Wi-Fi Module, controller receiver, SDR whip, handheld) follows a
  ship's position but its pattern doesn't tilt with the ship.

### Antennas

- No parasitic elements (no Yagis), no proximity detuning (only touching metal), no iron/gold wire
  tiers, one ground plane per antenna (under the feed point).
- A metal block directly under a vertical feed point joins the antenna instead of acting as a
  ground plane; gold blocks, cut copper and `c:storage_blocks/gold|aluminum` conduct but aren't
  "good ground".
- Off resonance the antenna reuses its resonant pattern shape; outside the swept bands (above 3.2×
  resonance or 30 MHz for block wire) it counts as useless.
- Antennas over 200 solver segments only get an estimate; walker limits (1024 blocks, 64 metal
  blocks, 256 Fine Wire pieces) truncate silently.
- Fine Wire joins block wires only at a feed point.
- The analyzer's pending line says "(estimate: solving) (solving…)".

### Power and hazards

- Insulator arcs, arc fires, coax melting, tuner burnout, RF exposure damage and the 10 kW tier are
  implemented and unit-tested but not tested in a running world. 100 W burnout and foldback are
  unit-tested only.
- No glow or screen tint: warnings are particles and sounds.
- Antenna tops aren't extra lightning attractors (vanilla decides where bolts land); any lightning
  bolt within 3 blocks counts, including visual-only ones.
- The RF Meter and RF exposure only see SDR transmissions (not Wi-Fi, controllers or microwave).
- Only the first amplifier in a chain amplifies.
- A disabled Burner Generator still pushes out FE it had stored; its recipe may survive a world's
  first load until `/reload`.
- Owner records of feed points are never cleared.

### SDR and programs

- SDR samples contain clean signals plus thermal noise (no fading, atmospheric noise, drift).
- Frames from Wi-Fi/controllers/microwave appear in SDR samples as noise bursts (not decodable).
- `bw` doesn't filter the samples; one read cursor per SDR; several settings aren't saved.
- `scan` uses a fixed 30 dB gain: strong nearby signals produce spurious hits; use `--gain 0`.
- Killed transmitters (Ctrl+T, `kill`) leave the SDR's tx flag on.
- `radio0` needs `--txdelay 100 --gain 10` to be reliable and still fails occasionally; ssh over
  radio0 is tested only between simulated kernels.
- Tested on the host only (not in a world): `waterfall` graphics, `iqrec`/`iqplay`, `rx_ssb`, Python
  flowgraphs on a real SDR.
- Python: `run()` without a limit on a silent SDR never returns; an SDR sink stays in transmit.

### Wi-Fi

- 802.11a/g only (no n/ac), WPA2-PSK or open only, no PTK rekey, MFP or power save.
- **5 GHz Access Points can't be found by computers** (the station scans channels 1–13 only).
- Roaming and 5 GHz are not tested in a world; ship flights in the tests are scripted (held
  positions), not flown with propellers.
- The AP never retransmits. Clients idle for 300 s are deauthenticated; only `wpa_supplicant -B`
  reconnects automatically (`wifi connect` doesn't leave a supplicant running).
- Hidden SSIDs need an `iw dev wlan0 scan ssid <name>` first. `wifi connect` rejects 64-hex keys.
- The AP's factory reset needs no permission and transfers ownership.
- `tcpdump` doesn't decrypt; `iw`, `wpa_*` and `tcpdump` aren't in `help`.
- Channel 14 is accepted by `iw` but the radio doesn't tune to it.

### Handheld, controller, microwave

- Handheld VHF frequencies snap to a 12.5 kHz grid from 0 Hz: typing 146.52 tunes 146.525 MHz.
- Typing an AM frequency below 1000 without a unit is read as MHz and rejected.
- The Wireless Controller is horizontally polarized against vertical receivers (up to 20 dB
  loss); its dBm reading is rarely visible on the HUD.
- Dishes don't visually turn; no microwave settings screen; three radios on one channel all bridge.

### Bugs fixed while writing the scenarios (for reference)

- **Access Point ↔ computer Wi-Fi**: the AP block never sent 802.11 ACKs and passed received
  frames to its core with the module's FCS still attached, so a computer's Wi-Fi Module (which
  retries unacknowledged frames, then gives up) could never authenticate with a real AP. Fixed:
  the AP verifies/strips the FCS and ACKs at SIFS.
- **Wi-Fi scans**: 30 ms per channel missed APs (they answer probes on the 50 ms server tick and
  beacon every 102 ms); the dwell is now 120 ms (a full scan ~1.6 s).
- **Handheld AM**: a fixed +40 dB gain clipped near stations (flattening AM audio). It now has AGC
  with a +40 to −60 dB front-end attenuator.
- **Python REPL**: input wasn't echoed; `shell.write_file` returned False on success and rejected
  absolute paths; `python /file.py` couldn't open absolute paths.
- **GameTests with Access Points** now run at 20 ticks per second (EAPOL timers are on game time).

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `wifi: this computer has no Wi-Fi Module` | no module, or no expansion card | Module Expansion Card on the computer's side, then the Wi-Fi Module on the same side |
| `wifi` / `iw scan` doesn't list my AP | stone/water/iron between them; out of range; AP on 5 GHz; hidden SSID; AP not configured | Line of sight through glass/wood; move closer; use channel 1/6/11; `iw dev wlan0 scan ssid <name>`; check the AP's Status tab |
| `the password for 'X' is wrong` | wrong passphrase | Re-type it; the AP's Status tab shows `4-way M2 MIC mismatch` |
| `joined 'X', but nothing gave this computer an address` | no DHCP server on the AP's cable | Run `dhcpd &` on a wired computer (pool on its eth0), or `ifconfig wlan0 a.b.c.d/24`. The Internet Gateway never serves DHCP |
| Wi-Fi worked, then stopped after a while | idle 300 s timeout, or beacon loss; `wifi connect` doesn't reconnect | `wifi connect` again, or run `wpa_supplicant -B -i wlan0 -c /etc/wpa_supplicant.conf` |
| Access Point GUI won't open | not the owner / not op / not creative, or > 8 blocks away | Ask the owner, or factory-reset it with an RF Wrench (sneak + right-click) |
| AP LEDs off | not touching a network cable | Place it against a cable |
| `can't open the first SDR` | SDR not touching the computer | Place it against a computer face |
| `can't transmit ...; a Basic SDR is receive-only` | Basic SDR | Use Standard or Advanced |
| Receiver hears only hiss | wrong frequency/mode; transmitter not running; walls; raised antenna above you | `scan`, check `jobs` on the transmitter, move closer or to the same height, use a vertical on the ground |
| Receiver hears garbage near a strong station / `scan` finds many signals | overload (clipping) | Lower the transmitter's `--power` (0 dBm is plenty nearby), or `--gain 0` on the receiver |
| Handheld silent | radio off; not held; Jukebox/Note Blocks slider at 0; squelch high | Right-click to switch on; raise the slider; squelch 0 |
| Handheld hears a station "nearby" on VHF a bit off | 12.5 kHz grid offset | Tune ±1 step |
| Analyzer: `No antenna: nothing conductive on the feed point's arms` | nothing on the bare (axis) faces, or arms cut | Put wire on the two faces along the feed point's axis (its lugs) |
| Analyzer: `No resonance found` / SWR very high | arms unequal, one arm cut, metal touching, too short | Check for wrench cuts, metal blocks touching, arm lengths (N ≈ 72/f per arm) |
| Resonance lower than expected | touching metal (iron bars, lightning rod, metal blocks) joined the antenna | Move metal away or insulate |
| Vertical reads as `dipole`/`wire antenna`, not `monopole` | feed point not directly on ground, or metal under it | Put the feed point on a dirt/grass/stone/water block, nothing below it connected |
| Amplifier `BROWNOUT` | no FE | Feed it from a Burner Generator (with fuel) or another FE source |
| Amplifier `No exciter` | no SDR on the chain | SDR touching the amp, or coax from SDR to amp |
| `Feedline ends open: no antenna` | coax doesn't reach a feed point's side | Connect coax to one of the feed point's four non-axis faces |
| `No lightning arrestor in the feedline` | | Put a Lightning Arrestor block in the coax run between feed point and the first device |
| Wire glows/sizzles, then melts | more power than the wire's rating | Thicker wire (Antenna Wire, Heavy Cable, Rod), less power, or `/gamerule radioHazards 0` |
| Tuner smoking | absorbing too much mismatch | Fix the antenna's length instead |
| Microwave `no link` | dishes not aimed, different band/width/channel, obstruction, rain at 60 GHz | `align(40)` or `aim_at`; match channels; clear the line; use 10/24 GHz or bigger dishes in rain |
| Controller `No receiver` | no receiver module, or the Wi-Fi Module in Wi-Fi mode | Install a Controller Receiver Module or `wifi mode controller` |
| Controller `No signal` | out of range, walls, polarization, 2.4 GHz interference | Move closer; change the receiver's channel (`set_channel`) away from busy Wi-Fi |
| `radio0` pings lost | collisions, AGC settling | Use `--txdelay 100 --gain 10`, lower `--power` for close stations, retry |
| `/ecm radio link`: `The in-world radio medium isn't running.` | server not fully started | Wait, or reload the world |

## Glossary

| Term | Meaning |
|---|---|
| **dB** | Ratio in decibels: 10·log10(power ratio). +3 dB = ×2, +10 dB = ×10 |
| **dBm** | Power relative to 1 mW: 0 dBm = 1 mW, 20 dBm = 100 mW, 30 dBm = 1 W, 37 dBm = 5 W, −90 dBm = 1 pW |
| **dBi** | Antenna gain relative to an isotropic antenna. A half-wave dipole: 2.15 dBi |
| **dBFS** | Level relative to the SDR's full scale (0 dBFS = clipping) |
| **Path loss** | How much weaker a signal gets between two antennas |
| **Free-space loss (FSPL)** | Path loss with nothing in the way; 6 dB more per doubling of distance |
| **Fresnel zone** | The ellipsoid around the line of sight that must be clear for a "clean" path |
| **Diffraction** | Bending of a wave over an edge (hill); stronger for long wavelengths |
| **Two-ray** | Direct wave + ground reflection interfering |
| **Ground wave** | MF/HF wave travelling along the ground's surface |
| **Skywave** | HF wave reflected by the ionosphere |
| **MUF** | Maximum usable frequency: the highest frequency the ionosphere reflects for a given path |
| **Skip zone** | The ring around a transmitter where skywave can't come down |
| **foF2** | Critical frequency of the ionosphere's F2 layer (10 MHz at noon, 4 MHz at night here) |
| **Noise floor** | The noise power a receiver sees in its bandwidth |
| **SNR / SINR** | Signal to noise (plus interference) ratio, dB |
| **Sensitivity** | Weakest signal a receiver accepts |
| **Noise figure (NF)** | How much noise a receiver adds, dB |
| **PER / BER** | Packet / bit error rate |
| **Fading** | Random variation of a link's strength; Rayleigh (no line of sight) or Rician (line of sight, factor K) |
| **Polarization** | Orientation of the electric field (vertical, horizontal, circular). Mismatch costs signal |
| **SWR** | Standing wave ratio: how well an antenna matches 50 Ω. 1:1 perfect, 2:1 acceptable (11% reflected), 3:1 25% reflected |
| **Impedance (Z = R + jX)** | Resistance R and reactance X at the feed point; resonance is where X = 0 |
| **Resonance** | The frequency where the antenna's reactance is zero |
| **Dipole** | Two equal wires fed in the middle; resonant when ~half a wavelength long |
| **Monopole / vertical** | One wire fed against ground; resonant at ~a quarter wavelength |
| **Ground plane / radials** | Wires that act as the ground for a raised vertical |
| **Feedline** | Cable from radio to antenna (coax, hardline) |
| **Tuner (ATU)** | Matching network that makes a mismatched antenna look like 50 Ω to the transmitter |
| **Exciter** | The low-power transmitter driving an amplifier (the SDR) |
| **Foldback** | An amplifier reducing its output to protect itself |
| **MoM** | Method of moments: the numerical antenna solver |
| **IQ** | Complex baseband samples (in-phase I and quadrature Q) from an SDR |
| **Sample rate (S/s)** | IQ samples per second; the SDR sees a band this wide |
| **AGC** | Automatic gain control |
| **AM** | Amplitude modulation (MW/SW broadcast) |
| **NBFM / WBFM** | Narrow-band FM (5 kHz deviation, two-way radio) / wide-band FM (75 kHz, broadcast 88–108 MHz) |
| **SSB (USB/LSB)** | Single sideband, upper/lower |
| **AFSK1200** | Audio FSK at 1200 baud (Bell 202 tones), used for APRS |
| **AX.25** | Amateur packet radio link protocol |
| **KISS** | Simple framing between a computer and a packet modem (TNC) |
| **SigMF** | Signal metadata format (`.sigmf-meta` next to an IQ file) |
| **Waterfall** | Scrolling spectrum display over time |
| **SSID / BSSID** | Wi-Fi network name / the AP's MAC address |
| **WPA2-PSK, CCMP** | Wi-Fi security with a shared passphrase; AES-based encryption |
| **4-way handshake** | WPA2 key exchange after association (EAPOL messages M1–M4) |
| **Beacon** | Periodic AP announcement |
| **Monitor mode** | Receiving every frame on a channel, with a radiotap header |
| **Radiotap** | Header with per-frame radio info (rate, channel, signal) in captures |
| **DSSS / CCK / OFDM / HT** | 802.11b / b / a/g / n modulations |
| **SIFS / DIFS / CSMA/CA / NAV** | 802.11 timing and medium access rules |
| **DHCP** | Automatic address assignment |
| **FE, FE/t** | Forge Energy, per tick; 1 FE/t = 5 W here |
| **Sub-level** | A Sable ship (blocks that move as one structure) |

## Spec vs implementation

Where [RADIO_WIRELESS_SPEC.md](../RADIO_WIRELESS_SPEC.md) and the code differ (code wins):

| Area | Spec | Code |
|---|---|---|
| Bands | "seven bands"; 5 GHz to 5.925 GHz | eight `api.Band` entries (incl. MICROWAVE); 5 GHz to 5.895 GHz in `api.Band` (5.925 in `phys.Band`) |
| Link cache | per (tx, rx, band) with per-section version counters | per unordered pair × band shard with a section → pairs dependency map |
| Link recompute | spread across worker threads | server thread only, fixed `raysPerTick` budget |
| Hot path | no per-frame allocation | ~48 B per co-channel radio per frame (dense) |
| Chunk summaries | column heights + average attenuation | heights + the lossiest block near the surface |
| Fading | per frame; ships fade from linear and angular velocity | per coherence block from a scalar speed; static links keep one draw |
| Noise | per-band atmospheric, lightning impulses, rain above 10 GHz | constant +15 dB thunderstorm term, no impulses; rain only on microwave links |
| Realism preset | arcade forgiving noise; simulation drift, DC spike, IQ imbalance | only the LOS fading K changes |
| Skywave distance | 2–8k blocks | longest single hop ≈ 6,830 blocks at compression 500; skip 0.9–4k blocks |
| Ground for antennas | first solid block under each segment | one plane under the feed point |
| Fine Wire segmentation | λ/10 | λ/20 (λ/10 only at the 200-segment cap) |
| Antennas | Yagis with parasitic elements; iron/gold wire tiers | only connected conductors; copper tiers, aluminium rod, iron mast |
| Insulated wire tiers unaffected by water | | no insulated tiers; any waterlogged conductor ×20 resistance |
| `NetworkCableBlock` on `ConductorBlock` | planned | not moved (shared with 26.1) |
| Tooltips | Jade/WTHIT tooltips ("no lightning arrestor") | chat/status lines and analyzer text only |
| Burner Generator | 20–40 FE/t; hidden via `c:hidden_from_recipe_viewers`; stops producing when disabled | 40 FE/t; no recipe + no creative entry; still pushes stored FE |
| Hazards "only player-built parts can break" | | anything in the chain can break; tuner burnout posts no `AntennaOverloadEvent` |
| RF exposure | screen tint; meter sees every antenna | hum only; only SDR chains |
| SDR synthesis | fading, atmospheric/lightning noise, emitters 10 dB under noise skipped, worker thread, ~1 tick latency | thermal noise only, on the reading program's thread, 0.25 s backlog |
| SDR control | `tx on|off` | also `tx on <dBm>`, `agc 0|1|off`, `gain agc`; more status keys |
| SDR programs doc | receivers read 10–20 ms; radio0 holds off only after frames | reads 20–100 ms; hold-off also on carrier sense |
| Access Point permission | owner or claim permission | owner, op, creative, or anyone if unowned; wrench reset needs none |
| Beacons | only to scanning/associated radios | idle modules accept them too |
| Monitor mode | decrypts | no decryption |
| `dhcpd` | reuses the router's DHCP logic | uses `ecm_net::dhcp_server` |
| Dish | aimed in a GUI | sneak-click nudges and the peripheral; no GUI; model doesn't turn |
| Handheld | frequency dial; reuses the Speaker's streaming | buttons + text box; its own audio client (same ADPCM codec) |
| Controller | pairing warns without a receiver; HUD shows dBm | warns only on connect; dBm rarely visible |
| `airship_radio` (IN_GAME_TESTING §12) | needs Aeronautics | needs Sable; Aeronautics optional (wool/iron-bar fallbacks) |
| `radio_station`, `rx_am` (PR #52 notes) | host-tested only | `radio_station` and `rx_am` now run in-world in the `radio_station` scenario |
| Frozen item list | | adds `melted_scrap` |
