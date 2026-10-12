//! Framed PHY mode: whole LTE / mmWave frames with measurements, through a
//! framed radio's device files (the Advanced SDR in framed mode, or a modem
//! module in framed-radio form). See docs/radio/SDR_PROGRAMS.md "Framed PHY mode".
//!
//! - `/dev/sdr<N>.frames` (`/dev/sdr.frames` = the first SDR) or
//!   `/dev/frames.<attachment>`: binary records, little-endian, each starting
//!   with `u16 len` (whole record) and `u8 type`. Write [`encode_tx`] /
//!   [`encode_beacon`] records; read [`Record::Rx`] / [`Record::Status`] ones
//!   (a read returns whole records; a blocking read gives up after 2 s with 0
//!   bytes). Pollable with `poll_oneoff` / `sock_poll` next to sockets.
//! - `/dev/sdrctl<N>` or `/dev/framesctl.<attachment>`: text lines
//!   `mode framed|iq`, `phy lte|mmwave`, `role bs|ue`, `channel N`,
//!   `phy_bw HZ`, `power DBM`, `address HEX`, `promisc 0|1`; reading gives
//!   `key value` status lines (`framed`, `phy`, `role`, `channel`, `phy_bw`,
//!   `tx_freq`, `rx_freq`, `power_dbm`, `address`, `slot_us`, `slot`, `ta_us`,
//!   counters).

use crate::device::Status;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

pub const BROADCAST: u32 = 0xFFFF_FFFF;
pub const MAX_PAYLOAD: usize = 1500;
pub const TX: u8 = 0x01;
pub const BEACON: u8 = 0x02;
pub const RX: u8 = 0x81;
pub const STATUS: u8 = 0x83;
pub const TX_HEADER: usize = 20;
pub const BEACON_HEADER: usize = 12;
pub const RX_HEADER: usize = 44;
pub const STATUS_LEN: usize = 24;
/// Largest record a read returns (an RX record with a full payload).
pub const MAX_RECORD: usize = RX_HEADER + MAX_PAYLOAD;

/// Frame flags (TX records and received frames).
pub const FLAG_HARQ: u8 = 0x01;
pub const FLAG_ACK: u8 = 0x02;
pub const FLAG_BEACON: u8 = 0x04;
pub const FLAG_RETX: u8 = 0x08;

/// STATUS results.
pub const SENT: u8 = 0;
pub const ACKED: u8 = 1;
pub const FAILED: u8 = 2;
pub const REJECTED: u8 = 3;

/// A frame as the radio received it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rx {
    pub flags: u8,
    pub src: u32,
    pub dst: u32,
    pub seq: u16,
    /// CQI this frame's SINR supports (what to report back to the sender).
    pub cqi: u8,
    /// CQI the frame was sent at.
    pub cqi_sent: u8,
    pub rssi_dbm: f64,
    pub sinr_db: f64,
    /// End of the frame on the airtime clock, µs (propagation delay included).
    pub timestamp_us: u64,
    /// Slot the frame started in.
    pub slot: u64,
    /// 0 = whole channel, 1..4 = the quarter it was sent on.
    pub subchannel: u8,
    /// The sender's report of how it hears us (0 = none).
    pub cqi_report: u8,
    pub delay_us: u16,
    pub payload: Vec<u8>,
}

impl Rx {
    pub fn is_beacon(&self) -> bool {
        self.flags & FLAG_BEACON != 0
    }
}

/// The outcome of one TX record.
#[derive(Clone, Debug, PartialEq)]
pub struct TxStatus {
    pub result: u8,
    pub dst: u32,
    pub tag: u16,
    pub attempts: u8,
    pub cqi: u8,
    pub slot: u64,
    pub airtime_us: u32,
}

impl TxStatus {
    pub fn result_name(&self) -> &'static str {
        match self.result {
            SENT => "sent",
            ACKED => "acked",
            FAILED => "failed",
            REJECTED => "rejected",
            _ => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Rx(Rx),
    Status(TxStatus),
}

/// Options of one transmitted frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TxOpts {
    /// Ask the receiving radio for a HARQ ACK; the radio retransmits without one.
    pub harq: bool,
    /// Absolute slot to start in; 0 = the next slot.
    pub slot: u64,
    /// 0 = automatic (the peer's reported CQI, CQI 1 for broadcasts).
    pub cqi: u8,
    /// 0 = the whole channel, 1..4 = a quarter of it.
    pub subchannel: u8,
    /// Echoed in the STATUS record.
    pub tag: u16,
}

