# SDR programs, the `radio` Python module and `radio0`

Lanes 6C and 6E of the [implementation guide](IMPLEMENTATION_GUIDE.md). Everything
here runs on a computer next to an SDR block (`/dev/sdr*`, see
[CONTRACTS.md](CONTRACTS.md)). The Advanced SDR also has a framed PHY mode for
cellular; see [Framed PHY mode](#framed-phy-mode-advanced-sdr) and
[When to use which mode](#when-to-use-which-mode).

## Layout

| Piece | Where | Notes |
| --- | --- | --- |
| Shared library | `rust/crates/ecm-radio` | device files, flowgraph blocks, packet modems, WAV, SigMF, scanner/waterfall, CLI parsing, program loops, the `radio0` link layer and TNC. Pure `std`, host-tested. |
| Python module | `rust/wasm-programs/python/src/radio_module.rs` (`_radio`) + `radio.py` | Bootstrap installs `radio` like `peripheral`/`audio`. The python crate now has a library target so `_radio` is tested on the host with a real interpreter. |
| Programs | `rust/wasm-programs/{rx_fm,rx_am,rx_ssb,waterfall,scan,tx_tone,afsk1200,radio_station,iqrec,iqplay,radiod}` | thin `main`s over `ecm_radio::run` (waterfall and radiod also use the host ABI). |
| Tun interfaces | `rust/operating-system/rust/src/net/radio0.rs`, `rust/crates/ecm-host-abi/src/tun.rs` | kernel side + program API. |
| In-world | `testing/scenario/RadioScenarios.java` (`sdr_lab`, `radio0_lab`), `FramedScenarios.java` (`sdr_framed`), `testing/v1211/RadioSdrProgramTests.java` | |
| Framed PHY mode | `radio/framed/*` (Java), `ecm_radio::framed`, `rust/wasm-programs/framed`, `radio.framed()` | [below](#framed-phy-mode-advanced-sdr) |

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
radio_station <file.wav|pcm|playlist.m3u|dir>... <freq> [--mode fm|am|wbfm] [--loop] [--gap S] [--power DBM] [--iq FILE]
iqrec <freq> <file.cf32|.cs16> [--seconds S]      iqplay <file> [freq] [--loop]
radiod radio0 up sdr_0 144.39e6 --call N0CALL-1 --ip 10.44.0.1/24 [--txdelay MS] [--seconds S]
```

Timing rules from the SDR contract: reads block until the world clock has
samples; writes never block, so transmitters keep at most 0.3 s ahead of the
`timestamp` (`device::TxPacer`). The SDR's AGC moves once per read, so
receivers read 10-20 ms at a time to settle within a packet's lead-in.
`radio_station` plays its files, playlists (one path per line, `#` comments,
relative to the playlist) and directories (audio files in name order) one
after another with `--gap` seconds (default 1) of carrier between songs, and
prints `now playing: <title>` for each (the WAV's LIST/INFO title, else the
file name); `--loop` repeats the whole list.
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

## Framed PHY mode (Advanced SDR)

A second mode of the SDR blocks, added for Cellular v2 ([CELLULAR_V2.md](../gui/CELLULAR_V2.md)).
In IQ mode (everything above) the program is the modem: it reads and writes samples. In framed
mode the radio is the modem: the program picks a PHY profile and sends and receives whole frames
with measurements, and the radio does the per-frame work a real baseband does in silicon.

### When to use which mode

| Use | Mode | Why |
| --- | --- | --- |
| Listening to broadcast radio, scanners, waterfalls | IQ | You want the signal itself. |
| Your own modulation, DSP experiments, AFSK/FSK/chirp packet, `radio0` | IQ | The program is the modem; any waveform works end to end. |
| Cellular (LTE, mmWave): cell sites, phones, anything that needs many frames per second with link adaptation | Framed | Real base stations offload the PHY to dedicated silicon; a Wasm program can't do LTE signal processing in real time. |
| A protocol that needs scheduled slots, ACK/retransmit timing or beacons on the airtime clock | Framed | The radio keeps exact airtime-clock timing; a program only sees the clock in server ticks. |

One SDR is in one mode at a time. In framed mode `/dev/sdr<N>` fails with EIO; in IQ mode the
frames file does. Switching to framed mode keys the IQ transmitter off.

**Tiers.** Only the Advanced SDR has framed mode (Basic and Standard reject `mode framed`). Its
framed mode includes the front ends of both profiles: LTE (703-841 MHz) and mmWave (27.5-28.4 GHz),
so mmWave works although the IQ tuner stops at 6 GHz. The LTE / mmWave Modem modules can be the
same radio built into a module (below).

### Profiles

The v1 cellular PHY tables moved here as reusable profiles: `radio/framed/PhyProfile` (was
`cellular/CellTech`: channel plans, powers, antenna gains, noise figures, slot length) and
`radio/framed/CqiTable` (was `cellular/CellPhy`: the 3GPP CQI table, rates, airtime). (The v1 modem engine that first used them was deleted in Cellular v2 phase 4.)

| | `lte` | `mmwave` |
| --- | --- | --- |
| Channels (downlink / uplink centre) | 1-6: 791-841 / 703-753 MHz | 1-4: 28.05-28.35 / 27.55-27.85 GHz |
| Bandwidths | 1.4, 3, 5, 10 MHz (centred in the 10 MHz channel) | 50, 100 MHz |
| Uplink sub-channels | 4 (a TX record's `subchannel` 1-4 is one quarter of the carrier) | 4 |
| Slot | 1 ms (subframe) | 125 µs (NR numerology 3) |
| Default power, base station / UE | 43 / 23 dBm (capped by the hardware: 37 dBm exciter, more through an amplifier) | 30 / 23 dBm |
| CQI | 1-15 (QPSK-64QAM) | 1-18 (adds 256QAM) |

Roles: `bs` transmits on the downlink and hears the uplink; `ue` the reverse (FDD, full duplex).

### Interface

Peripheral methods (type `sdr` on the Advanced SDR, `framed_radio` on a modem module; the frames
themselves never go through methods):

| Method | Meaning |
| --- | --- |
| `set_mode("iq" \| "framed")`, `get_mode()` | SDR only |
| `set_phy(profile)` | `lte` / `mmwave`: channel 1, full bandwidth, the role's default power; clears peers and beacons |
| `set_role("bs" \| "ue")`, `set_channel(n)`, `set_phy_bandwidth(hz)`, `set_power(dbm)` | a retune flushes frames still queued from the old channel |
| `set_address(hex)`, `set_promiscuous(bool)` | 32-bit frame address (default from the block's id); promiscuous delivers frames to anyone |
| `set_beacon(index, period_ms, payload)` | the same as a BEACON record |
| `framed_status()` | `{framed, phy, role, channel, phy_bw, tx_freq, rx_freq, power_dbm, max_power_dbm, address, promisc, slot_us, slot, ta_us, tx_frames, tx_bytes, rx_frames, rx_bytes, rx_collisions, rx_dropped, harq_acked, harq_retx, harq_failed, beacons_sent, blocked, beacons, frames}` |
| `peers()` | `{address: {rssi_dbm, sinr_db, cqi, reported_cqi, age_ms, delay_us, frames}}` |
| `profiles()` | the profiles with their channel plans |
| `slot()` | the current slot number on the airtime clock |

Event: `framed_rx` when records start waiting in the frames file.

Device files:

- `/dev/sdr<N>.frames` (`/dev/sdr.frames` = the first SDR, `/dev/sdr.<attachment>.frames`), or
  `/dev/frames.<attachment>` for any framed radio (an SDR or a modem module): the frames.
- Control lines in `/dev/sdrctl<N>` (SDRs) or `/dev/framesctl.<attachment>` (any framed radio):
  `mode iq|framed`, `phy lte|mmwave`, `role bs|ue`, `channel N`, `phy_bw HZ`, `power DBM`,
  `address HEX`, `promisc 0|1`. Reading gives `key value` lines (in framed mode the SDR appends
  `mode framed` and the `framed_status` keys after its IQ keys).

The frames file holds binary records, little-endian, each starting with `u16 len` (the whole
record) and `u8 type`. Write whole records (several per write are fine; a partial or malformed one
fails the write with EINVAL); a read returns as many whole records as fit (read with a 2048-byte
buffer: the largest record is 1544 bytes). A blocking read waits up to 2 s for one record and then
returns 0 bytes; with O_NONBLOCK an empty file gives EAGAIN. The file is pollable with
`poll_oneoff` (fd_read subscription) and `sock_poll` next to sockets, like the UI fd.

```
written by the program
  TX      0x01  u16 len, u8 type, u8 flags (bit0 HARQ), u32 dst (0xFFFFFFFF = broadcast),
                u64 slot (0 = the next slot), u8 cqi (0 = automatic), u8 subchannel (0 = whole
                carrier, 1..4 = a quarter), u16 tag; payload <= 1500 bytes        (header 20 bytes)
  BEACON  0x02  u16 len, u8 type, u8 index (0..3), u32 period_ms (5..10000; 0 = stop),
                u8 cqi (0 = CQI 1), u8 subchannel, u16 reserved; payload           (header 12 bytes)
read by the program
  RX      0x81  u16 len, u8 type, u8 flags, u32 src, u32 dst, u16 seq, u8 cqi (what this frame's
                SINR supports: report it back), u8 cqi_sent, i32 rssi_dbm_x10, i32 sinr_db_x10,
                u64 timestamp_us (end of the frame on the airtime clock, propagation delay included),
                u64 slot (the slot it started in), u8 subchannel, u8 cqi_report (the sender's report of
                how it hears this radio, 0 = none), u16 delay_us; payload             (header 44 bytes)
  STATUS  0x83  u16 len, u8 type, u8 result (0 sent, 1 acked, 2 failed, 3 rejected), u32 dst,
                u16 tag, u8 attempts, u8 cqi, u64 slot, u32 airtime_us                (24 bytes)
flags: 0x01 HARQ (ACK requested), 0x02 ACK, 0x04 beacon, 0x08 retransmission
```

Every TX record gets exactly one STATUS record (`sent` at once, or with HARQ `acked` / `failed`
after the exchange); `rejected` means framed mode off, a payload over 1500 bytes, a bad
sub-channel, a slot already past or more than 1 s ahead, or a transmit blocked by server rules.
Beacons send no status. On the air a frame is the payload behind a 16-byte radio header
(magic `0xF7`, version, flags, cqi_report, src, dst, seq).

### What the radio models, and what it simplifies

- **Medium.** Frames are `Emission.frame`s on the shared medium with modulation `CELL-CQI<n>`, so
  path loss, walls and terrain, fading, interference and the packet-error roll are the medium's,
  as for every other radio. They go out through the SDR's transmit chain: amplifier, coax loss,
  antenna pattern, height and mismatch all apply. Antennas: the LTE Sector Antenna and the mmWave
  Array Antenna now take coax on their bottom jack (a fixed 14 dBi sector / 20 dBi array pattern
  pointing where the panel faces, matched only in its band); wire antennas built from blocks are
  modelled to 30 MHz (fine wire 500 MHz) and make poor UHF antennas, as the analyzer shows.
- **Slots.** A transmission starts on a slot boundary of the airtime clock: the next one, or the
  slot the TX record names (up to 1 s ahead).
- **Link adaptation.** Each received frame is measured (RSSI, SINR, the CQI it supports, with
  the v1 3 dB margin). Every frame header and ACK carries the sender's measured CQI of the peer
  it is addressed to; unicast frames go at the CQI the destination last reported, broadcasts,
  beacons and unknown peers at CQI 1; a TX record can force a CQI. Airtime = one OFDM symbol +
  bits / (efficiency x bandwidth x (1 - overhead)).
- **HARQ.** With the HARQ flag the receiving radio sends an ACK frame 4 slots after the frame's
  start slot (or at the first slot after it notices the frame, if later); without an ACK 8 slots
  after a try the sender retransmits two CQI steps lower, at most 3 times, then reports `failed`.
  ACKs are real frames on the air and can be lost. Duplicates of a delivered frame (lost ACK) are
  acknowledged again but delivered once. Simplified: stop-and-wait per frame (no HARQ processes or
  soft combining), and timers run when the radio is pumped: every server tick and on every
  device-file access, so an ACK round trip takes up to two ticks.
- **Timing advance.** A UE measures the propagation delay of base-station frames and starts its
  own transmissions that much early, so they arrive on the slot boundary (`ta_us`). At block
  distances this is 0-1 µs; the UE's slot clock is otherwise the true airtime clock (no clock
  offset is modelled).
- **Beacons.** Up to 4 per radio, repeated every N ms on boundaries of the airtime clock, put on
  the air up to 150 ms ahead so they keep exact spacing even when the server lags.
- **Collisions.** A received frame is held until it has ended on the airtime clock. Emissions that
  overlapped it but were not on the air when the medium delivered it (another transmitter keying
  later in the same slot) are then added to its interference and the error roll is taken again,
    so two transmitters in one slot both lose, not just the second. A radio's own emissions never
  collide with each other (since Cellular v2 phase 4, in the medium too): its PHY multiplexes the
  data, ACKs and beacons it puts on the air at once, as LTE does in a subframe. Interference only counts inside
  the frame's own band (the medium now uses the frame-channel / receiver-channel intersection), so
  UEs on different uplink sub-channels don't collide.
- **Not modelled.** Resource-block scheduling inside a slot (one frame per transmitter per time,
  sub-channels are the only frequency division), MIMO, power control loops, encryption (a protocol
  matter for the programs above), frequency/clock offsets, half-duplex UEs.

### Modem modules as framed radios

`FramedRadio` (pure Java: `radio/framed/FramedRadio`, records `FramedRecords`, device file
`FramedDeviceFd`, peripheral methods `FramedPeripheral`) is hosted by the SDR block entity and by
`radio/cellular/FramedModemModule`. LTE / mmWave Modem modules are bare framed radios (the only modem since Cellular v2 phase 4): peripheral
type `framed_radio`, `/dev/frames.<bay>` and `/dev/framesctl.<bay>`, profile fixed to the modem's
technology, role UE by default, 23 dBm, the v1 device antenna at the host's pose. A modem in a Terminal has its own SIM slot (the card kept on the server, `ModemSimStore`; companion peripheral
`<bay>_sim`, type `sim`); `celld`
drives the module ([CELLULAR_V2.md](../gui/CELLULAR_V2.md#as-built-phase-3-phone-side)).

### Software

- Rust: `ecm_radio::framed` (records, `Framed` device wrapper with `configure`, `send`, `beacon`,
  `recv(timeout)` using `poll_oneoff`, the `framed` program's logic).
- Python: `radio.framed(name)` -> `Framed` with `configure(phy=, role=, channel=, bandwidth=, power=,
  address=, promiscuous=)`, `send(payload, dst=, ack=, slot=, cqi=, subchannel=)` (returns the status
  dict), `beacon(payload, period_ms=, index=)`, `recv(timeout=, count=)` (rx dicts), `status()`,
  `iq_mode()`; native helpers `_radio.framed_tx/framed_beacon/framed_parse/resolve_framed`.
- Program `framed`:

```
framed [--sdr NAME] status
framed [--sdr NAME] config [--phy lte|mmwave] [--role bs|ue] [--channel N] [--bw HZ] [--power DBM] [--addr HEX] [--promisc on|off] [--iq]
framed [--sdr NAME] send <dst|bcast> <text...> [--ack] [--count N] [--interval MS] [--slot N|+N] [--cqi N] [--sub N]
framed [--sdr NAME] recv [--count N] [--seconds S] [--quiet]
framed [--sdr NAME] beacon <period_ms|off> [text...] [--index I]
```

Two computers with Advanced SDRs (scenario `sdr_framed`):

```
A> framed config --phy lte --role bs --channel 3 --power 0 --addr 1
B> framed config --phy lte --role ue --channel 3 --power 0 --addr 2
B> framed recv --count 3                      A> framed send bcast hello from A --count 3
rx 00000001 -> bcast seq 1 rssi -27.6 dBm sinr 71.3 dB cqi 15 (sent 1) slot 1234567 t 1234.568 s "hello from A"
A> framed recv --count 1                      B> framed send 1 ack me --ack
tx 00000001: acked (attempts 1, cqi 1, slot ..., ... us)
A> framed beacon 100 cell A                   B> framed recv --seconds 3 --quiet
framed: 30 frames (30 beacons) in 3.0 s, mean rssi -27.6 dBm sinr 71.3 dB, beacon every 100.0 ms
```

### Tests

- JUnit `FramedRadioTest` (26.1) on `BasicRadioMedium`: delivery in range only with RSSI / SINR /
  CQI, wrong channel hears nothing; unicast at the peer's reported CQI; scheduled slots (LTE 1 ms,
  mmWave 125 µs), late slots rejected; beacon offload exactly every 10 ms; LTE through a block of
  stone, mmWave stopped by it; two transmitters in one slot both lost (and different sub-channels
  both heard); HARQ acked / failed after 4 attempts; retune flushes; device file and control file.
- `cargo test -p ecm-radio framed` (records, names, config lines, program arguments, the wrapper
  over plain files); `cargo test -p python framed` (`radio.framed()` over plain files).
- GameTest `ecm_radio` `sdr_framed` (above; control: B on channel 4 hears nothing).

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
