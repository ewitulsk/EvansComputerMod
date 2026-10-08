# SDR programs, the `radio` Python module and `radio0`

Lanes 6C and 6E of the [implementation guide](IMPLEMENTATION_GUIDE.md). Everything
here runs on a computer next to an SDR block (`/dev/sdr*`, see
[CONTRACTS.md](CONTRACTS.md)).

## Layout

| Piece | Where | Notes |
| --- | --- | --- |
| Shared library | `rust/crates/ecm-radio` | device files, flowgraph blocks, packet modems, WAV, SigMF, scanner/waterfall, CLI parsing, program loops, the `radio0` link layer and TNC. Pure `std`, host-tested. |
| Python module | `rust/wasm-programs/python/src/radio_module.rs` (`_radio`) + `radio.py` | Bootstrap installs `radio` like `peripheral`/`audio`. The python crate now has a library target so `_radio` is tested on the host with a real interpreter. |
| Programs | `rust/wasm-programs/{rx_fm,rx_am,rx_ssb,waterfall,scan,tx_tone,afsk1200,radio_station,iqrec,iqplay,radiod}` | thin `main`s over `ecm_radio::run` (waterfall and radiod also use the host ABI). |
| Tun interfaces | `rust/operating-system/rust/src/net/radio0.rs`, `rust/crates/ecm-host-abi/src/tun.rs` | kernel side + program API. |
| In-world | `testing/scenario/RadioScenarios.java` (`sdr_lab`, `radio0_lab`), `testing/v1211/RadioSdrProgramTests.java` | |

## Python

```python
import radio
sdr = radio.open("sdr_0")            # "", index, sdr_N, attachment name, or data_path=/ctl_path=
sdr.tune(146.52e6, rate=48_000)
fg = radio.Flowgraph(sdr >> radio.fm_demod(5e3) >> radio.lowpass(3e3) >> radio.speaker())
fg.run(seconds=10)
```

- Streams are `complex`, `real` or `frames`; buffers are `radio.Samples` (packed
  float32, `to_list()`, `power_db()`). Blocks are specs built natively once the
  input kind and rate are known; mismatches raise when the `Flowgraph` is built.
- Blocks: `fm_demod, wbfm_demod, fm_mod, am_demod, am_mod, ssb_demod(mode), ssb_mod(mode),
  lowpass, resample, decimate, agc, squelch, shift, gain, dc_block, real, to_complex`,
  and the packet modems `afsk1200(), fsk(baud, deviation), bpsk(baud), chirp(sf, bw)`,
  which modulate when fed frames and demodulate when fed samples.
- Sources: SDR, `tone()`, `file_source()` (rate/frequency from `.sigmf-meta`), `wav_source()`, `frames_source()`.
- Sinks: `speaker()` (resamples to a rate the Speaker takes), `wav_sink()`, `iq_sink()` (+SigMF),
  `collect()`, `frames()`, or an SDR (transmit, paced to the world clock).
- Helpers: `ax25()`, `parse_ax25()`, `find_signals()`, `parse_freq()`.

## Programs

```
rx_fm <freq> [--wide] [--squelch DB] [--gain DB] [--speaker SIDE] [--seconds S] [--wav F [--no-speaker]]
rx_am <freq> ...            rx_ssb <freq> [usb|lsb] ...
waterfall <freq> [--screen | --text] [--size WxH] [--min DB] [--max DB]
scan <start> <stop> [--threshold DB] [--dwell MS] [--passes N] [--log FILE]
tx_tone <freq> [--offset HZ] [--fm AUDIO_HZ] [--seconds S] [--power DBM] [--iq FILE]
afsk1200 send <freq> <SRC> <DEST> <text...> [--repeat N] | recv <freq> [--seconds S] [--count N]  [--iq FILE]
radio_station <file.wav|pcm> <freq> [--mode fm|am|wbfm] [--loop] [--power DBM] [--iq FILE]
iqrec <freq> <file.cf32|.cs16> [--seconds S]      iqplay <file> [freq] [--loop]
radiod radio0 up sdr_0 144.39e6 --call N0CALL-1 --ip 10.44.0.1/24 [--txdelay MS] [--seconds S]
```

Timing rules from the SDR contract: reads block until the world clock has
samples; writes never block, so transmitters keep at most 0.3 s ahead of the
`timestamp` (`device::TxPacer`). The SDR's AGC moves once per read, so
receivers read 10-20 ms at a time to settle within a packet's lead-in.
`--iq FILE` on the transmitters writes the would-be transmission (cf32 +
SigMF) instead of keying the SDR, for offline checks with `afsk1200 recv --iq`
or inspectrum.

## `radio0` design

A KISS-TNC style bridge, like Linux `kissattach` + IP over AX.25, split between
the kernel and a userspace daemon:

```
ping/ssh -> kernel stack -> radio0 (Ethernet iface) -> tun socket -> radiod
radiod: Ethernet <-> AX.25 UI (link::Link) <-> KISS <-> AFSK1200 over NBFM (modem) <-> /dev/sdr
```

**Kernel (tun interfaces).** No new host functions: a tun is a socket of
domain `AF_ECM_TUN` (1024) on the existing socket syscalls, so Java and the
simulator forward it unchanged. `Kernel::sock_ipc` offers each call to
`Net::tun_ipc` before `net::ipc` (the same interception the sshd session calls
use), so `net/ipc.rs` and the parallel AF_PACKET work are untouched.
`setsockopt(SOL_TUN, TUN_SETIFF, [mac][ip][prefix][name])` adds the interface
(link up, address set); `send` injects a frame as if received on it; `recv`
returns the next frame the stack sent on it or `TUN_EMPTY` (-2) at once; socket
ids start at `TUN_ID_BASE` (0x1000); `close` or process exit removes it.
`Net::flush` hands stack output for tun interfaces to `Tuns::on_tx` (64-frame queue).

**Link layer.** IPv4 rides in UI frames with PID 0xCC and ARP with PID 0xCD; the
Ethernet header is rebuilt on receive. Each station's MAC is derived from its
callsign (`02:ac:` + CRC-32 of `CALL-SSID`), so a received frame's source MAC
follows from its AX.25 source; unicast destinations come from a table learned
from received frames, broadcasts go to `QST`. Packets over 256 bytes use AX.25
2.2 segmentation (PID 0x08); TCP SYNs get their MSS clamped to 216 so segments
fit one frame. Frames from other stations to other callsigns, and our own
echoes, are ignored.

**Channel access.** Half duplex: queued frames go out as one burst (one TX
delay preamble) once no frame has arrived for 100-250 ms (hold-off with jitter).
The receiver uses the SDR's AGC with 10 ms reads.

## Tests

- `cargo test -p ecm-radio`: blocks (FM/AM/SSB receivers recover tones, squelch,
  packet modems both ways), modems, device files on plain files, pacing, SigMF,
  WAV, scanner, waterfall rendering, CLI parsing of every program, link layer
  (IP <-> AX.25 <-> KISS, segmentation, MSS clamp), and the program loops through `--iq` files.
- `cargo test -p python`: RustPython with `_radio` + radio.py: FM demod of a
  synthetic cs16 IQ "SDR" gives the 1 kHz tone; AFSK1200 frames round trip
  through `iq_sink`/`file_source` with SigMF; composition rules and SDR control.
- `cargo test -p terminal-os radio0`: tun syscalls; two kernels with `radio0`
  and the full TNC (AX.25/KISS/AFSK/NBFM IQ) between them: ping and a TCP
  connection carrying data both ways.
- GameTests `ecm_radio` (`sdr_lab`, `radio0_lab`), paced to real time because the
  SDR clock is game time.