pub fn encode_tx(dst: u32, payload: &[u8], o: &TxOpts) -> Vec<u8> {
    let len = TX_HEADER + payload.len();
    let mut b = Vec::with_capacity(len);
    b.extend_from_slice(&(len as u16).to_le_bytes());
    b.push(TX);
    b.push(if o.harq { FLAG_HARQ } else { 0 });
    b.extend_from_slice(&dst.to_le_bytes());
    b.extend_from_slice(&o.slot.to_le_bytes());
    b.push(o.cqi);
    b.push(o.subchannel);
    b.extend_from_slice(&o.tag.to_le_bytes());
    b.extend_from_slice(payload);
    b
}

/// A beacon repeated every `period_ms` (0 stops beacon `index`).
pub fn encode_beacon(index: u8, period_ms: u32, cqi: u8, subchannel: u8, payload: &[u8]) -> Vec<u8> {
    let len = BEACON_HEADER + payload.len();
    let mut b = Vec::with_capacity(len);
    b.extend_from_slice(&(len as u16).to_le_bytes());
    b.push(BEACON);
    b.push(index);
    b.extend_from_slice(&period_ms.to_le_bytes());
    b.push(cqi);
    b.push(subchannel);
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(payload);
    b
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

/// Parse the records in one read. Malformed or unknown records end the parse (they are skipped).
pub fn parse_records(b: &[u8]) -> Vec<Record> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 3 <= b.len() {
        let n = u16_at(b, p) as usize;
        if n < 3 || p + n > b.len() {
            break;
        }
        let r = &b[p..p + n];
        match r[2] {
            RX if n >= RX_HEADER => out.push(Record::Rx(Rx {
                flags: r[3],
                src: u32_at(r, 4),
                dst: u32_at(r, 8),
                seq: u16_at(r, 12),
                cqi: r[14],
                cqi_sent: r[15],
                rssi_dbm: u32_at(r, 16) as i32 as f64 / 10.0,
                sinr_db: u32_at(r, 20) as i32 as f64 / 10.0,
                timestamp_us: u64_at(r, 24),
                slot: u64_at(r, 32),
                subchannel: r[40],
                cqi_report: r[41],
                delay_us: u16_at(r, 42),
                payload: r[RX_HEADER..].to_vec(),
            })),
            STATUS if n >= STATUS_LEN => out.push(Record::Status(TxStatus {
                result: r[3],
                dst: u32_at(r, 4),
                tag: u16_at(r, 8),
                attempts: r[10],
                cqi: r[11],
                slot: u64_at(r, 12),
                airtime_us: u32_at(r, 20),
            })),
            _ => {}
        }
        p += n;
    }
    out
}

/// Device file paths of one framed radio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramedPaths {
    pub frames: String,
    pub ctl: String,
}

/// `""` -> the first SDR; `0`, `sdr0`, `sdr_0` -> SDR 0; a `/dev/...` path ->
/// that file; anything else -> the attachment (`/dev/frames.<name>`, an SDR
/// or a modem module in framed-radio form).
pub fn resolve(name: &str) -> FramedPaths {
    let n = name.trim();
    if n.is_empty() {
        return FramedPaths { frames: "/dev/sdr.frames".into(), ctl: "/dev/sdrctl".into() };
    }
    if let Some(rest) = n.strip_prefix("/dev/frames.") {
        return FramedPaths { frames: n.into(), ctl: format!("/dev/framesctl.{rest}") };
    }
    if let Some(rest) = n.strip_prefix("/dev/sdr") {
        let rest = rest.strip_suffix(".frames").unwrap_or(rest);
        return FramedPaths { frames: format!("/dev/sdr{rest}.frames"), ctl: format!("/dev/sdrctl{rest}") };
    }
    let idx = n.strip_prefix("sdr_").or_else(|| n.strip_prefix("sdr")).unwrap_or(n);
    if !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()) {
        return FramedPaths { frames: format!("/dev/sdr{idx}.frames"), ctl: format!("/dev/sdrctl{idx}") };
    }
    FramedPaths { frames: format!("/dev/frames.{n}"), ctl: format!("/dev/framesctl.{n}") }
}

/// Settings for [`Framed::configure`]; `None` leaves a setting as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub phy: Option<String>,
    pub role: Option<String>,
    pub channel: Option<u32>,
    pub bandwidth_hz: Option<f64>,
    pub power_dbm: Option<f64>,
    pub address: Option<u32>,
    pub promiscuous: Option<bool>,
}

