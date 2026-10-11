# Programming radios: peripherals, Python and Rust

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

There are three ways to drive radio hardware from a computer:

1. **Ready-made programs** ([Programs](programs.md), [Wi-Fi programs](wifi.md#5-programs)).
2. **Python** (`python` in a terminal): the `peripheral` module for any peripheral, and the
   `radio` module for SDR signal processing.
3. **Rust** compiled to WebAssembly (`wasm32-wasip1`), using the same crates the built-in
   programs use.

## 1. Which blocks are peripherals

A computer sees blocks **touching** its faces and modules **in its bays**; never through cables.

| Peripheral type | Block / item | Where it must be | Name |
|---|---|---|---|
| `sdr` | SDR (any tier) | touching the computer | the side: `front`, `back`, `left`, `right`, `top`, `bottom` |
| `antenna` | Feed Point | touching the computer (any face of the feed point) | side |
| `amplifier` | Power Amplifier (any tier) | touching | side |
| `microwave_radio` | Microwave Radio | touching | side |
| `dish` | Dish (any part) | touching | side |
| `wifi` | Wi-Fi Module | in a bay | `left_bay_1`, `left_bay_2`, `right_bay_1`, `right_bay_2` |
| `controller_receiver` | Controller Receiver Module | in a bay | bay slot |

The Access Point, Antenna Tuner, Burner Generator, wires, coax and the Handheld Radio have no
peripheral. List what you have with `peripherals`, or in Python `peripheral.attached()`.

### Method reference

**`sdr`** — see [SDR §4](sdr.md#4-the-sdr-peripheral): `set_frequency(hz)`, `set_sample_rate(rate)`,
`set_gain(db)`, `set_agc(on)`, `set_bandwidth(hz)`, `set_format("cs16"|"cf32")`,
`tx_enable(on, power_dbm?)`, `timestamp()`, `info()`. Events `sdr_overflow`, `sdr_underflow`.
Samples themselves go through `/dev/sdr*` (the peripheral only controls).

**`antenna`** (reads the cached solution; never blocks; pending answers are estimates):

| Method | Returns |
|---|---|
| `summary()` | the analyzer's summary line |
| `status()` | `{present, solved, pending, kind, summary, details, resonant_hz, analysis_hz, band_low_hz, band_high_hz, efficiency, gain_dbi, wire_m, segments, ground, feed: {x, y, z}}` |
| `resonant_hz()` | float, NaN if none |
| `impedance(hz)` | `{f, r, x, swr, efficiency, in_band}` (outside the swept bands r/x NaN, swr inf) |
| `swr(hz)` | float (inf outside) |
| `efficiency(hz)` | 0..1 |
| `sweep(f0, f1, points=51)` | list of `{f, swr, r, x}`; points 2..401 |
| `peak(hz)` | `{gain_dbi, az, el}`: az is a compass bearing (0 = north = −Z, 90 = east = +X) |
| `pattern(hz, plane="azimuth", step=5, angle=?)` | list of dBi from 0° in `step` increments; plane `azimuth`/`az`/`h`/`horizontal` or `elevation`/`el`/`v`/`vertical`; `angle` = the cut's elevation (azimuth plane) or bearing (elevation plane), default the peak's. In an elevation cut 0 = horizon towards the bearing, 90 = zenith, 180 = the opposite horizon |
| `polarization(hz)` | `{az, el, x, y, z, tilt_deg, sense}`; sense `horizontal` (< 20°), `vertical` (> 70°), else `slant` |
| `power_limit(amp_watts?)` | `{watts, wire_watts, voltage_watts, cause, weakest: {x, y, z, block, part}, transmitter: {name, watts}?, verdict?, text}`; verdict `ok`, `will arc`, `will overheat`, `no antenna` |

Errors: `frequency must be a positive number of Hz`, `stop frequency must be above the start`,
`points must be 2..401`, `step must be 1..90 degrees`, `plane must be 'azimuth' or 'elevation'`.

**`amplifier`**: `status()` → `{tier_w, transmitting, output_w, drive_w, reflected_w, swr,
foldback, temperature_c, fe_per_tick, energy, capacity, supply, antenna_w, bursts}`;
`warnings()` → list of strings.

**`wifi`** (Wi-Fi Module) <a id="wi-fi-module-wifi"></a>:

| Method | Notes |
|---|---|
| `get_mode()` | `"wifi"` or `"controller"` |
| `set_mode(m)` | `"wifi"` / `"controller"`, else `mode must be "wifi" or "controller", got X` |
| `get_channel()` | the Wi-Fi channel (set by the kernel) or the controller channel |
| `set_controller_channel(n)` | 1–13 (default 6) |
| `set_max_power(dbm)` | 0–20 caps the kernel's transmit power |
| `get_mac()` | `aa:bb:cc:dd:ee:ff` |
| `stats()` | `{mode, channel, frequency_hz, rx_filter, max_power_dbm, tx_frames, tx_attempts, tx_failed, rx_frames, rx_dropped, acks_sent, acks_received, cca_deferrals, airtime_us, controller_reports, last_controller_rssi_dbm}` |

Wi-Fi itself (scan, join) is done by the kernel and the Wi-Fi programs, not by peripheral calls.

**`controller_receiver`**: `get_channel()`, `set_channel(1..13)` (paired controllers follow),
`stats()` → `{channel, frequency_hz, received, last_rssi_dbm}`.

**`microwave_radio`**:

| Method | Notes |
|---|---|
| `set_band(ghz)` | 10, 24 or 60; keeps width/channel if valid there |
| `set_channel(n)` | `channel must be 0-N` |
| `set_bandwidth(mhz)` | 10 GHz: 28/56; 24 GHz: 28/56/112; 60 GHz: 250/500/1000/2000 |
| `set_tx_power(dbm)` | −40 to +30 |
| `info()` | `{mac, band_ghz, channel, bandwidth_mhz, frequency_mhz, tx_power_dbm, dish, dish_gain_dbi, beamwidth_deg, linked, peer, rssi_dbm, sinr_db, modulation, rate_mbps, atmosphere_db, rain_mm_h, tx_frames, rx_frames, tx_bytes, rx_bytes, filtered_frames, faded_frames, stray_frames}` (`peer`: the one radio this one pairs with; `stray_frames`: frames from radios it doesn't pair with) |

**`dish`**:

| Method | Notes |
|---|---|
| `set_aim(yaw, pitch)` | degrees; yaw 0 = south, 90 = west, 180 = north, −90 = east; pitch −90..90 |
| `nudge(dyaw, dpitch)` | relative |
| `get_aim()` | `{yaw, pitch, world_yaw, world_pitch}` (world values differ on a turned ship) |
| `aim_at(x, y, z)` | aim at a world point (accounts for ship rotation); returns `get_aim()` |
| `align(span=10)` | search ±span° (≤ 90) for the strongest radio on the same band/width/channel; applies the best aim if it is above sensitivity; returns `{found, rssi_dbm, yaw, pitch, radios, points}` |
| `info()` | `{size, diameter_m, radio, frequency_mhz, gain_dbi, beamwidth_deg, x, y, z}` |

## 2. Python

Start `python` in a terminal. Type lines at `>>>`; `exit()` returns to the shell. Scripts:
`python /path/script.py`. Files can be written with `import shell; shell.write_file(path, text)`
(or `edit`).

### 2.1 The `peripheral` module

```python
import peripheral
peripheral.attached()            # [('left', 'sdr'), ('left_bay_1', 'wifi'), ...]
peripheral.names(); peripheral.is_present("left"); peripheral.get_type("left")
peripheral.get_methods("left")
peripheral.call("left", "set_frequency", 146.52e6)
s = peripheral.find("sdr")       # first of a type, or None; find_all("sdr") for all
s.set_frequency(146.52e6); s.set_gain(20); print(s.info()["device"])   # /dev/sdr.left
w = peripheral.wrap("left_bay_1")   # by name; .name, .type, .methods(), .call(m, *args)
ev = peripheral.pull_event("sdr_overflow", timeout=5)   # (event, attachment, *args) or None
```

Errors from the block raise `peripheral.PeripheralError` with its message. Arguments may be None,
bool, int, float, str, bytes, list, tuple, dict. `pull_event(filter, timeout)`: None waits
forever, 0 only checks; other events are discarded while waiting.

Example (the `microwave_link` scenario, typed at `>>>` on each host):

```python
import peripheral
r = peripheral.find("microwave_radio")
d = peripheral.find("dish")
r.set_tx_power(-40)
print(d.aim_at(x, y, z))             # the far dish's centre, world coordinates
# real: {'yaw': -92.20259816176582, 'pitch': 0.0, 'world_yaw': -92.20259816176582, 'world_pitch': 0.0}
d.nudge(30, 0)                       # control: link lost
print(d.align(40))                   # {'found': True, ...}: link back
```

### 2.2 The `radio` module (SDR flowgraphs)

`import radio` gives GNU-Radio-style flowgraphs whose blocks run natively (the same Rust DSP as
the programs). Streams are one of three **kinds**: `radio.COMPLEX` (IQ), `radio.REAL` (audio),
`radio.FRAMES` (packets). Compose with `>>`; a `Flowgraph` checks kinds and rates when it is
built (a mismatch raises `ValueError`).

```python
import radio
sdr = radio.open()                         # first SDR; or open("left"), open(0), open("sdr_0")
sdr.tune("146.52M", rate=48000)
fg = radio.Flowgraph(sdr >> radio.fm_demod(5e3) >> radio.lowpass(3e3) >> radio.speaker())
print(fg.describe())                       # fm_demod(5000 Hz) >> lowpass(3000 Hz) >> ...
fg.run(seconds=10)
```

**Sources**

| Source | Kind | Notes |
|---|---|---|
| `radio.open(name="", data_path=None, ctl_path=None)` → `SDR` | complex | Raises `RadioError("no SDR at ...")` if missing |
| `radio.tone(freq, rate=48000, amplitude=0.5, seconds=None, kind=COMPLEX)` | complex/real | Test tone |
| `radio.file_source(path, fmt=None, rate=None, repeat=False)` | complex | cf32/cs16/...; rate and frequency from `<base>.sigmf-meta` (else pass `rate=`) |
| `radio.wav_source(path, repeat=False, raw_rate=8000)` | real | WAV PCM 8/16-bit (24-bit, float, compressed or non-WAV files raise `ValueError`), or headerless 16-bit from a `.pcm`/`.raw` file |
| `radio.frames_source(frames, rate=48000)` | frames | list of bytes, one per read |

**`SDR` object**: `.tune(freq, rate=None, bw=None)` (accepts `"146.52M"`), `.set_rate(r)`,
`.gain(db)` (manual), `.agc(on=True)`, `.set_format(fmt)`, `.tx(on=True, power=None)`,
`.timestamp()`, `.status()` (dict of the ctl status), `.control("freq 7.1e6")`, `.read(n=4096)` →
`Samples` (empty on a 2 s timeout), `.write(samples)` (paced 0.25 s ahead), `.close()`;
attributes `.rate`, `.freq`, `.format`. Used as the **last** item of a flowgraph it transmits
(a `resample` to its rate is added automatically); call `.tx(True, dbm)` first. `.close()` (which
a flowgraph does when it ends) lets the queued samples go out (at most 1.25 s) and switches transmit
off, so call `.tx(True, dbm)` again before the next transmitting flowgraph.

**Blocks** (all take and return the noted kinds):

| Block | Kinds | Notes |
|---|---|---|
| `fm_demod(deviation=5e3, tau=0.0)` | complex → real | NBFM; `tau` = de-emphasis |
| `wbfm_demod(deviation=75e3, tau=75e-6)` | complex → real | broadcast FM (≥ 150 kS/s) |
| `fm_mod(deviation=5e3, tau=0.0)` | real → complex | |
| `am_demod()` / `am_mod(index=0.8)` | complex ↔ real | |
| `ssb_demod(mode="usb")` / `ssb_mod(mode="usb")` | complex ↔ real | needs ≥ 6000 S/s |
| `lowpass(cutoff)`, `resample(rate)`, `decimate(factor)`, `gain(g)`, `dc_block()` | any | |
| `agc(target=0.5)` | any | |
| `squelch(threshold=-40.0)` | complex | dBFS |
| `shift(hz)` | complex | frequency shift |
| `real()` / `to_complex()` | complex → real / real → complex | |
| `afsk1200()` | frames ↔ real | modulates frames, demodulates audio (use with `fm_mod(3e3)`/`fm_demod(3e3)`) |
| `fsk(baud=1200, deviation=2400)`, `bpsk(baud=2400)`, `chirp(sf=7, bw=12000)` | frames ↔ complex | fsk: rate ≥ 4×baud; bpsk: integer samples/symbol ≥ 2; chirp: sf 6–12, rate a multiple of bw |

**Sinks**

| Sink | Kind | Notes |
|---|---|---|
| `speaker(side=None, volume=None, rate=None)` | real | resamples to the input rate if 8–48 kHz, else 24 kHz |
| `wav_sink(path)` | real | 16-bit WAV |
| `iq_sink(path, fmt=None, freq=0.0, description="")` | complex | writes `.sigmf-meta` |
| `collect()` | any | `.samples`, `.frames` afterwards (added automatically if the graph has no sink) |
| `frames(callback=None, echo=True)` | frames | echoes `SRC>DEST: text` |
| an `SDR` | complex | transmit |

**`Flowgraph(pipeline, block_size=4096)`**: first item a source, last a sink (or SDR), blocks in
between. `.describe()`, `.step()`, `.run(seconds=None, samples=None, timeout=10.0)` (limits count
input samples; also ends after `timeout` seconds in a row without input, e.g. a silent SDR whose
reads come back empty, setting `.idle = True`; `timeout=None` waits forever; returns the sink;
always closes the chain, the sink and the source), `.stop()` (ends `run()` after the current
buffer, e.g. from a `frames()` callback), `.close()`.

**`Samples`**: `.kind`, `.rate`, `len()`, `.to_list()`, `.power_db()`,
`Samples.from_list(values, rate, kind=COMPLEX)`.

**Helpers**: `radio.ax25(dest, src, info, pid=0xF0)` → frame bytes; `radio.parse_ax25(frame)` →
dict (`"text"`, ...) or None; `radio.find_signals(samples, center=0.0, nfft=1024, threshold=15.0)`
→ `[(freq, power_db, snr_db)]`; `radio.parse_freq("146.52M")`; `radio.RadioError`.

Examples:

```python
# AFSK1200 packet through an IQ file and back
msgs = [radio.ax25("APRS", "N0CALL-1", "hello from ecm")]
radio.Flowgraph(radio.frames_source(msgs, rate=48000) >> radio.afsk1200() >> radio.fm_mod(3e3)
                >> radio.iq_sink("packet.cf32", freq=144.39e6)).run()
got = []
radio.Flowgraph(radio.file_source("packet.cf32") >> radio.fm_demod(3e3) >> radio.afsk1200()
                >> radio.frames(got.append, echo=False)).run()
print(radio.parse_ax25(got[0])["text"])

# Transmit a tone, then look for signals (Standard/Advanced SDR)
sdr = radio.open(0); sdr.tune("146.52M", rate=24000)
sdr.tx(True, 20)
radio.Flowgraph(radio.tone(500, rate=48000, seconds=2) >> sdr).run()   # ends with tx off
print(radio.find_signals(sdr.read(8192), center=sdr.freq))

# Record 5 s of FM audio to a WAV
fg = radio.Flowgraph(radio.open().tune("146.52M") >> radio.fm_demod() >> radio.wav_sink("out.wav"))
fg.run(seconds=5)
```

Caveats: `frames_source` has no gap between frames; `from radio import *` shadows
the builtin `open`. In a running world the scenario only checks that `import radio` works; the
flowgraphs are tested on the host with a real interpreter.

## 3. Rust

The built-in programs are Rust crates in `rust/wasm-programs/*` built for `wasm32-wasip1`, using
shared crates:

| Crate | What you get |
|---|---|
| `ecm-host-abi` | Calls into the computer: `peripheral` (`list()`, `methods(name)`, `call(name, method, &[Value])`, `wait_event(filter, timeout_ms)`; `Value::{Nil, Bool, Int, Float, Str, Bytes, List, Map}`), `wifi` (`ctl("status")` → the kernel's Wi-Fi control text protocol, `ScanEntry`, `WifiEvent`), `socket` (`AF_PACKET` raw sockets: `packet_socket(ifname, ethertype)`, `sendto_ll`, `recvfrom_ll`, `poll`), `tun` (`AF_ECM_TUN` interfaces: `open(name, mac, ip, prefix)`, `read`, `write`), `dhcp` (`start`, `release`, `stop`, `status(ifname)`) |
| `ecm-radio` | SDR programs' library: `device` (`Sdr::open(name)`, `tune`, `set_rate`, `set_gain`, `set_agc`, `set_format`, `set_tx(on, Option<dBm>)`, `read`, `read_at_least`, `read_n`, `write`, `timestamp`, `status`; `TxPacer`), `blocks` (`Chain::build(Vec<Spec>, Kind, rate)`, `Buf::{C, R, F}`, `Tone`, `receiver`, `transmitter`), `modem` (AFSK/FSK/BPSK/chirp packet modems), `spectrum` (`find_signals`, waterfall), `sigmf`, `audio` (WAV), `units` (`parse_freq`, `fmt_freq`), `link` (radio0 AX.25/KISS), `cli` (`split`), `run` (`setup_rx`, `setup_tx`, `TxOut`, `fail`) |
| `ecm-dsp` | DSP: FFT, Welch, FIR/IIR, NCO, resamplers, AM/FM/SSB/CW/FSK/PSK/chirp modems, AGC, squelch, PLL/Costas, AX.25/HDLC/KISS, CRC, convolutional/Viterbi, Reed-Solomon, IQ sample formats |
| `ecm-wifi` | The 802.11/WPA2 station used by the kernel |

### A minimal program: measure the power at a frequency

`rust/wasm-programs/sigpower/Cargo.toml`:

```toml
[package]
name = "sigpower"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "sigpower"
path = "src/main.rs"

[dependencies]
ecm-radio = { workspace = true }
ecm-dsp = { workspace = true }
```

`rust/wasm-programs/sigpower/src/main.rs`:

```rust
//! `sigpower` — measure the power received at a frequency.
//!
//!   sigpower <freq> [--bw HZ] [--gain DB] [--seconds S] [--sdr NAME]

use ecm_dsp::complex::{mean_power, to_db};
use ecm_radio::blocks::{Buf, Chain, Kind, Spec};
use ecm_radio::{cli, run, units};

const USAGE: &str = "<freq> [--bw HZ] [--gain DB] [--seconds S] [--sdr NAME]";
const RATE: u32 = 48_000;

fn main() {
    let o = cli::split(&run::args(), &["bw", "gain", "seconds", "sdr"], &["help"])
        .unwrap_or_else(|e| run::fail("sigpower", USAGE, e));
    if let Err(e) = measure(&o) {
        eprintln!("sigpower: {e}");
        std::process::exit(1);
    }
}

fn measure(o: &cli::Opts) -> Result<(), String> {
    let freq = o.pos.first().and_then(|s| units::parse_freq(s)).ok_or("missing or bad <freq>")?;
    let bw = o.freq("bw")?.unwrap_or(12_500.0);
    // A fixed gain (not AGC) so dBFS converts to dBm: full scale = -10 dBm - gain.
    let gain = o.num("gain")?.unwrap_or(30.0);
    let seconds = o.num("seconds")?.unwrap_or(1.0);
    let name = o.str("sdr").unwrap_or_default();

    let mut sdr = run::setup_rx(&name, freq, RATE, Some(gain))?;
    // Channel filter: only the power within +-bw/2 of the tuned frequency.
    let mut chan = Chain::build(vec![Spec::Lowpass { cutoff: (bw / 2.0) as f32 }], Kind::Complex, RATE as f64)?;
    sdr.read_at_least(RATE as usize / 10, RATE as usize / 10).map_err(|e| e.to_string())?; // drop stale samples

    let want = (seconds * RATE as f64) as usize;
    let (mut sum, mut n) = (0.0f64, 0usize);
    while n < want {
        let x = sdr.read_at_least(RATE as usize / 20, RATE as usize / 10).map_err(|e| e.to_string())?;
        if x.is_empty() {
            return Err("the SDR stopped delivering samples".into());
        }
        let Buf::C(y) = chan.process(Buf::C(x))? else { unreachable!() };
        sum += mean_power(&y) as f64 * y.len() as f64;
        n += y.len();
    }
    let dbfs = to_db((sum / n as f64) as f32) as f64;
    println!("{}: {:.1} dBFS = {:.1} dBm in {:.0} Hz (gain {gain} dB)", units::fmt_freq(freq), dbfs, dbfs - 10.0 - gain, bw);
    Ok(())
}
```

(This exact program was compiled against the repository crates for `wasm32-wasip1` while writing
this guide; it was not run in a world.)

Build and install:

1. Add `"wasm-programs/sigpower",` to `[workspace] members` in `rust/Cargo.toml`
   (`rust/.cargo/config.toml` already sets the wasm32-wasip1 flags).
2. Build: `cd rust; cargo build --release --target wasm32-wasip1 -p sigpower` →
   `rust/target/wasm32-wasip1/release/sigpower.wasm`.
3. Either copy that `.wasm` into a computer's storage (`<world>/computer-data/<computer id>/bin/`
   or the folder you'll run it from; the shell looks for `<cwd>/<cmd>.wasm`, `bin/<cmd>.wasm`,
   `bin/<cmd>`, then the built-in programs), or bundle it with the mod: `bash scripts/stage-wasm.sh`
   builds every program into `wasm-bin/` and regenerates the manifest, and the next mod build
   ships it. Add a line to `rust/wasm-programs/help/src/main.rs` if `help` should list it.
4. Run: `sigpower 146.52M --seconds 2`.

For peripherals from Rust:

```rust
use ecm_host_abi::peripheral::{self, Value};
let list = peripheral::list()?;                                       // [(name, type)]
let info = peripheral::call("left", "info", &[])?;                     // Value::Map(...)
peripheral::call("left", "set_frequency", &[Value::Float(146.52e6)])?;
```

## 4. Lower-level interfaces

- **`/dev/sdr*` and `/dev/sdrctl*`**: [SDR §3](sdr.md#3-device-files).
- **Wi-Fi control** (`WIFI_CTL`, the text protocol `iw`/`wpa_supplicant` use):
  [Wi-Fi §4](wifi.md#4-inside-the-computer-wlan0-and-the-wi-fi-control-protocol).
- **AF_PACKET raw Ethernet sockets** (protocol = EtherType, bound to an interface; how
  `wpa_supplicant` and `dhcpd` work): address `[family u16 = 17][protocol u16 network order]
  [ifname 12 bytes]`; whole Ethernet frames without FCS, 14–1518 bytes; 64 frames queue per
  socket; see `docs/radio/CONTRACTS.md`.
- **Tun interfaces** (how `radiod` makes `radio0`): a socket of domain `AF_ECM_TUN` (1024);
  `setsockopt(SOL_TUN, TUN_SETIFF, [mac][ip][prefix][name])` creates the interface; `send` injects
  a frame as received, `recv` returns the next frame the stack sent (or −2 when empty); closing
  removes it.
- **Kernel DHCP client** via private netlink `RTM_ECM_DHCP` (0x7E10): start / release / status /
  stop (`ecm_host_abi::dhcp`).
