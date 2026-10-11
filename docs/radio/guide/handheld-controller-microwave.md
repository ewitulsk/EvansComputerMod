# Handheld Radio, Wireless Controller and microwave links

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

## 1. Handheld Radio

A receive-only AM / shortwave / VHF FM radio you carry. The server runs the receiver for you
(the same radio medium and IQ synthesis as an SDR) and streams the audio to your client.

### Bands

| Band | Range | Step | Default | Channel width | Demodulation | Whip loss |
|---|---|---|---|---|---|---|
| AM | 530 – 1700 kHz | 10 kHz | 1000 kHz | 10 kHz | AM | 25 dB |
| SW | 3 – 30 MHz | 5 kHz | 7.1 MHz | 6 kHz | AM | 12 dB |
| VHF | 30 – 300 MHz | 12.5 kHz | 146.52 MHz | 12.5 kHz | narrow FM (5 kHz deviation) | 2 dB |
| VHF 88–108 MHz | (inside VHF) | 100 kHz | | 200 kHz | wide FM (75 kHz, 75 µs de-emphasis) | 2 dB |

The whip loss models a short telescopic antenna on long wavelengths: at MW the radio is
25 dB worse than a proper antenna. The antenna is a vertical dipole at your eye height − 0.3,
noise figure 7 dB.

### Using it

- Hold it in either hand. **Right-click**: on/off (`Radio on` / `Radio off` on the action bar).
- **Sneak + right-click**: the tuning screen (it doesn't pause the game).
- It plays only while held and switched on; in the inventory it is silent. If you hold two, the
  main-hand one plays.
- While held and on, a HUD at the right edge shows `<BAND> <frequency>` and an S-meter
  (`S ||||......`; 0 bars below −125 dBm, full at −65 dBm).
- **Audio** comes through the **Jukebox/Note Blocks** volume slider (sound category
  "records"); check it isn't at 0. It is not positional (it's "in your ear").
- Settings (on, band, frequency, volume, squelch) are stored on the item (custom data
  `ecm_radio`). Defaults: off, VHF 146.52 MHz, volume 70, squelch 0.
- Tooltip: `<BAND> <freq> ON|off`, frequencies below 3 MHz in kHz, else in MHz with 4 decimals.

### The tuning screen ("Handheld Radio")