impl Config {
    /// The control lines, in the order the radio needs them (the profile first: it resets the channel).
    pub fn lines(&self) -> Vec<String> {
        let mut v = vec!["mode framed".to_string()];
        if let Some(p) = &self.phy {
            v.push(format!("phy {p}"));
        }
        if let Some(r) = &self.role {
            v.push(format!("role {r}"));
        }
        if let Some(c) = self.channel {
            v.push(format!("channel {c}"));
        }
        if let Some(b) = self.bandwidth_hz {
            v.push(format!("phy_bw {}", b.round() as i64));
        }
        if let Some(p) = self.power_dbm {
            v.push(format!("power {p}"));
        }
        if let Some(a) = self.address {
            v.push(format!("address {a:08x}"));
        }
        if let Some(p) = self.promiscuous {
            v.push(format!("promisc {}", p as u8));
        }
        v
    }
}

/// An open framed radio.
pub struct Framed {
    pub paths: FramedPaths,
    rx_path: String,
    tx_path: String,
    rx: Option<File>,
    tx: Option<File>,
    buf: Vec<u8>,
    pending: std::collections::VecDeque<Record>,
}

impl Framed {
    /// Open the framed radio called `name` (see [`resolve`]). Fails if its
    /// control file can't be read (nothing there).
    pub fn open(name: &str) -> io::Result<Framed> {
        let paths = resolve(name);
        let f = paths.frames.clone();
        Framed::with_files(paths, &f, &f)
    }

    /// Separate receive and transmit files (host tests use plain files).
    pub fn with_files(paths: FramedPaths, rx_path: &str, tx_path: &str) -> io::Result<Framed> {
        let mut f = Framed {
            paths,
            rx_path: rx_path.into(),
            tx_path: tx_path.into(),
            rx: None,
            tx: None,
            buf: vec![0; 64 * 1024],
            pending: Default::default(),
        };
        f.status()?;
        Ok(f)
    }

    pub fn control(&mut self, cmd: &str) -> io::Result<()> {
        let mut f = OpenOptions::new().write(true).open(&self.paths.ctl)?;
        f.write_all(format!("{}\n", cmd.trim()).as_bytes())
    }

    pub fn status(&mut self) -> io::Result<Status> {
        let mut s = String::new();
        File::open(&self.paths.ctl)?.read_to_string(&mut s)?;
        Ok(Status::parse(&s))
    }

    /// Switch to framed mode and apply `c`.
    pub fn configure(&mut self, c: &Config) -> io::Result<()> {
        for line in c.lines() {
            self.control(&line)?;
        }
        Ok(())
    }

    /// Back to IQ mode (an SDR).
    pub fn iq_mode(&mut self) -> io::Result<()> {
        self.control("mode iq")
    }

    /// This radio's address.
    pub fn address(&mut self) -> io::Result<u32> {
        let s = self.status()?;
        s.get("address")
            .and_then(|a| u32::from_str_radix(a, 16).ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no address in the status"))
    }

    /// The current slot on the airtime clock.
    pub fn slot(&mut self) -> io::Result<u64> {
        Ok(self.status()?.num("slot").unwrap_or(0.0) as u64)
    }

    fn write_record(&mut self, rec: &[u8]) -> io::Result<()> {
        if self.tx.is_none() {
            self.tx = Some(OpenOptions::new().write(true).open(&self.tx_path)?);
        }
        self.tx.as_mut().unwrap().write_all(rec)
    }

    /// Send one frame.
    pub fn send(&mut self, dst: u32, payload: &[u8], o: &TxOpts) -> io::Result<()> {
        if payload.len() > MAX_PAYLOAD {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("payload over {MAX_PAYLOAD} bytes")));
        }
        self.write_record(&encode_tx(dst, payload, o))
    }

    /// Start (or replace) beacon `index`, every `period_ms`; 0 stops it.
    pub fn beacon(&mut self, index: u8, period_ms: u32, payload: &[u8]) -> io::Result<()> {
        self.write_record(&encode_beacon(index, period_ms, 0, 0, payload))
    }

    /// The next record, waiting up to `timeout` (None = nothing came).
    pub fn recv(&mut self, timeout: Duration) -> io::Result<Option<Record>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(r) = self.pending.pop_front() {
                return Ok(Some(r));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if self.rx.is_none() {
                self.rx = Some(File::open(&self.rx_path)?);
            }
            if !poll_readable(self.rx.as_ref().unwrap(), left)? {
                return Ok(None);
            }
            let n = self.rx.as_mut().unwrap().read(&mut self.buf)?;
            if n == 0 {
                if Instant::now() >= deadline || !cfg!(target_os = "wasi") {
                    return Ok(None);   // host files: end of file
                }
                continue;
            }
            self.pending.extend(parse_records(&self.buf[..n]));
        }
    }

    /// The device's raw fd for a caller's own `poll` (wasm only).
    #[cfg(target_os = "wasi")]
    pub fn rx_fd(&mut self) -> io::Result<i32> {
        use std::os::fd::AsRawFd;
        if self.rx.is_none() {
            self.rx = Some(File::open(&self.rx_path)?);
        }
        Ok(self.rx.as_ref().unwrap().as_raw_fd())
    }
}

