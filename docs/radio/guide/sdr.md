# The SDR and its device files

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md). Programs that use it:
[Programs](programs.md). Python and Rust APIs: [Programming](programming.md).

A software-defined radio turns the radio medium into numbers. The SDR block gives the computer
it touches a stream of **IQ samples** (complex baseband: I + jQ) centred on a tuned frequency, at
a sample rate you choose. Everything a real SDR user does (FM/AM/SSB receivers, a spectrum
waterfall, packet radio, recording and replaying signals) is done in software on the computer.
Standard and Advanced SDRs can also **transmit** any waveform the computer writes.

## 1. Tiers

| | SDR (Basic) | SDR (Standard) | SDR (Advanced) |
|---|---|---|---|
| Tuning | 0.5 – 1700 MHz | 10 kHz – 6 GHz | 1 kHz – 6 GHz |
| Max sample rate | 48 000 S/s | 250 000 S/s | 1 000 000 S/s |
| ADC resolution | 8 bit | 12 bit | 16 bit |
| Noise figure | 8 dB | 5 dB | 5 dB |
| Transmit | no ("sdr_basic is receive-only") | ≤ 37 dBm (5 W) | ≤ 37 dBm (5 W) |

Server config caps the rates (`sdr.basicMaxRate` 48000, `sdr.standardMaxRate` 250000,
`sdr.advancedMaxRate` 1000000), and `sdr.chicoryMaxRate` (250000) caps every tier when computers
run on the Chicory pure-Java WebAssembly interpreter instead of wasmtime (so the Advanced SDR
gives 250 kS/s there). The cap in force is reported as `max_rate`.

Defaults: 100 MHz, 48 000 S/s (or the tier max if lower), AGC on with gain 30 dB, format cs16,
transmit off.

## 2. Placing and connecting

- **Computer**: the SDR must **touch** the computer (any face of either). It is named after the
  side of the computer it touches: `front`, `back`, `left`, `right`, `top`, `bottom`.
  `peripherals` lists it as type `sdr`.
- **Antenna**: with nothing attached, the SDR receives and transmits through a built-in whip
  (an ideal vertical half-wave dipole pattern, 2.15 dBi, at every frequency, 0.6 block above the
  block). Attach a feedline to use a real antenna: a Coax Cable or Hardline that connects to the
  SDR (coax auto-connects to SDR blocks), or a touching amplifier, tuner or a feed point's coax
  side. Then both receive and transmit use the antenna at the far end, with its pattern,
  mismatch and the coax loss ([Antennas](antennas.md)). The chain is re-checked every 4 ticks.
- No FE: the SDR is self-powered (amplifiers in the chain need FE).
- Several SDRs on one computer are allowed (up to six faces); each has its own device files.
- **Ships**: works on Sable ships; the whip position follows the ship.

## 3. Device files

The computer exposes each SDR as two files, in the style of Linux audio devices:

| File | What |
|---|---|
| `/dev/sdr`, `/dev/sdrctl` | the first SDR |
| `/dev/sdr<N>`, `/dev/sdrctl<N>` | the N-th SDR, counting attachment names in **alphabetical order** (`back` < `bottom` < `front` < `left` < `right` < `top`) |
| `/dev/sdr.<side>`, `/dev/sdrctl.<side>` | the SDR on that side, e.g. `/dev/sdr.left` |

Anything else is "no such file" (ENOENT). Programs take `--sdr NAME` with NAME = empty (first),
`0` / `sdr0` / `sdr_0` (index), a `/dev/...` path, or a side name. `sdr_0` is only an alias for
index 0, not a peripheral name.

### 3.1 `/dev/sdr<N>`: samples

Little-endian interleaved I, Q pairs:

| Format | Bytes per sample | Encoding |
|---|---|---|
| `cs16` (default) | 4 | two int16; read: round(clamp(v, −1, 1) × 32767); write: s / 32767 |
| `cf32` | 8 | two float32 |

**Reading** (receive):

- Returns whole samples only (a buffer smaller than one sample: EINVAL).
- The SDR produces samples on the **world clock** (game ticks × 50 ms plus the time since the
  tick started). A read returns what has accumulated since the last read; if nothing has yet, it
  waits (polling every 2 ms) up to 2 s, then returns 0 bytes. In non-blocking mode it returns 0
  at once. The very first read just starts the clock and returns 0.
- If you fall more than **0.25 s** behind, the oldest samples are dropped, the `overflows`
  counter increases and an `sdr_overflow` event is queued.
- The samples are synthesised when you read: every emission the medium says your antenna hears
  within ±rate/2 of the centre, shifted to its frequency offset, delayed by its propagation time,
  at its received power; plus thermal noise of −174 dBm/Hz + 10·log10(rate) + noise figure.
  Analogue signals from other SDRs come through as their real waveforms (decodable); Wi-Fi,
  controller and microwave frames appear as band-limited noise bursts (visible on a waterfall,
  not decodable).
- Full scale: an input of **−10 dBm − gain** reaches amplitude 1. Above that it clips. So with a
  manual gain, **dBm = dBFS − 10 − gain** (dBFS = 10·log10 of mean |x|²).
