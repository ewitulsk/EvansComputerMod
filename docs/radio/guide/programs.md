# Radio programs

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

Every program here is a WebAssembly program shipped with the mod (type its name in a computer's
terminal). The Wi-Fi programs (`wifi`, `iw`, `wpa_supplicant`, `wpa_cli`, `dhcpd`, `dhclient`,
`tcpdump`) are in the [Wi-Fi chapter](wifi.md#5-programs).

| Program | Needs | Purpose |
|---|---|---|
| [`rx_fm`, `rx_am`, `rx_ssb`](#rx_fm-rx_am-rx_ssb) | SDR, Speaker (or `--wav`) | Listen to FM / broadcast FM / AM / SSB |
| [`waterfall`](#waterfall) | SDR (Screen optional) | Live spectrum and waterfall |
| [`scan`](#scan) | SDR | Find active frequencies in a band |
| [`tx_tone`](#tx_tone) | Standard/Advanced SDR | Transmit a test carrier or FM tone |
| [`afsk1200`](#afsk1200) | SDR (Standard/Advanced to send) | 1200-baud AX.25 packet radio (APRS style) |
| [`radio_station`](#radio_station) | Standard/Advanced SDR | Broadcast WAV files / playlists as AM, FM or wide FM |
| [`iqrec`, `iqplay`](#iqrec-and-iqplay) | SDR | Record / replay raw IQ with SigMF metadata |
| [`radiod`](#radiod-radio0) | Standard/Advanced SDR | `radio0`: IP networking over VHF packet radio |
| [`antenna`](#antenna) | computer touching a Feed Point | Resonance, SWR plots, impedance, patterns, power limits |
| [`controllertest`](#controllertest) | Wireless Controller paired | Shows controller input |
| [`peripherals`](#peripherals) | | List attached peripherals and their methods |

`help` lists them under "Radio (SDR block next to the computer)" and "Antennas (computer next to
a feed point)" (it does not list `iw`, `wpa_supplicant`, `wpa_cli` or `tcpdump`).

## Common rules for the SDR programs

All SDR programs share one library (`rust/crates/ecm-radio`), so these apply to all of them:

- **Frequencies**: `146.52e6`, `146.52M`, `146.52MHz`, `7.1 MHz`, `600k`, `2.4G`, `1_000`
  (suffix case-insensitive; `m` means mega). Rates use the same syntax.
  Shown as `146.5200 MHz` (4 decimals for MHz/GHz, 3 for kHz).
- **Options**: `--name value` or `--name=value`. Errors: `unknown option --x`,
  `--x needs a value`, `--x: bad frequency "v"`, `--x: bad number "v"`, `missing frequency`.
- **`--help`/`-h`** prints `usage: <prog> <usage>` and exits 0. A usage error prints
  `<prog>: <error>` and the usage to stderr, exit 1. A runtime error prints `<prog>: <error>`, exit 1.
- **`--sdr NAME`**: empty = first SDR; `0`, `sdr0`, `sdr_0` = index; `/dev/sdr...` = that file;
  anything else = the SDR on that side (`left`, `top`...). No SDR:
  `can't open the first SDR (...); place an SDR block next to the computer`.
- Receivers set `format cs16`, the rate, the frequency, then `gain <db>` (with `--gain`) or AGC.
- Transmitters do the same, then `tx on [dBm]`; a Basic SDR gives
  `can't transmit (...); a Basic SDR is receive-only`. Without `--power` the SDR's maximum
  (37 dBm = 5 W) is used. They keep at most 0.3 s of samples queued ahead of the SDR clock,
  write 4800-sample chunks, wait for the queue to drain at the end, then `tx off`.
- `--iq FILE` on transmitters writes the would-be transmission to a cf32 file plus a
  `.sigmf-meta` instead of keying the SDR (useful to test offline with `afsk1200 recv --iq`).
- Receivers read 20–100 ms at a time (at least rate/50 samples).
- Stop a running program with **Ctrl+T**, or `kill <pid>` from another shell (`ps`, `jobs`).
  Programs started with `&` run in the background.

---

## rx_fm, rx_am, rx_ssb

Listen to a frequency through a **Speaker** touching the computer.

```
rx_fm  <freq> [--wide] [--sdr NAME] [--rate SPS] [--squelch DB] [--gain DB] [--speaker SIDE] [--volume 0-100] [--seconds S] [--wav FILE] [--no-speaker] [--audio-rate HZ]
rx_am  <freq> [same options]
rx_ssb <freq> [usb|lsb] [same options]
```

(The usage line printed by `--help` is the shared one and omits `--wide` and `usb|lsb`.)

| Option | Default | Meaning |
|---|---|---|
| `--wide` (rx_fm) or a second word `wbfm` | narrow FM | Broadcast FM: 240 kS/s, 48 kHz audio, 75 kHz deviation, 75 µs de-emphasis. Needs ≥ 150 kS/s (so a Standard or Advanced SDR) |
| second word `usb`/`lsb` (rx_ssb) | `usb` | Sideband |
| `--rate` | 48000 (240000 wide FM) | SDR sample rate |
| `--squelch DB` | off | Mute below this level (dBFS, 3 dB hysteresis) |
| `--gain DB` | AGC | Fixed SDR gain 0–60 |
| `--speaker SIDE` | first Speaker | Which Speaker |
| `--volume` | speaker default | 0–100 |
| `--seconds S` | until Ctrl+T | Stop after S seconds of samples and print a tone summary |
| `--wav FILE` | | Also write the audio to a WAV file (only written if `--seconds` ends the run) |
| `--no-speaker` | | Only the WAV (needs `--wav`) |
| `--audio-rate HZ` | 24000 (48000 wide FM) | 8000–48000 |

Demodulators: NBFM = 8 kHz low-pass → FM discriminator (5 kHz deviation) → 3.5 kHz low-pass;
AM = 5 kHz low-pass → envelope (carrier-normalised) → 4.5 kHz low-pass; SSB = Weaver demodulator
(300–2700 Hz) → AGC. A wrong mode word: `mode "am" doesn't fit this receiver`.

Output: a header with the chain, a level line every 2 s, and with `--seconds` a summary of the
strongest audio tone. Real output (`sdr_lab`, then the `radio_station` listener):

```
FM 146.5200 MHz (lowpass(8000 Hz) >> fm_demod(5000 Hz) >> lowpass(3500 Hz) >> gain(0.8) >> resample(24000)), 48000 S/s -> speaker  [Ctrl+T stops]
fm: strongest audio tone 996 Hz, 90 dB over the noise; peak level 1.98

AM 11.6000 MHz (lowpass(5000 Hz) >> am_demod >> lowpass(4500 Hz) >> gain(0.8) >> resample(24000)), 48000 S/s -> speaker  [Ctrl+T stops]
  signal   -3.6 dBFS
  signal   -6.3 dBFS
am: strongest audio tone 129 Hz, 85 dB over the noise; peak level 245.35
```

No Speaker: `no speaker (...); place a Speaker next to the computer or use --wav FILE --no-speaker`.

**Limitations**

- `rx_am`'s tone summary is unreliable: the AM demodulator normalises by its own running carrier
  average, so plain noise also comes out at full level, and its start-up transient produces a
  huge "peak level" (hundreds) and a bogus low "tone". Don't use `rx_am --seconds` to decide
  whether a station is there; listen, or use the Handheld Radio's meter.
- `--wav` without `--seconds` never writes the file (Ctrl+T kills the program first).
- `rx_ssb` and the WAV output are tested on the host only, not in a running world.

## waterfall

```
waterfall <freq> [--sdr NAME] [--rate SPS] [--screen | --text] [--min DB] [--max DB] [--size WxH] [--gain DB] [--seconds S]
```

Live spectrum (top third) and scrolling waterfall below, ±rate/2 around `<freq>`.

| Option | Default |
|---|---|
| `--rate` | 48000 |
| `--min`, `--max` | −110, −20 dB (colour scale) |
| `--size WxH` | 256x160 (16..1024 each) |
| `--gain` | AGC |
| display | the terminal's graphics; `--screen` = an attached Screen at its size; `--text` = text lines |

Header: `waterfall: 146.4960 MHz .. 146.5440 MHz (centre 146.5200 MHz), 48000 S/s  [Ctrl+T stops]`.
Text mode prints one line per read: `|<shades ' .:-=+*#%@'>| <dB> dB @ <peak frequency>` (width
up to 78). On exit: `waterfall: <rows> rows; strongest <dB> dB at <freq>`. Errors:
`--screen and --text don't mix`, `--max must be above --min`,
`no Screen attached to this computer (use --text or the terminal display)`.
The graphics mode is tested on the host only.

## scan

```
scan <start> <stop> [--sdr NAME] [--rate SPS] [--threshold DB] [--dwell MS] [--passes N] [--gain DB] [--log FILE]
```

Steps through the band in 0.8 × rate steps, listens `--dwell` ms at each (default 100, 10–10000),
and reports peaks at least `--threshold` dB (default 15) above the median noise, ignoring the
outer 10% of each step. Default rate 48000; default gain is a **fixed 30 dB** (not AGC);
`--passes` repeats (default 1); `--log FILE` appends `<freq_hz> <power_db> <snr_db>` lines.

Real output (`sdr_lab`: one carrier at 146.525 MHz from `tx_tone --offset 5k --power 0`, 12 blocks away):

```
scan: 146.4500 MHz - 146.6000 MHz in 4 steps, 300 ms each, threshold 15 dB
  146.4986 MHz   -32.0 dB  (+27 dB over noise)
  146.5034 MHz   -19.3 dB  (+39 dB over noise)
  146.5202 MHz   -41.6 dB  (+17 dB over noise)
  146.5250 MHz    -0.7 dB  (+58 dB over noise)
  146.5370 MHz   -34.8 dB  (+23 dB over noise)
  146.5490 MHz   -35.5 dB  (+22 dB over noise)
  146.5610 MHz   -17.9 dB  (+39 dB over noise)
scan: 7 active frequencies
```

The real carrier is the strongest line (−0.7 dB); the others are spurs from a signal so strong it
clips at the fixed 30 dB gain. With strong local signals, lower `--gain` (e.g. `--gain 0`).

## tx_tone

```
tx_tone <freq> [--offset HZ] [--fm AUDIO_HZ] [--seconds S] [--power DBM] [--sdr NAME] [--rate SPS] [--iq FILE]
```

Transmit a carrier (at `freq + offset`) or, with `--fm`, an NBFM-modulated audio tone (≈ 4 kHz
peak deviation). Defaults: rate 48000, offset 0 (must be inside ±rate/2), 5 s (0–3600), full SDR
power. Output:

```
tx_tone: 146.5200 MHz FM, 1000 Hz audio, 5.0 s
tx_tone: done
```

or `tx_tone: carrier at 146.5250 MHz, 6.0 s`. Typical uses: test a receiver
(`tx_tone 146.52M --fm 1000 --power 0`), drive an amplifier (`tx_tone 7.1M --seconds 2 --power 37`).
A 0 dBm (1 mW) tone is plenty across a room; 37 dBm next to a receiver overloads it.
Quirk: `--fm` at a rate of 10 kHz or less fails after keying the SDR and leaves transmit on.

## afsk1200

```
afsk1200 send <freq> <SRC> <DEST> <text...> [--repeat N] [--power DBM]
afsk1200 recv <freq> [--seconds S] [--count N]
   both: [--sdr NAME] [--rate SPS] [--iq FILE]
```

Bell 202 AFSK (1200 Hz mark / 2200 Hz space, 1200 baud) carrying AX.25 UI frames (PID 0xF0,
CRC-16 FCS, HDLC flags, NRZI) on NBFM with 3 kHz deviation: the classic APRS packet format.

- Callsigns: 1–6 letters/digits with an optional `-SSID` 0–15 (`bad callsign "X"`). Text ≤ 256
  bytes. Rate ≥ 9600 (default 48000). `--repeat` 1–100.
- `send` keys 0.4 s of carrier before each frame (lets the receiver's AGC settle), prints
  `afsk1200: sent N0CALL-1>APRS: hello from A` (with ` (i/N)` when repeating).
- `recv` prints `afsk1200: listening on 144.3900 MHz [Ctrl+T stops]`, then each frame as
  `SRC>DEST[,DIGI*]:text` (or `[N bytes, not AX.25] [hex]`), and at the end
  `afsk1200: N frame(s) decoded` (plus overflow/bad-FCS counts if any). `--count` stops after N
  frames; `--iq FILE` decodes a recording.

Real output (`sdr_lab`): `N0CALL-1>APRS:hello from A`, `afsk1200: 1 frame decoded`; the
off-frequency control prints `afsk1200: 0 frames decoded`.

## radio_station

```
radio_station <file.wav|file.pcm|playlist.m3u|dir>... <freq> [--mode fm|am|wbfm] [--power DBM] [--loop] [--gap S] [--sdr NAME] [--rate SPS] [--raw-rate HZ] [--iq FILE]
```

Broadcasts audio for Handheld Radios and SDR receivers. The **last** positional argument is the
frequency; everything before it is a source.

| Option | Default | Meaning |
|---|---|---|
| `--mode` | `fm` | `fm` (NBFM, 5 kHz deviation, for the Handheld's VHF band), `am` (for the AM and SW bands), `wbfm` (75 kHz, for 88–108 MHz; rate 240 kS/s, so Standard/Advanced) |
| `--loop` | off | Repeat the whole list forever (not with `--iq`) |
| `--gap S` | 1 | Seconds of silent carrier between songs (0–60) |
| `--power DBM` | SDR max (37) | Transmit power |
| `--rate` | 48000 (240000 wbfm) | SDR rate |
| `--raw-rate HZ` | 8000 | Rate assumed for headerless `.pcm`/`.raw` (16-bit LE mono) |

Sources:

- **WAV**: PCM 8- or 16-bit, mono or stereo (mixed down), any sample rate. Mono 8 kHz is
  smallest. Each song is normalised to a 0.9 peak.
- **Directories**: every `.wav`, `.pcm`, `.raw` file inside, in name order.
- **Playlists** (`.m3u`, `.m3u8`, `.txt`): one path per line, relative to the playlist; blank
  lines and `#` lines (`#EXTM3U`, `#EXTINF`) skipped.
- The title shown is the WAV's `LIST/INFO` `INAM` field, else the file name without extension
  with `_` as spaces.

Output:

```
radio_station: 5 songs on 9.7000 MHz AM, looping
now playing: Mendelssohn - Wedding March (A Midsummer Night's Dream) (1/5, 180 s, 8000 Hz)
```

then one `now playing:` line per song, and `radio_station: done` when a non-looping list ends.
Errors: `missing audio file`, `no audio files to play`, `none of the songs could be played`, a
missing playlist entry is skipped with `radio_station: <path>: <error>`.

Getting files onto the computer: copy them into the world's `computer-data/<computer id>/`
folder (e.g. `<world>/computer-data/<id>/radio/`), which appears as the computer's `/`.

Example (the `radio_station` scenario): `radio_station /radio/playlist.m3u 11.6M --mode am --loop --power 0 &`.
1 mW is plenty within a few dozen blocks; 37 dBm next to a receiver overloads it.

Limitations: 24-bit, float and WAVE_FORMAT_EXTENSIBLE WAVs (and anything else that isn't PCM,
e.g. an `.mp3` named in a playlist) are played as raw 16-bit data, i.e. loud noise, with no
warning. **AM mode clips**: the AM modulator's envelope reaches about 1.7 while the SDR's cs16
samples are clamped at ±1, so AM broadcasts are audibly distorted on peaks. A killed station
(Ctrl+T, `kill`) leaves the SDR's transmit flag on (nothing radiates without new samples).

## iqrec and iqplay

```
iqrec <freq> <file.cf32|file.cs16> [--seconds S] [--rate SPS] [--gain DB] [--sdr NAME]
iqplay <file.cf32|file.cs16|file.sigmf-data> [freq] [--rate SPS] [--power DBM] [--loop] [--sdr NAME]
```

- `iqrec` records S seconds (default 5, max 600) at `--rate` (48000) with AGC or `--gain`. The
  format follows the extension (`.cf32`/`.cfile`/`.fc32` → cf32, `.cs16`/`.ci16`/`.sc16` → cs16,
  else cf32; a hidden `--format` overrides). It writes `<base>.sigmf-meta` (SigMF 1.0, recorder
  `ecm iqrec`, the SDR tier as hardware, start timestamp). Output:
  `iqrec: <n> samples (<fmt>) -> <file> + <meta>`.
- `iqplay` transmits a recording, taking format, rate and frequency from the `.sigmf-meta`
  (arguments override). Without metadata: `<file> has no .sigmf-meta: give --rate`. Output:
  `iqplay: <file> (<n> samples, <rate> S/s) on <freq>`, then `iqplay: done`. `--loop` repeats
  until killed.
- Files open in inspectrum, GNU Radio, etc. Host-tested only.

## radiod (radio0)

```
radiod <iface> up <sdr> <freq> --call CALL[-SSID] [--ip A.B.C.D/N] [--rate SPS] [--power DBM] [--txdelay MS] [--gain DB] [--seconds S] [-v]
```

Creates a network interface (usually `radio0`) that carries IP over 1200-baud packet radio, like
Linux `kissattach` + IP over AX.25. Two computers running it on the same frequency can `ping` and
even `ssh` each other with no cable.

```
ping/ssh -> kernel IP stack -> radio0 -> radiod: Ethernet <-> AX.25 UI <-> KISS <-> AFSK1200 over NBFM -> /dev/sdr
```

| Option | Default | Meaning |
|---|---|---|
| `<iface>` | | 1–15 of `[A-Za-z0-9_]` |
| `<sdr>` | | SDR name (`sdr_0`, `0`, a side...) |
| `--call` | required | Your AX.25 callsign |
| `--ip` | 0.0.0.0/0 | Address; `/24` if no prefix |
| `--txdelay MS` | 300 (10–2000) | Flag preamble before each burst |
| `--gain DB` | AGC | Fixed SDR gain |
| `--power` | SDR max | Transmit power |
| `--rate` | 48000 (≥ 9600) | |
| `--seconds S` | forever | Stop after S seconds of samples |
| `-v` | | Log frames |

- Your MAC is derived from your callsign: `02:ac:` + CRC-32 of `CALL-SSID` (SSID 0 written as
  `CALL-0`). IPv4 uses AX.25 PID 0xCC, ARP 0xCD; other traffic is dropped. Broadcasts go to `QST`.
- Packets over 256 bytes are segmented (AX.25 2.2); TCP SYNs get MSS 216.
- Half duplex: queued frames go out as one burst once the channel has been quiet for 100–250 ms
  (random); a frame heard or a carrier more than 10 dB over the noise floor holds off.
- Output: `radiod: radio0 up on sdr_0 144.3900 MHz as N0CALL-1 (mac 02:ac:c4:bf:6a:2b, 10.44.0.1/24), AFSK1200/NBFM  [Ctrl+T stops]`;
  with `-v`: `radiod: kernel sent 60 bytes (type 0806), 1 frame(s) queued`,
  `radiod: transmitting 1 frame(s), 0.75 s`, `radiod: heard 1 frame(s), 1 packet(s) for us`;
  every 60 s `radiod: tx N pkts / rx N pkts, N stations heard`; at the end (only with
  `--seconds`) a summary.
- Errors: `missing --call CALLSIGN (your station's AX.25 address)`,
  `sdr_0 can't transmit (...); use a Standard or Advanced SDR`,
  `can't create radio0 (already exists, or the kernel has no tun support)`.

Working recipe (the `radio0_lab` scenario, two computers with Standard SDRs 12 blocks apart):

```
A: radiod radio0 up sdr_0 144.39M --call N0CALL-1 --ip 10.44.0.1/24 --gain 10 --txdelay 100 -v --power 0 &
B: radiod radio0 up sdr_0 144.39M --call N0CALL-2 --ip 10.44.0.2/24 --gain 10 --txdelay 100 -v --power 0 &
A: ping 10.44.0.2 -n 3
```

**Limitations**: it is slow (1200 bit/s, about 0.5–0.75 s per packet) and half duplex. Without
`--txdelay 100 --gain 10` pings are unreliable; even with them the scenario has failed
occasionally (39+ passes, 7 failures across the test receipts). `ssh` over radio0 is tested only
between two simulated kernels, not in a running world.

---

## antenna

On a computer touching a Feed Point (peripheral type `antenna`). See [Antennas](antennas.md).

```
antenna [summary]                     resonance, SWR band, ratings
antenna swr <f1> <f2> [points]        SWR plot across f1..f2 (default 41 points, 2..401)
antenna z <f>                         impedance, SWR and efficiency at f
antenna polar <f> [az|el] [--step D]  polar gain plot (step 1..90, default 5)
antenna limits [--amp W]              power limit, weakest link, transmitter verdict
options: --name P (default: first 'antenna' peripheral), --wait S (default 20 s),
         --gfx (terminal display 256x160) | --screen (attached Screen) | --text, --seconds S (default 15)
frequencies: 7.1M, 7100k, 7.1e6, 146.52MHz
```

- `summary`: the analyzer's summary and details lines, then `feed point at (x, y, z) - solved`
  (or `- estimate`).
- `swr`: a 61×12 text plot with the best point marked `@`, then
  `antenna: min SWR 1.62:1 at 7.200 MHz; resonant at 7.255 MHz; 2:1 band 7.091 MHz - 7.389 MHz`
  (real output in [Antennas](antennas.md#the-antenna-program)).
- `z`: `7.100 MHz: Z = R ± jX ohms, SWR 1.23:1, efficiency NN%` or
  `<f>: outside the analysed bands (no reading)`.
- `polar`: a 19×37 character polar plot (`.` = peak ring, `*` = pattern on a 30 dB scale);
  azimuth (`N up`) or elevation (`U up`) cut; footer
  `antenna: peak G dBi at bearing A deg, elevation E deg; horizontal polarization (tilt T deg)`.
- `limits`: the limit text, weakest link, wire and insulation limits, and a transmitter verdict
  (`ok`, `will overheat`, `will arc`), using the amplifier/SDR found on the feedline or `--amp W`
  (`1k` accepted).
- `--gfx` / `--screen` draw the plot as graphics for `--seconds` (default 15).
- While the solver is busy: `antenna: solving...`, then after `--wait` seconds
  `antenna: the solver is still running; showing the estimate`.
- Errors (exit 1 or 2): `no antenna attached (place the computer next to a feed point)`,
  `<name> is a <type>, not an antenna`, `nothing to measure` (bare feed point).

## controllertest

```
controllertest [player 1-4]
```

Draws a live picture of a Wireless Controller's buttons and sticks (320×200, 30 Hz). Hold
**Back + Start** to quit. Messages: `controllertest: <name> (player N). Hold Back + Start to quit.`,
`controllertest: controller disconnected.`, `controllertest: bye.`; with no controller,
`controllertest: no controller connected.` plus pairing hints, exit 1. See
[Wireless Controller](handheld-controller-microwave.md#2-wireless-controller).

## peripherals

```
peripherals [name]
```

No argument: a `NAME  TYPE` table of everything attached (sides and bay slots), or
`No peripherals attached.` plus hints. With a name: `<name> (<type>)` and its methods. An SDR
lists `set_frequency, set_sample_rate, set_gain, set_agc, set_bandwidth, set_format, tx_enable,
timestamp, info`.