/// Wait until `f` has something to read (WASI `poll_oneoff` with an fd_read and a
/// clock subscription). On other targets files are always readable.
#[cfg(target_os = "wasi")]
pub fn poll_readable(f: &File, timeout: Duration) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    #[link(wasm_import_module = "wasi_snapshot_preview1")]
    extern "C" {
        fn poll_oneoff(subs: *const u8, events: *mut u8, nsubs: usize, nevents: *mut usize) -> u16;
    }
    let mut subs = [0u8; 96];
    // [0] fd_read on the device
    subs[0..8].copy_from_slice(&1u64.to_le_bytes());
    subs[8] = 1;
    subs[16..20].copy_from_slice(&(f.as_raw_fd() as u32).to_le_bytes());
    // [1] relative monotonic clock
    subs[48..56].copy_from_slice(&2u64.to_le_bytes());
    subs[56] = 0;
    subs[64..68].copy_from_slice(&1u32.to_le_bytes());
    subs[72..80].copy_from_slice(&(timeout.as_nanos().min(u64::MAX as u128) as u64).to_le_bytes());
    let mut events = [0u8; 64];
    let mut n: usize = 0;
    let rc = unsafe { poll_oneoff(subs.as_ptr(), events.as_mut_ptr(), 2, &mut n) };
    if rc != 0 {
        return Err(io::Error::new(io::ErrorKind::Other, format!("poll_oneoff errno {rc}")));
    }
    Ok((0..n).any(|i| events[i * 32..i * 32 + 8] == 1u64.to_le_bytes()))
}

#[cfg(not(target_os = "wasi"))]
pub fn poll_readable(_f: &File, _timeout: Duration) -> io::Result<bool> {
    Ok(true)
}

/// Format an address the way the radio does (`0000002a`, `ffffffff` = broadcast).
pub fn fmt_addr(a: u32) -> String {
    if a == BROADCAST {
        "bcast".into()
    } else {
        format!("{a:08x}")
    }
}

/// Parse `bcast` / `broadcast` / hex.
pub fn parse_addr(s: &str) -> Result<u32, String> {
    let t = s.trim().to_ascii_lowercase();
    if t == "bcast" || t == "broadcast" || t == "ffffffff" {
        return Ok(BROADCAST);
    }
    let t = t.strip_prefix("0x").unwrap_or(&t);
    u32::from_str_radix(t, 16).map_err(|_| format!("bad address {s:?} (hex, or bcast)"))
}