- **AGC** (default on): once per read, the gain moves by up to ±6 dB to put the peak |I| or |Q|
  at 0.5 (−6 dBFS), within 0–60 dB. Read 10–20 ms at a time so it settles within a packet's
  lead-in.
- There is **one read cursor per SDR**: two programs reading the same SDR split its samples.
- `bw` narrows only which emissions are considered, not the noise: samples always cover the full
  sample rate.

**Writing** (transmit; Standard/Advanced, after `tx on`):

- Writes never block. Samples are scheduled back to back on the world clock; the first write
  starts "now". If you let the queue run dry, the next write starts at "now" again and the
  `underflows` counter increases (`sdr_underflow` event).
- Amplitude: an RMS of 1.0 radiates the set transmit power. cs16 clamps each component to ±1;
  cf32 doesn't clamp (amplitude above 1 radiates above the set power).
- Keep at most ~0.3 s ahead of the `timestamp` (the Rust programs use 0.3 s, Python 0.25 s) so
  stopping is prompt and the medium's view stays current.
- Each written chunk is one `IQ` emission in the medium; the SDR first posts a cancellable
  `RadioTransmitEvent` (kind `sdr`). A write while transmit is off, or one blocked by an event or
  the amplifier chain, fails with EIO.

### 3.2 `/dev/sdrctl<N>`: control

Write text commands, one per line (`\n`). A last line without a newline is applied when the file
is closed (and its error is ignored). Any bad command, missing argument or out-of-range value
makes the write fail with EINVAL.

| Command | Effect |
|---|---|
| `freq <hz>` | Tune. Must be inside the tier's range ("sdr_standard tunes 10000 Hz - 6000000000 Hz, got ...") |
| `rate <sps>` | Sample rate, 1000 to the cap. Resets the read cursor |
| `bw <hz>` | Channel bandwidth considered (0 = the sample rate) |
| `gain <db>` | Manual gain 0–60 dB (turns AGC off) |
| `gain agc` | AGC on |
| `agc 0` / `agc off` / `agc 1` | AGC off / off / on (anything but `0`/`off` turns it on) |
| `format cf32` / `format cs16` | Sample format (anything other than `cf32` selects cs16) |
| `tx on [dBm]` / `tx 1 [dBm]` | Transmit on, at dBm (default and maximum: 37 for Standard/Advanced) |
| `tx off` | Transmit off |

**Reading** the ctl file returns a status snapshot (one `key value` per line), then end of file. Example values:

```
tier sdr_standard
freq 146520000
rate 48000
max_rate 250000
bw 0
gain 30.0
agc 1
format cs16
tx 0
tx_power_dbm 37.0
adc_bits 12
timestamp 1234567890
read 4800
written 0
overflows 0
underflows 0
```

(`timestamp` is the absolute sample index: world-clock µs × rate / 10⁶; it rescales when the rate
changes.)

Shell example (the shell has no redirection, so use a program or Python):

```python
with open("/dev/sdrctl", "wb", buffering=0) as f:
    f.write(b"freq 146.52e6\nrate 48000\ngain 20\n")
print(open("/dev/sdrctl", "rb", buffering=0).read().decode())
# or: import radio; s = radio.open(); s.control("freq 146.52e6"); print(s.status())
```

## 4. The `sdr` peripheral

The same SDR as a peripheral (type `sdr`, named by side). All methods run off the main thread;
errors raise with the Java message.

| Method | Arguments | Returns / notes |
|---|---|---|
| `set_frequency(hz)` | float | range error as above |
| `set_sample_rate(rate)` | int | "sample rate must be 1000-<max>, got N" |
| `set_gain(db)` | float 0–60 | AGC off |
| `set_agc(on)` | bool | |
| `set_bandwidth(hz)` | float ≥ 0 | not saved |
| `set_format(fmt)` | `"cf32"` or anything else (cs16) | |
| `tx_enable(on, power_dbm?)` | bool, float | Basic: "sdr_basic is receive-only" |
| `timestamp()` | | int |
| `info()` | | map of **strings**: `tier, freq, rate, max_rate, bw, gain, agc, format, tx, tx_power_dbm, adc_bits, timestamp, read, written, overflows, underflows, device` (`/dev/sdr.<side>`), `ctl` (`/dev/sdrctl.<side>`) |

Events: `sdr_overflow` and `sdr_underflow` (with the attachment name, which arrives twice in the
event's arguments).

## 5. Saved state

Frequency, sample rate, gain and AGC are saved with the block (and survive Sable ship assembly).
Bandwidth, format, transmit state and transmit power are not: after a reload the SDR is receiving
in cs16. If the saved rate is above a lowered config cap, gain and AGC are not restored either.

## 6. What the SDR does not model

The spec mentions fading, atmospheric noise bursts, oscillator drift, a DC spike and IQ imbalance
in the synthesised samples; the code produces clean signals plus thermal noise only. (Fading and
noise do apply to decoded frames in the medium; the SDR's IQ path uses unfaded levels.)
