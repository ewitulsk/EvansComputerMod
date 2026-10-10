# Radio & Wireless: the complete guide

Everything about the radio feature of EvansComputerMod (Minecraft **1.21.1** only): every block
and item, how radio waves travel in the world, how to build antennas, every program, how to
script radios from Python and Rust, and every known limitation. Numbers, commands and messages
are taken from the code and from real test runs.

Commands in `code` blocks are typed **into a computer's terminal** unless they start with `/`
(Minecraft chat, operator). **1 block = 1 metre**, and frequencies are real.

## Contents

| Chapter | What's in it |
|---|---|
| This page | [Quick starts](#quick-starts), [bands and frequencies](#bands-and-frequencies), [modulations](#modulations), [the big picture](#the-big-picture) |
| [How radio travels](guide/propagation.md) | Link budget, walls (material table), terrain, ground, ionosphere and day/night, noise, fading, interference, ships, `/ecm radio link` |
| [Every block and item](guide/blocks-and-items.md) | All 30 radio blocks and items: recipes, placement, right-click behaviour, blockstates, connections, power, limits |
| [Antennas and feedlines](guide/antennas.md) | How antennas are found and solved, the analyzer, materials and ratings, cutting for a frequency, build recipes (dipole, inverted-V, vertical, ground plane, long wire, loop, Fine Wire VHF), coax vs hardline, tuner, arrestor |
| [Power and hazards](guide/power-and-hazards.md) | Burner Generator, amplifiers, brownout and foldback, thermal model, melting, arcs, lightning, RF exposure, gamerules |
| [Wi-Fi](guide/wifi.md) | Access Point (GUI, security, channels), Wi-Fi Module, `wifi`, `iw`, `wpa_supplicant`, `wpa_cli`, `dhcpd`, `dhclient`, `tcpdump`, the kernel's Wi-Fi protocol |
| [The SDR](guide/sdr.md) | SDR tiers, `/dev/sdr` and `/dev/sdrctl` protocol, sample formats, AGC, the `sdr` peripheral |
| [Radio programs](guide/programs.md) | `rx_fm`/`rx_am`/`rx_ssb`, `waterfall`, `scan`, `tx_tone`, `afsk1200`, `radio_station`, `iqrec`/`iqplay`, `radiod` (radio0), `antenna`, `controllertest`, `peripherals` |
| [Programming](guide/programming.md) | Every peripheral's methods; Python `peripheral` and `radio` modules; Rust crates and a complete example program |
| [Handheld, controller, microwave](guide/handheld-controller-microwave.md) | Handheld Radio (bands, tuning screen, scan, AGC), Wireless Controller over 2.4 GHz, microwave dishes and links |
| [Scenarios](guide/scenarios.md) | All 14 `/ecm scenario` radio demos, what they build and show; GameTests |
| [Developer reference](guide/developer.md) | Implementation overview, the medium's internals, API, events, config, data maps, performance, tests |
| [Limitations, troubleshooting, glossary](guide/reference.md) | Every known quirk, symptom → fix table, glossary, spec-vs-code differences |

Related: [IN_GAME_TESTING.md](IN_GAME_TESTING.md) (hands-on test checklist),
[RADIO_WIRELESS_SPEC.md](RADIO_WIRELESS_SPEC.md) (original design),
[CONTRACTS.md](CONTRACTS.md), [SDR_PROGRAMS.md](SDR_PROGRAMS.md), [BENCHMARKS.md](BENCHMARKS.md),
[RELEASE_GATE.md](RELEASE_GATE.md).

---

## Quick starts

Need a demo instead? `/ecm scenario spawn wifi_connect`, `radio_station`, `ham_station` and
friends build these for you ([Scenarios](guide/scenarios.md)).

### Wi-Fi in two minutes (the `wifi` program)

You need: a computer A on a network cable, an **Access Point**, a computer B, a **Module
Expansion Card** and a **Wi-Fi Module**.

1. Place the Access Point touching the cable (its LEDs light up). Right-click it:
   SSID `home`, Security `WPA2-PSK (AES-CCMP)`, Passphrase `letmein123`, **Apply** (`Settings applied`).
2. On A, give the cable network addresses: `mkdir -p /etc`, create `/etc/dhcpd.conf` (with `edit`, or in `python`:
   `import shell; shell.write_file("/etc/dhcpd.conf", "pool eth0 192.168.1.10 192.168.1.100 router 192.168.1.1 dns 1.1.1.1 lease 3600\n")`), then

   ```
   ifconfig eth0 192.168.1.1/24
   dhcpd &
   ```

3. Right-click a side of B (not its screen) with the Module Expansion Card, then the same side with
   the Wi-Fi Module.
4. On B:

   ```
   wifi scan
   wifi connect home letmein123
   ping 192.168.1.1
   wifi
   ```

   `wifi connect` prints `Connected. Address 192.168.1.10/24, router 192.168.1.1, DNS 1.1.1.1`,
   or tells you exactly what's missing (no module, wrong password, no DHCP server...).

Range: tens of metres through glass, wood and leaves; a stone wall stops it. For a link that must
survive drops, use `wpa_supplicant -B -i wlan0 -c /etc/wpa_supplicant.conf` (it reconnects by
itself; `wifi connect` already wrote the file). More: [Wi-Fi](guide/wifi.md).

### Listen to a station with the Handheld Radio

1. Get a **Handheld Radio** (creative tab, or `/give @p evanscomputermod:handheld_radio`: it has no
   recipe).
2. Something must be transmitting. Easiest: `/ecm scenario spawn radio_station` (an AM music
   station on 11.6 MHz). Or your own: a computer with a **Standard SDR** touching it and some WAV
   files in its storage:

   ```
   radio_station /music 11.6M --mode am --loop --power 0 &
   ```

   (`/music` = `<world>/computer-data/<computer id>/music/`, PCM WAV files, played in name order.)
3. Hold the radio, **right-click** (`Radio on`), **sneak + right-click** to open the tuning screen.
4. Type `11.6` in the box at the top and press **Enter** (the SW band is chosen for you), or press
   **Scan +** to jump to the next station you can hear.
5. Turn up the **Jukebox/Note Blocks** volume slider if you hear nothing. The S-meter shows the
   signal; walk away and it fades into hiss. **Sql +** cuts the hiss between songs.

For VHF FM use `--mode fm` on a VHF frequency (e.g. `146.52M`; tune the handheld to `146.52`).
More: [Handheld](guide/handheld-controller-microwave.md#1-handheld-radio),
[radio_station](guide/programs.md#radio_station).

### A first HF station: dipole + SDR

Build two of these a few hundred blocks apart (or one, and use the Handheld on SW to listen).

1. **Antenna** (40 m band): two posts 22 blocks apart, an **Insulator** on top of each (10
   blocks up), **10 Antenna Wire** inward from each insulator, a **Feed Point** clicked into the
   middle gap. Right-click it with an **Antenna Analyzer**:
   `Resonant at 7.2 MHz · 2:1 SWR band 7.0–7.4 MHz · rated 228 W (antenna wire) ...`.
   (Copper Wire works too, rated ~60 W.) Dipole length: **N ≈ 72 / f(MHz) blocks per arm**.
2. **Feedline**: **Coax Cable** from one of the feed point's four side faces (not the wire ends)
   down to the ground. Put a **Lightning Arrestor** block somewhere in the run.
3. **Radio**: the coax ends at a **Standard SDR** (coax connects to it) that touches a **computer**;
   a **Speaker** touching the computer too.
4. Check: `peripherals` lists the SDR; `antenna` on a computer touching the feed point shows the
   same as the analyzer.
5. Transmit from one station (`tx_tone 7.1M --fm 1000 --seconds 20 --power 30`) and listen on the
   other (`rx_fm 7.1M`), or broadcast music in AM and tune a Handheld to SW 7.1 MHz.
6. For more power: put a **Power Amplifier (100 W)** between SDR and coax, feed it FE from a
   **Burner Generator** with coal, and add an **Antenna Tuner**. `/ecm scenario spawn ham_station`
   builds exactly this.

Keep both antennas at a similar height (an antenna radiates nothing more than 5° below its own
horizon, see [Antennas](guide/antennas.md#what-the-solver-does-player-terms)). HF reaches
thousands of blocks by skywave when the ionosphere supports it: around noon 14–28 MHz, at night
3.5–7 MHz ([propagation](guide/propagation.md#24-skywave-the-ionosphere-mf-and-hf)).

---

## Bands and frequencies

The radio medium covers everything from VLF to 60 GHz; it groups frequencies into these bands
(`radio.api.Band`):

| Band | Range | Typical use here | Character |
|---|---|---|---|
| VLF/LF | 3 – 300 kHz | SDR experiments | Through earth and water; very noisy at night |
| MF | 0.3 – 3 MHz | AM broadcast (Handheld AM band) | Ground wave, skywave at night |
| HF | 3 – 30 MHz | Shortwave (Handheld SW), ham bands, `radio_station` | Ignores most walls; skywave over thousands of blocks |
| VHF | 30 – 300 MHz | Handheld VHF FM, broadcast FM 88–108, APRS 144.39 MHz, `radio0` | Line of sight, some diffraction |
| UHF | 300 MHz – 1 GHz | SDR | Line of sight |
| Wi-Fi 2.4 GHz | 2.400 – 2.4835 GHz | Wi-Fi, Wireless Controller | Blocked by stone, water, metal |
| Wi-Fi 5 GHz | 5.150 – 5.895 GHz | Access Point channels 36–165 | Even more absorbed |
| Microwave | 10 – 60 GHz | Microwave links | Needs a clear line; rain and oxygen fade |

Frequencies between bands (e.g. 1–2.4 GHz) still work for SDRs; they are handled as "other".

What each device can use:

| Device | Frequencies |
|---|---|
| SDR (Basic) | 0.5 – 1700 MHz, receive only, ≤ 48 kS/s |
| SDR (Standard) | 10 kHz – 6 GHz, ≤ 250 kS/s, transmit ≤ 5 W |
| SDR (Advanced) | 1 kHz – 6 GHz, ≤ 1 MS/s (250 kS/s on Chicory), transmit ≤ 5 W |
| Handheld Radio | AM 530–1700 kHz (10 kHz steps), SW 3–30 MHz (5 kHz), VHF 30–300 MHz (12.5 kHz; 88–108 MHz wide FM in 100 kHz steps) |
| Access Point | 2.4 GHz channels 1, 6, 11 (or Auto among them); 5 GHz channels 36, 40, 44, 48, 149, 153, 157, 161, 165; 20 MHz; 0–20 dBm |
| Wi-Fi Module | scans and joins 2.4 GHz channels 1–13 (`iw set channel` accepts 1–14 and 32–177); 0–20 dBm |
| Wireless Controller / receiver | 2.4 GHz channel 1–13 (default 6 = 2437 MHz), 2 MHz wide, 0 dBm |
| Microwave Radio | 10 GHz (10.00–10.68, 28/56 MHz channels), 24 GHz (24.00–24.25, 28/56/112 MHz), 60 GHz (57–60, 250/500/1000/2000 MHz); −40 to +30 dBm |
| Built antennas | resonant where you cut them; usable across their 2:1 band (with a tuner, more) |

## Modulations

| Who | Modulation |
|---|---|
| Access Point / Wi-Fi Module | 802.11b DSSS/CCK 1–11 Mb/s (module only), 802.11a/g OFDM 6–54 Mb/s; WPA2-PSK CCMP or open |
| Wireless Controller | `CTRL` frames (45 bytes, 1 Mb/s) |
| Microwave Radio | adaptive MW-BPSK … MW-QAM4096 |
| Handheld Radio | AM (AM and SW bands), NBFM (VHF), WBFM (88–108 MHz) — receive only |
| `rx_fm` / `rx_am` / `rx_ssb` | NBFM, WBFM (`--wide`), AM, USB/LSB |
| `tx_tone` | carrier (CW), NBFM tone |
| `radio_station` | AM, NBFM, WBFM |
| `afsk1200`, `radiod` | AFSK1200 (Bell 202) on NBFM (3 kHz deviation), AX.25 |
| Python `radio` | FM/WBFM, AM, SSB, AFSK1200, FSK, BPSK, LoRa-style chirp (`chirp(sf, bw)`), plus any custom IQ |
| SDR raw IQ | anything you can compute |

---

## The big picture

```
 Wi-Fi Module ─┐                                                       ┌─ Access Point ── network cable
 Controller   ─┤                                                       ├─ Microwave Radio + Dish ── cable
 Handheld     ─┤──► WorldRadioMedium (one per server) ◄────────────────┤
 SDR ── amp ── tuner ── coax ── Feed Point + wires (solved antenna) ───┘
                        │
       per pair of radios, cached:  free space + walls (voxel ends) + terrain diffraction (heightmap)
                                    + ground (two-ray / ground wave) or ionosphere (skywave)
       per frame:  antenna gains + polarization + fading → RSSI; noise + interference → SINR → PER
```

- **The medium** (`WorldRadioMedium`): every radio registers as an endpoint (position, antenna
  pattern, channel). Sending a frame never reads the world: it looks up a cached link budget for
  each nearby receiver on an overlapping channel, adds antenna gains, polarization and fading,
  sums noise and interference, and draws success from the modulation's error curve. The cached
  links are traced on the server thread within a per-tick budget and retraced when blocks,
  radios or ships on the path change.
- **Antennas** are block structures walked from a Feed Point into a wire model and solved with a
  method-of-moments solver on a background thread; the result is the pattern, impedance and power
  rating used for transmit and receive.
- **SDRs** read the medium as IQ samples (synthesised on demand from what the antenna hears) and
  write IQ that becomes an emission; programs, Python and Rust do the signal processing.
- **Wi-Fi** is real 802.11 + WPA2 on both sides (Java Access Point, Rust station in the computer's
  kernel) over the medium; the Access Point is a layer-2 bridge onto its cable.
- **Ships**: Sable sub-levels and Create Aeronautics airships carry radios; hulls attenuate;
  antennas turn with the ship; settings survive assembly.

Details: [Developer reference](guide/developer.md).