/// A payload for printing: the text if it is printable UTF-8, else hex.
pub fn show_payload(p: &[u8]) -> String {
    match std::str::from_utf8(p) {
        Ok(s) if s.chars().all(|c| !c.is_control() || c == ' ') => format!("{s:?}"),
        _ => p.iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// One line per received frame, as the `framed` program prints it.
pub fn describe_rx(r: &Rx) -> String {
    let mut kind = String::new();
    if r.is_beacon() {
        kind.push_str(" beacon");
    }
    if r.flags & FLAG_RETX != 0 {
        kind.push_str(" retx");
    }
    format!(
        "rx {} -> {} seq {} rssi {:.1} dBm sinr {:.1} dB cqi {} (sent {}) slot {} t {:.6} s{} {}",
        fmt_addr(r.src),
        fmt_addr(r.dst),
        r.seq,
        r.rssi_dbm,
        r.sinr_db,
        r.cqi,
        r.cqi_sent,
        r.slot,
        r.timestamp_us as f64 / 1e6,
        kind,
        show_payload(&r.payload)
    )
}

// ---------------------------------------------------------------- the `framed` program

pub const USAGE: &str = "[--sdr NAME] <command>
  status                                   settings and counters
  config [--phy lte|mmwave] [--role bs|ue] [--channel N] [--bw HZ] [--power DBM] [--addr HEX] [--promisc on|off]
                                           switch to framed mode and set it up (--iq: back to IQ mode)
  send <dst|bcast> <text...> [--ack] [--count N] [--interval MS] [--slot N|+N] [--cqi N] [--sub N]
  recv [--count N] [--seconds S] [--quiet]  print frames with RSSI, SINR, CQI, slot, time
  beacon <period_ms|off> [text...] [--index I]   repeat a frame without sending each one
NAME: '' (first SDR), sdr_N, or an attachment (an Advanced SDR, or a modem module in framed-radio form)";

/// A parsed `framed` command line.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Status,
    Config { config: Config, iq: bool },
    Send { dst: u32, text: String, opts: TxOpts, relative_slot: bool, count: u32, interval_ms: u64 },
    Recv { count: Option<u32>, seconds: f64, quiet: bool },
    Beacon { index: u8, period_ms: u32, text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub radio: String,
    pub cmd: Cmd,
}

fn num<T: std::str::FromStr>(o: &crate::cli::Opts, k: &str) -> Result<Option<T>, String> {
    o.str(k).map(|v| v.parse::<T>().map_err(|_| format!("--{k}: bad value {v:?}"))).transpose()
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let o = crate::cli::split(
        args,
        &["sdr", "phy", "role", "channel", "bw", "power", "addr", "promisc", "count", "interval", "slot", "cqi", "sub", "seconds", "index"],
        &["ack", "quiet", "iq", "help"],
    )?;
    let radio = o.str("sdr").unwrap_or_default();
    let cmd = o.pos.first().map(String::as_str).ok_or("missing command")?;
    let rest = &o.pos[1..];
    let cmd = match cmd {
        "status" => Cmd::Status,
        "config" => {
            let promiscuous = match o.str("promisc").as_deref() {
                None => None,
                Some("on" | "1" | "true") => Some(true),
                Some("off" | "0" | "false") => Some(false),
                Some(v) => return Err(format!("--promisc: on or off, got {v:?}")),
            };
            let bandwidth_hz = match o.str("bw") {
                Some(v) => Some(crate::units::parse_freq(&v).ok_or(format!("--bw: bad bandwidth {v:?}"))?),
                None => None,
            };
            Cmd::Config {
                config: Config {
                    phy: o.str("phy"),
                    role: o.str("role"),
                    channel: num(&o, "channel")?,
                    bandwidth_hz,
                    power_dbm: o.num("power")?,
                    address: o.str("addr").map(|a| parse_addr(&a)).transpose()?,
                    promiscuous,
                },
                iq: o.flag("iq"),
            }
        }
        "send" => {
            let dst = parse_addr(rest.first().ok_or("send: missing destination (hex address or bcast)")?)?;
            let text = rest[1..].join(" ");
            if text.is_empty() {
                return Err("send: missing text".into());
            }
            let (slot, relative_slot) = match o.str("slot") {
                None => (0, false),
                Some(v) => match v.strip_prefix('+') {
                    Some(r) => (r.parse().map_err(|_| format!("--slot: bad value {v:?}"))?, true),
                    None => (v.parse().map_err(|_| format!("--slot: bad value {v:?}"))?, false),
                },
            };
            Cmd::Send {
                dst,
                text,
                opts: TxOpts {
                    harq: o.flag("ack"),
                    slot,
                    cqi: num(&o, "cqi")?.unwrap_or(0),
                    subchannel: num(&o, "sub")?.unwrap_or(0),
                    tag: 0,
                },
                relative_slot,
                count: num(&o, "count")?.unwrap_or(1).max(1),
                interval_ms: num(&o, "interval")?.unwrap_or(100),
            }
        }
        "recv" => Cmd::Recv { count: num(&o, "count")?, seconds: o.num("seconds")?.unwrap_or(10.0), quiet: o.flag("quiet") },
        "beacon" => {
            let p = rest.first().ok_or("beacon: missing period (ms) or off")?;
            let period_ms = if p == "off" { 0 } else { p.parse().map_err(|_| format!("beacon: bad period {p:?}"))? };
            let text = rest[1..].join(" ");
            if period_ms > 0 && text.is_empty() {
                return Err("beacon: missing text".into());
            }
            Cmd::Beacon { index: num(&o, "index")?.unwrap_or(0), period_ms, text }
        }
        other => return Err(format!("unknown command {other:?}")),
    };
    Ok(Args { radio, cmd })
}

pub fn main() {
    let a = parse_args(&crate::run::args()).unwrap_or_else(|e| crate::run::fail("framed", USAGE, e));
    let mut f = match Framed::open(&a.radio) {
        Ok(f) => f,
        Err(e) => {
            let shown = if a.radio.is_empty() { "the first SDR".to_string() } else { format!("{:?}", a.radio) };
            eprintln!("framed: can't open {shown} ({e}); place an Advanced SDR next to the computer");
            std::process::exit(1);
        }
    };
    if let Err(e) = run(&mut f, &a.cmd) {
        eprintln!("framed: {e}");
        std::process::exit(1);
    }
}

/// Run one command on an open radio (printing like the program).
pub fn run(f: &mut Framed, cmd: &Cmd) -> Result<(), String> {
    let io = |e: io::Error| e.to_string();
    match cmd {
        Cmd::Status => {
            let s = f.status().map_err(io)?;
            for (k, v) in &s.0 {
                println!("{k} {v}");
            }
        }
        Cmd::Config { config, iq } => {
            if *iq {
                f.iq_mode().map_err(io)?;
                println!("framed: IQ mode");
                return Ok(());
            }
            f.configure(config).map_err(io)?;
            let s = f.status().map_err(io)?;
            println!(
                "framed: {} {} channel {} tx {:.1} MHz rx {:.1} MHz bw {} MHz power {} dBm address {}",
                s.get("phy").unwrap_or("?"),
                s.get("role").unwrap_or("?"),
                s.get("channel").unwrap_or("?"),
                s.num("tx_freq").unwrap_or(0.0) / 1e6,
                s.num("rx_freq").unwrap_or(0.0) / 1e6,
                s.num("phy_bw").unwrap_or(0.0) / 1e6,
                s.get("power_dbm").unwrap_or("?"),
                s.get("address").unwrap_or("?")
            );
        }
        Cmd::Send { dst, text, opts, relative_slot, count, interval_ms } => {
            let mut sent = 0;
            let (mut acked, mut failed) = (0, 0);
            for i in 0..*count {
                let mut o = opts.clone();
                o.tag = i as u16;
                if *relative_slot {
                    o.slot += f.slot().map_err(io)?;
                }
                f.send(*dst, text.as_bytes(), &o).map_err(io)?;
                sent += 1;
                // Each frame gets one STATUS record; with --ack it comes after the HARQ exchange.
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        println!("tx {}: no status", fmt_addr(*dst));
                        break;
                    }
                    if let Some(Record::Status(s)) = f.recv(left).map_err(io)? {
                        if s.tag != o.tag {
                            continue;
                        }
                        match s.result {
                            ACKED => acked += 1,
                            FAILED | REJECTED => failed += 1,
                            _ => {}
                        }
                        println!(
                            "tx {}: {} (attempts {}, cqi {}, slot {}, {} us)",
                            fmt_addr(s.dst),
                            s.result_name(),
                            s.attempts,
                            s.cqi,
                            s.slot,
                            s.airtime_us
                        );
                        break;
                    }
                }
                if i + 1 < *count {
                    std::thread::sleep(Duration::from_millis(*interval_ms));
                }
            }
            if opts.harq {
                println!("framed: sent {sent} frame(s), {acked} acked, {failed} failed");
            } else {
                println!("framed: sent {sent} frame(s)");
            }
        }
        Cmd::Recv { count, seconds, quiet } => {
            let start = Instant::now();
            let deadline = start + Duration::from_secs_f64(seconds.max(0.0));
            let (mut frames, mut beacons) = (0u32, 0u32);
            let mut beacon_times: Vec<u64> = Vec::new();
            let (mut sum_rssi, mut sum_sinr) = (0.0, 0.0);
            while count.map_or(true, |c| frames < c) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                if let Some(Record::Rx(r)) = f.recv(left).map_err(io)? {
                    frames += 1;
                    sum_rssi += r.rssi_dbm;
                    sum_sinr += r.sinr_db;
                    if r.is_beacon() {
                        beacons += 1;
                        beacon_times.push(r.timestamp_us);
                    }
                    if !quiet {
                        println!("{}", describe_rx(&r));
                    }
                }
            }
            println!("{}", recv_summary(frames, beacons, start.elapsed().as_secs_f64(), sum_rssi, sum_sinr, &beacon_times));
        }
        Cmd::Beacon { index, period_ms, text } => {
            f.beacon(*index, *period_ms, text.as_bytes()).map_err(io)?;
            if *period_ms == 0 {
                println!("framed: beacon {index} off");
            } else {
                println!("framed: beacon {index} every {period_ms} ms: {}", show_payload(text.as_bytes()));
            }
        }
    }
    Ok(())
}