| Control | Action |
|---|---|
| Frequency box (hint "e.g. 11.6") + **Tune** or Enter | Type a frequency: `11.6` (MHz), `1000k` or `1000` (kHz), `146.52`, `11.6M`, `1000 kHz`, `7,1` (comma = decimal point). Without a unit, values ≥ 1000 are kHz, smaller values are MHz. The band is picked for you (first band containing it: AM, SW, VHF; so `30` is SW). The frequency snaps to the band's step |
| `<<` `<` `>` `>>` | −10, −1, +1, +10 steps |
| **Band** | AM → SW → VHF → AM (jumps to the band's default frequency) |
| **Scan -** / **Scan +** | Jump instantly to the next station below/above that you can actually hear (see below) |
| **Power** | on/off |
| **Vol -** / **Vol +** | volume in steps of 10 (0–100) |
| **Sql -** / **Sql +** | squelch in steps of 10 (0–100) |

Display: `<BAND>  <freq>` (`(off)` when off), the meter line `S ||||......  -NN dBm`,
`Volume N   Squelch N`, and a status line: `Tuned to SW 11.6000 MHz` (` (radio is off: press Power)`),
`Scanning SW...`, `Station at <freq>`, `No stations on VHF here`,
`Type a frequency like 11.6 (MHz) or 1000k`,
`Out of range: AM 530-1700 kHz, SW 3-30 MHz, VHF 30-300 MHz`.

**Scan** is server-side and instant: the server lists everything your antenna has heard on the
whole band in the last second at −120 dBm or stronger (about 9 dB over the noise in 6 kHz),
snaps each to the band's step grid and picks the next one above (or below) the current
frequency, wrapping round the band once. Nothing found: `No stations on <band> here`.

### Receiver details

- Audio 24 kHz, sent as IMA-ADPCM blocks every tick, only to you; playback stops a second after
  the last block.
- **AGC with a front-end attenuator**: gain = clamp(−10 − strongest signal (dBm) − 15, −60 dB,
  +40 dB). Full scale is −50 dBm at +40 dB gain and +50 dBm with the attenuator fully in, so a
  station next to you doesn't clip. With nothing heard, gain is +40 dB.
- **Squelch**: audio is muted below −130 + 0.7 × squelch dBm: 0 = always open, 10 = −123 dBm,
  50 = −95 dBm, 100 = −60 dBm.
- AM: envelope detector; FM: discriminator (5 kHz, or 75 kHz in 88–108 MHz) with de-emphasis.

Measured (GameTest `handheld_hears_am_station`, 5 W 80%-modulated 1 kHz AM on 11.6 MHz): at 3,
20 and 150 blocks the handheld read 24.1, 8.7 and −19.5 dBm and the 1 kHz tone came through at
level ~0.9 (peak ratios 195, 59, 199); 11.67 MHz (the control) heard nothing (ratio 1.5). In the
`radio_station` scenario (1 mW AM, ground-mounted vertical, listener 20 blocks away):
`SW 11.600 MHz signal -34.0 dBm, audio RMS 0.156; 11.670 MHz signal -120.2 dBm, audio RMS 0.524`.

### Quirks

- **VHF grid offset**: frequencies snap to multiples of 12.5 kHz counted from 0 Hz, so typing
  `146.52` tunes **146.525 MHz** (5 kHz off). An NBFM station on exactly 146.52 MHz is still
  heard (it falls inside the 12.5 kHz channel), slightly off-centre. The default 146.52 MHz is
  itself off-grid, so `<`/`>` keep the .02 offset until you type a frequency.
- AM-band frequencies typed without a unit below 1000 (e.g. `540`) are read as MHz and rejected;
  type `540k`.
- Between stations on AM/SW the AGC turns the noise up, so off-channel hiss can sound louder than
  a station's audio (the meter tells the truth: −120 dBm vs −34 dBm above).
- There is no recipe: get it from the creative tab or `/give @p evanscomputermod:handheld_radio`.

## 2. Wireless Controller

The Wireless Xbox Controller (`wireless_controller`, a non-radio item of the mod) talks to its
computer over 2.4 GHz through the radio medium. **There is no fixed range**: walls, distance and
other 2.4 GHz traffic decide it. (On Minecraft 26.1 there is no radio at all: the controller
works at any range without a receiver.)

### Setting it up

1. Give the computer a receiver: a **Controller Receiver Module** in a bay (Module Expansion Card
   first), or a **Wi-Fi Module in controller mode** (`wifi mode controller`, or in Python
   `peripheral.find('wifi').set_mode('controller')`; this removes `wlan0`).
2. **Pair**: right-click the computer's Terminal with the controller
   (`Controller paired with the computer at x, y, z`). Right-clicking the paired Terminal again
   opens it.
3. **Connect**: right-click in the air with the controller (again to disconnect). Sneak +
   right-click opens the key-binding screen.
4. Run `controllertest` to see the input.

Up to 4 controllers per computer (`Computer already has 4 controllers`).

### Radio parameters

| | |
|---|---|
| Frequency | the receiver's channel n (1–13, default 6): 2407 + 5n MHz (channel 6 = 2437 MHz), 2 MHz wide |
| Transmit power | 0 dBm, from your eye height − 0.4 |
| Controller antenna | −2 dBi in every direction, vertically polarized (like the receivers: a gamepad's PCB antenna runs up the grip and its dongle stands upright, so a real pair is co-polarized). About −55 dBm at 5 blocks in the open |
| Receiver | vertical dipole at the computer's centre, sensitivity −92 dBm |
| Frames | 45 bytes, 440 µs, modulation `CTRL` (needs ~2 dB SINR), sent on every input change and every 0.5 s as a keep-alive |
| Lost link | a slot is dropped after 2 s with no frame (inputs go neutral) |

Controller frames and Wi-Fi share 2.4 GHz: Wi-Fi on an overlapping channel adds interference to
the controller, and controller frames count as interference to Wi-Fi.

### Messages

| Shown | When |
|---|---|
| `Not paired: right-click a Terminal with it` | connect before pairing |
| `Computer not found` | the paired computer is gone |
| `No receiver: install a Controller Receiver module` | the computer has no receiver (or the Wi-Fi Module is in Wi-Fi mode) |
| `No signal` | frames haven't arrived for more than 1 s |
| `Connected (-58 dBm)` / `Connected (-58 dBm, Wi-Fi module)` | link up |
| `P1 -58 dBm` | the HUD label once you have a player slot: the player number and the latest signal reading (refreshed every 2 s) |

The HUD and the Terminal screen's toggle (`Controller: off (click)`, `Controller <n>: playing (-58 dBm)`,
`Controller <n>: on (typing)`) show the player number and the signal once connected; `No signal`
only appears after the slot times out.

### `controller_receiver` peripheral

`get_channel()`, `set_channel(1..13)` (paired controllers follow; error `channel must be 1-13, got N`),
`stats()` → `{channel, frequency_hz, received, last_rssi_dbm}`. A Wi-Fi Module in controller mode:
`set_controller_channel(1..13)`, and `stats()` includes `controller_reports` and
`last_controller_rssi_dbm`.

### Quirks

- Pairing doesn't warn about a missing receiver; connecting does.
- Walls and Wi-Fi interference on the controller are not covered by a test (distance and the
  missing receiver are).

## 3. Microwave links

A Microwave Radio plus a Dish makes a point-to-point Ethernet bridge between two separate cable
networks, over kilometres, on 10, 24 or 60 GHz. Like a real microwave hop: narrow beams that must
be aimed, line of sight, and rain fade.

### Building a link

On each end:

1. A **Microwave Radio** touching a **Network Cable** of that network (it joins the first
   touching cable or Internet Gateway, checked down, up, N, S, W, E).
2. A **Dish** touching the radio (any face, any part of the dish).
3. Aim the dishes at each other:
   - sneak + right-click near an edge of the dish face to nudge it (half a beamwidth per click
     when a radio is attached, else 1°); right-click to read the aim and link;
   - or from a computer touching the dish: `d.aim_at(x, y, z)` (the far dish's centre), or
     `d.align(40)` to search ±40° for the strongest radio on the same channel.
4. Both radios on the **same band, width and channel** (sneak + right-click a radio to step
   channels; or the peripheral). Defaults: 24 GHz, 56 MHz, channel 0, 20 dBm.

Any two radios on the same band/width/channel that hear each other bridge; there is no pairing.

### Bands and channels

Channel n is centred at band start + width × (n + ½).

| Band | Range | Widths (MHz) | Channels | Sensitivity |
|---|---|---|---|---|
| 10 GHz | 10.00 – 10.68 GHz | 28, **56** | 24 at 28 MHz (0–23), 12 at 56 MHz | −97.5 / −94.5 dBm |
| 24 GHz | 24.00 – 24.25 GHz | 28, **56**, 112 | 8 / 4 / 2 | −97.5 / −94.5 / −91.5 dBm |
| 60 GHz | 57 – 60 GHz | 250, 500, **1000**, 2000 | 12 / 6 / 3 / 1 | −88 / −85 / −82 / −79 dBm |

(Bold = default width. Example: 24 GHz, 56 MHz, channel 0 = 24 028 MHz.)

### Dishes

| Dish | Size | Gain at 10 / 24 / 57.5 GHz | Beamwidth at 10 / 24 / 57.5 GHz |
|---|---|---|---|
| Dish (0.6 m) | 1×1 | 33.4 / 41.0 / 48.6 dBi | 3.5° / 1.5° / 0.61° |
| Dish (1.2 m) | 2×2 | 39.4 / 47.0 / 54.6 dBi | 1.7° / 0.73° / 0.30° |
| Dish (2.4 m) | 3×3 | 45.4 / 53.0 / 60.6 dBi | 0.87° / 0.36° / 0.15° |

Gain 0.55·(πD/λ)², beamwidth 70·λ/D, sidelobes from the ITU-R F.699 envelope; the back is −13 to
−20 dBi. Aim: yaw 0 = south, 90 = west, 180 = north, −90 = east; pitch −90..90. A new dish points
the way you faced when placing it, level. The block model doesn't turn; the beam does.

### Adaptive modulation and rate

Each radio sends a beacon every 500 ms reporting the SINR it hears from its peer; the other end
picks the densest modulation with 3 dB margin (BPSK if it hasn't heard a recent report).

**Point to point.** A link is exactly two radios. From the beacons it hears each radio picks one
peer: the strongest radio that is free or already pairs with it (each beacon says whom its sender
pairs with). Frames are bridged only between two radios that pair with each other, so a third
radio on the same channel never joins in (no Ethernet loop): it shows `no link` until a free
partner turns up, and frames from it count as `stray_frames` in `info()`. A new link comes up
after the first beacons (within a second):

| Modulation | SINR needed | Rate at 56 MHz | 112 MHz | 1 GHz | 2 GHz |
|---|---|---|---|---|---|
| MW-BPSK | 4 dB | 47.6 Mb/s | 95.2 | 850 | 1700 |
| MW-QPSK | 7 dB | 95.2 | 190.4 | 1700 | 3400 |
| MW-QAM16 | 13 dB | 190.4 | 380.8 | 3400 | 6800 |
| MW-QAM64 | 19 dB | 285.6 | 571.2 | 5100 | 10200 |
| MW-QAM256 | 25 dB | 380.8 | 761.6 | 6800 | 13600 |
| MW-QAM1024 | 31 dB | 476.0 | 952.0 | 8500 | 17000 |
| MW-QAM4096 | 37 dB | 571.2 | 1142.4 | 10200 | 20400 |

(rate = width × bits per symbol × 0.85). "Linked" means a frame was heard in the last 2 s.

### Weather and atmosphere

- **Oxygen** absorption peaks at 60 GHz: ~12 dB/km at 57.5 GHz (a 3 km hop loses ~37 dB in clear
  sky); 0.15 dB/km at 24 GHz; ~0 at 10 GHz.
- **Rain** (ITU-R P.838): rain rate = rain level × 10 mm/h, rising to 50 mm/h in a thunderstorm;
  a third where it snows; none in dry biomes. Per km: 10 GHz 0.22 (10 mm/h) / 1.7 (50 mm/h) dB;
  24 GHz 1.5 / 7.4 dB; 57.5 GHz 4.8 / 16.8 dB. The test `microwave_rain_fade_60ghz`: a 3 km 60 GHz
  hop with 1.2 m dishes works in clear sky and loses every frame in a thunderstorm.
- Frames below sensitivity are dropped as `faded_frames`.

### Status

Radio right-click: `Microwave radio: 24 GHz band, channel 0, 56 MHz wide, dish connected, <RSSI> dBm, <modulation>, <rate> Mbit/s`,
or `..., no link`. Dish right-click:
`Dish (1.2 m): yaw -92.20°, pitch 0.00° (24 GHz ch 0, RSSI -55.7 dBm)`.

A real `info()` from a 60 GHz test hop in heavy rain:
`frequency_mhz=57500.0, dish_gain_dbi=54.587, beamwidth_deg=0.3041, rssi_dbm=-71.43, sinr_db=7.57, modulation=MW-QPSK, rate_mbps=1700.0, atmosphere_db=72.40, rain_mm_h=47.45, faded_frames=9`.

### On ships

A dish on a Sable ship keeps its aim relative to the ship, so turning the ship drags the beam off
target (a 30° turn costs 20+ dB); `aim_at` re-aims it (GameTest
`microwave_dish_follows_sable_ship`). The radio keeps its identity across assembly.

### Quirks

- No settings screen: channel by sneak-click or the peripheral, aim by sneak-click or peripheral.
- A third radio on a channel two radios already use stays unlinked (pick another channel).
- `align()` with no other radio returns `found: False` with `rssi_dbm` −Infinity.