/// The last line of `framed recv`.
pub fn recv_summary(frames: u32, beacons: u32, secs: f64, sum_rssi: f64, sum_sinr: f64, beacon_times: &[u64]) -> String {
    let mut line = format!("framed: {frames} frames ({beacons} beacons) in {secs:.1} s");
    if frames > 0 {
        line += &format!(", mean rssi {:.1} dBm sinr {:.1} dB", sum_rssi / frames as f64, sum_sinr / frames as f64);
    }
    if beacon_times.len() >= 2 {
        let span = beacon_times[beacon_times.len() - 1] - beacon_times[0];
        line += &format!(", beacon every {:.1} ms", span as f64 / (beacon_times.len() - 1) as f64 / 1000.0);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx_record(src: u32, payload: &[u8], flags: u8, ts: u64) -> Vec<u8> {
        let mut b = vec![0u8; RX_HEADER];
        let n = (RX_HEADER + payload.len()) as u16;
        b[0..2].copy_from_slice(&n.to_le_bytes());
        b[2] = RX;
        b[3] = flags;
        b[4..8].copy_from_slice(&src.to_le_bytes());
        b[8..12].copy_from_slice(&BROADCAST.to_le_bytes());
        b[12..14].copy_from_slice(&7u16.to_le_bytes());
        b[14] = 15;
        b[15] = 1;
        b[16..20].copy_from_slice(&(-612i32).to_le_bytes());
        b[20..24].copy_from_slice(&384i32.to_le_bytes());
        b[24..32].copy_from_slice(&ts.to_le_bytes());
        b[32..40].copy_from_slice(&(ts / 1000).to_le_bytes());
        b[40] = 2;
        b[41] = 9;
        b[42..44].copy_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn records_encode_and_parse() {
        let tx = encode_tx(0x2a, b"hi", &TxOpts { harq: true, slot: 99, cqi: 7, subchannel: 3, tag: 0x1234 });
        assert_eq!(tx.len(), TX_HEADER + 2);
        assert_eq!(u16_at(&tx, 0) as usize, tx.len());
        assert_eq!((tx[2], tx[3]), (TX, FLAG_HARQ));
        assert_eq!(u32_at(&tx, 4), 0x2a);
        assert_eq!(u64_at(&tx, 8), 99);
        assert_eq!((tx[16], tx[17], u16_at(&tx, 18)), (7, 3, 0x1234));
        assert_eq!(&tx[20..], b"hi");
        let b = encode_beacon(2, 100, 0, 1, b"cell");
        assert_eq!((b.len(), b[2], b[3], u32_at(&b, 4), b[9]), (BEACON_HEADER + 4, BEACON, 2, 100, 1));

        let mut both = rx_record(1, b"hello", FLAG_BEACON, 12_345_678);
        let mut st = vec![0u8; STATUS_LEN];
        st[0..2].copy_from_slice(&(STATUS_LEN as u16).to_le_bytes());
        st[2] = STATUS;
        st[3] = ACKED;
        st[4..8].copy_from_slice(&5u32.to_le_bytes());
        st[8..10].copy_from_slice(&3u16.to_le_bytes());
        st[10] = 2;
        st[11] = 4;
        st[12..20].copy_from_slice(&777u64.to_le_bytes());
        st[20..24].copy_from_slice(&1500u32.to_le_bytes());
        both.extend_from_slice(&st);
        both.extend_from_slice(&[9, 0, 1]); // a truncated record ends the parse
        let recs = parse_records(&both);
        assert_eq!(recs.len(), 2);
        let Record::Rx(r) = &recs[0] else { panic!() };
        assert_eq!((r.src, r.seq, r.cqi, r.cqi_sent, r.subchannel, r.cqi_report, r.delay_us), (1, 7, 15, 1, 2, 9, 1));
        assert!((r.rssi_dbm + 61.2).abs() < 1e-9 && (r.sinr_db - 38.4).abs() < 1e-9);
        assert!(r.is_beacon());
        assert_eq!(r.payload, b"hello");
        assert_eq!(
            describe_rx(r),
            "rx 00000001 -> bcast seq 7 rssi -61.2 dBm sinr 38.4 dB cqi 15 (sent 1) slot 12345 t 12.345678 s beacon \"hello\""
        );
        let Record::Status(s) = &recs[1] else { panic!() };
        assert_eq!((s.result_name(), s.dst, s.tag, s.attempts, s.cqi, s.slot, s.airtime_us), ("acked", 5, 3, 2, 4, 777, 1500));
    }

    #[test]
    fn names_resolve_to_framed_device_files() {
        assert_eq!(resolve(""), FramedPaths { frames: "/dev/sdr.frames".into(), ctl: "/dev/sdrctl".into() });
        for n in ["1", "sdr1", "sdr_1", "/dev/sdr1.frames", "/dev/sdr1"] {
            assert_eq!(resolve(n), FramedPaths { frames: "/dev/sdr1.frames".into(), ctl: "/dev/sdrctl1".into() }, "{n}");
        }
        assert_eq!(resolve("left_bay_1"), FramedPaths { frames: "/dev/frames.left_bay_1".into(), ctl: "/dev/framesctl.left_bay_1".into() });
        assert_eq!(resolve("/dev/frames.top").ctl, "/dev/framesctl.top");
    }

    #[test]
    fn config_lines_put_the_profile_first() {
        let c = Config {
            phy: Some("lte".into()),
            role: Some("bs".into()),
            channel: Some(3),
            bandwidth_hz: Some(5e6),
            power_dbm: Some(10.0),
            address: Some(1),
            promiscuous: Some(false),
        };
        assert_eq!(
            c.lines(),
            ["mode framed", "phy lte", "role bs", "channel 3", "phy_bw 5000000", "power 10", "address 00000001", "promisc 0"]
        );
        assert_eq!(Config::default().lines(), ["mode framed"]);
    }

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn program_arguments() {
        let a = parse_args(&argv("--sdr left config --phy lte --role bs --channel 3 --bw 5M --power 10 --addr 1")).unwrap();
        assert_eq!(a.radio, "left");
        let Cmd::Config { config, iq } = a.cmd else { panic!() };
        assert!(!iq);
        assert_eq!(config.lines(), ["mode framed", "phy lte", "role bs", "channel 3", "phy_bw 5000000", "power 10", "address 00000001"]);
        let a = parse_args(&argv("send bcast hello from A --ack --count 3 --slot +10 --cqi 5 --sub 2")).unwrap();
        assert_eq!(
            a.cmd,
            Cmd::Send {
                dst: BROADCAST,
                text: "hello from A".into(),
                opts: TxOpts { harq: true, slot: 10, cqi: 5, subchannel: 2, tag: 0 },
                relative_slot: true,
                count: 3,
                interval_ms: 100
            }
        );
        assert_eq!(parse_args(&argv("recv --count 2")).unwrap().cmd, Cmd::Recv { count: Some(2), seconds: 10.0, quiet: false });
        assert_eq!(
            parse_args(&argv("beacon 100 cell A --index 1")).unwrap().cmd,
            Cmd::Beacon { index: 1, period_ms: 100, text: "cell A".into() }
        );
        assert_eq!(parse_args(&argv("beacon off")).unwrap().cmd, Cmd::Beacon { index: 0, period_ms: 0, text: String::new() });
        assert!(parse_args(&argv("send 1")).is_err());
        assert!(parse_args(&argv("fly")).is_err());
        assert!(parse_args(&argv("config --promisc maybe")).is_err());
    }

    #[test]
    fn recv_summary_reports_the_beacon_period() {
        assert_eq!(recv_summary(0, 0, 3.0, 0.0, 0.0, &[]), "framed: 0 frames (0 beacons) in 3.0 s");
        assert_eq!(
            recv_summary(3, 3, 0.3, -150.0, 90.0, &[100_000, 200_000, 300_000]),
            "framed: 3 frames (3 beacons) in 0.3 s, mean rssi -50.0 dBm sinr 30.0 dB, beacon every 100.0 ms"
        );
    }

    #[test]
    fn addresses_and_payloads() {
        assert_eq!(parse_addr("bcast"), Ok(BROADCAST));
        assert_eq!(parse_addr("0x2A"), Ok(0x2a));
        assert_eq!(parse_addr("1"), Ok(1));
        assert!(parse_addr("zz").is_err());
        assert_eq!(fmt_addr(0x2a), "0000002a");
        assert_eq!(show_payload(b"a b"), "\"a b\"");
        assert_eq!(show_payload(&[0, 255]), "00ff");
    }

    #[test]
    fn framed_device_over_plain_files() {
        let dir = std::env::temp_dir().join(format!("ecm-framed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctl = dir.join("ctl");
        let rx = dir.join("rx");
        let tx = dir.join("tx");
        std::fs::write(&ctl, "framed 1\nphy lte\naddress 0000002a\nslot 1234\n").unwrap();
        let mut recs = rx_record(1, b"one", 0, 1000);
        recs.extend(rx_record(2, b"two", FLAG_BEACON, 2000));
        std::fs::write(&rx, &recs).unwrap();
        std::fs::write(&tx, b"").unwrap();
        let paths = FramedPaths { frames: rx.to_string_lossy().into(), ctl: ctl.to_string_lossy().into() };
        let mut f = Framed::with_files(paths, &rx.to_string_lossy(), &tx.to_string_lossy()).unwrap();
        assert_eq!(f.address().unwrap(), 0x2a);
        assert_eq!(f.slot().unwrap(), 1234);
        f.send(BROADCAST, b"hey", &TxOpts::default()).unwrap();
        f.beacon(0, 100, b"b").unwrap();
        let written = std::fs::read(&tx).unwrap();
        assert_eq!(written.len(), TX_HEADER + 3 + BEACON_HEADER + 1);
        let a = f.recv(Duration::from_millis(10)).unwrap().unwrap();
        let b = f.recv(Duration::from_millis(10)).unwrap().unwrap();
        assert!(matches!(a, Record::Rx(ref r) if r.payload == b"one"));
        assert!(matches!(b, Record::Rx(ref r) if r.is_beacon() && r.src == 2));
        assert_eq!(f.recv(Duration::from_millis(10)).unwrap(), None);
        assert!(f.send(1, &vec![0; MAX_PAYLOAD + 1], &TxOpts::default()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
