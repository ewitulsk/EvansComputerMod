//! The SDR device files.
//!
//! - `/dev/sdr<N>` (or `/dev/sdr.<attachment>`, `/dev/sdr` = the first):
//!   read = receive, write = transmit, interleaved little-endian `cs16`
//!   (default) or `cf32`. Reads block until samples exist on the world clock
//!   (up to 2 s, then 0); writes are scheduled back to back on the world clock
//!   and never block, so transmitters pace themselves ([`TxPacer`]).
//! - `/dev/sdrctl<N>`: write commands one per line (`freq <hz>`, `rate <sps>`,
//!   `bw <hz>`, `gain <db>|agc`, `agc 0|1`, `format cs16|cf32`,
//!   `tx on [dBm]|off`); read `key value` lines (tier, freq, rate, max_rate,
//!   bw, gain, agc, format, tx, tx_power_dbm, adc_bits, timestamp, read,
//!   written, overflows, underflows).

use ecm_dsp::iq::SampleFormat;
use ecm_dsp::C32;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

/// Device file paths for one SDR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdrPaths {
    /// Sample file, e.g. `/dev/sdr0`.
    pub data: String,
    /// Control file, e.g. `/dev/sdrctl0`.
    pub ctl: String,
}

/// Resolve an SDR name: `""` -> the first SDR; `0`, `sdr0`, `sdr_0` -> index 0;
/// `/dev/sdr...` -> that file; anything else -> the attachment name
/// (`/dev/sdr.<name>`, e.g. `left` or `sdr_standard_0`).
pub fn resolve(name: &str) -> SdrPaths {
    let n = name.trim();
    if n.is_empty() {
        return SdrPaths { data: "/dev/sdr".into(), ctl: "/dev/sdrctl".into() };
    }
    if let Some(rest) = n.strip_prefix("/dev/sdrctl") {
        return SdrPaths { data: format!("/dev/sdr{rest}"), ctl: n.to_string() };
    }
    if let Some(rest) = n.strip_prefix("/dev/sdr") {
        return SdrPaths { data: n.to_string(), ctl: format!("/dev/sdrctl{rest}") };
    }
    let idx = n
        .strip_prefix("sdr_")
        .or_else(|| n.strip_prefix("sdr"))
        .unwrap_or(n);
    if !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()) {
        return SdrPaths { data: format!("/dev/sdr{idx}"), ctl: format!("/dev/sdrctl{idx}") };
    }
    SdrPaths { data: format!("/dev/sdr.{n}"), ctl: format!("/dev/sdrctl.{n}") }
}

/// Parsed `/dev/sdrctl` contents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status(pub BTreeMap<String, String>);

impl Status {
    pub fn parse(text: &str) -> Status {
        let mut m = BTreeMap::new();
        for line in text.lines() {
            let mut it = line.splitn(2, char::is_whitespace);
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                if !k.is_empty() {
                    m.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
        Status(m)
    }
    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(String::as_str)
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        self.get(k)?.parse().ok()
    }
    pub fn rate(&self) -> Option<f64> {
        self.num("rate")
    }
    pub fn freq(&self) -> Option<f64> {
        self.num("freq")
    }
    pub fn timestamp(&self) -> Option<i64> {
        self.get("timestamp")?.parse().ok()
    }
    pub fn format(&self) -> SampleFormat {
        match self.get("format") {
            Some("cf32") => SampleFormat::Cf32,
            _ => SampleFormat::Cs16,
        }
    }
}

/// An open SDR.
pub struct Sdr {
    pub paths: SdrPaths,
    pub format: SampleFormat,
    /// Sample rate (from the control file at open, then `set_rate`).
    pub rate: f64,
    rx: Option<File>,
    tx: Option<File>,
    bytes: Vec<u8>,
}

impl Sdr {
    /// Open the SDR called `name` (see [`resolve`]). Fails if its control
    /// file can't be read (no such SDR attached).
    pub fn open(name: &str) -> io::Result<Sdr> {
        Sdr::with_paths(resolve(name))
    }

    pub fn with_paths(paths: SdrPaths) -> io::Result<Sdr> {
        let mut s = Sdr { paths, format: SampleFormat::Cs16, rate: 48_000.0, rx: None, tx: None, bytes: Vec::new() };
        let st = s.status()?;
        s.format = st.format();
        s.rate = st.rate().unwrap_or(48_000.0);
        Ok(s)
    }

    /// Send one control command, e.g. `freq 146520000`.
    pub fn control(&mut self, cmd: &str) -> io::Result<()> {
        let mut f = OpenOptions::new().write(true).open(&self.paths.ctl)?;
        f.write_all(format!("{}\n", cmd.trim()).as_bytes())
    }

    pub fn status(&mut self) -> io::Result<Status> {
        let mut s = String::new();
        File::open(&self.paths.ctl)?.read_to_string(&mut s)?;
        Ok(Status::parse(&s))
    }

    pub fn tune(&mut self, hz: f64) -> io::Result<()> {
        self.control(&format!("freq {}", hz.round() as i64))
    }
    pub fn set_rate(&mut self, sps: u32) -> io::Result<()> {
        self.control(&format!("rate {sps}"))?;
        self.rate = sps as f64;
        Ok(())
    }
    pub fn set_gain(&mut self, db: f64) -> io::Result<()> {
        self.control(&format!("gain {db}"))
    }
    pub fn set_agc(&mut self, on: bool) -> io::Result<()> {
        self.control(if on { "gain agc" } else { "agc 0" })
    }
    pub fn set_bandwidth(&mut self, hz: f64) -> io::Result<()> {
        self.control(&format!("bw {}", hz.round() as i64))
    }
    pub fn set_format(&mut self, f: SampleFormat) -> io::Result<()> {
        self.control(&format!("format {}", f.name()))?;
        self.format = f;
        Ok(())
    }
    /// Transmitter on (at `dbm`, or the tier maximum) or off.
    pub fn set_tx(&mut self, on: bool, dbm: Option<f64>) -> io::Result<()> {
        match (on, dbm) {
            (true, Some(p)) => self.control(&format!("tx on {p}")),
            (true, None) => self.control("tx on"),
            (false, _) => self.control("tx off"),
        }
    }

    /// The device sample clock (absolute sample index).
    pub fn timestamp(&mut self) -> io::Result<i64> {
        self.status()?
            .timestamp()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "sdrctl has no timestamp"))
    }

    /// Receive up to `max` samples (one device read; 0 = nothing within 2 s).
    pub fn read(&mut self, max: usize) -> io::Result<Vec<C32>> {
        if self.rx.is_none() {
            self.rx = Some(File::open(&self.paths.data)?);
        }
        let bps = self.format.bytes_per_sample();
        self.bytes.resize(max.max(1) * bps, 0);
        let n = self.rx.as_mut().unwrap().read(&mut self.bytes)?;
        let mut out = Vec::with_capacity(n / bps);
        self.format.decode(&self.bytes[..n - n % bps], &mut out);
        Ok(out)
    }

    /// Receive at least `min` (at most `max`) samples, sleeping while the
    /// clock catches up instead of polling: a device read returns as soon as
    /// any sample exists, and each read costs a host round trip (and one SDR
    /// AGC step), so tiny reads can fall behind real time. Returns early
    /// (possibly empty) if the device goes quiet.
    pub fn read_at_least(&mut self, min: usize, max: usize) -> io::Result<Vec<C32>> {
        let max = max.max(min).max(1);
        let mut out = self.read(max)?;
        while out.len() < min {
            let missing = (min - out.len()) as f64 / self.rate.max(1.0);
            std::thread::sleep(std::time::Duration::from_secs_f64(missing.min(0.25)));
            let more = self.read(max - out.len())?;
            if more.is_empty() {
                break;
            }
            out.extend(more);
        }
        Ok(out)
    }

    /// Receive exactly `n` samples unless the device goes quiet (`tries`
    /// empty reads in a row).
    pub fn read_n(&mut self, n: usize, tries: usize) -> io::Result<Vec<C32>> {
        let mut out = Vec::with_capacity(n);
        let mut empty = 0;
        while out.len() < n {
            let got = self.read(n - out.len())?;
            if got.is_empty() {
                empty += 1;
                if empty >= tries {
                    break;
                }
            } else {
                empty = 0;
            }
            out.extend(got);
        }
        Ok(out)
    }

    /// Transmit samples (needs `set_tx(true, ..)`).
    pub fn write(&mut self, x: &[C32]) -> io::Result<()> {
        if self.tx.is_none() {
            self.tx = Some(OpenOptions::new().write(true).open(&self.paths.data)?);
        }
        let mut buf = Vec::with_capacity(x.len() * self.format.bytes_per_sample());
        self.format.encode(x, &mut buf);
        let f = self.tx.as_mut().unwrap();
        for c in buf.chunks(4096 - 4096 % self.format.bytes_per_sample()) {
            f.write_all(c)?;
        }
        Ok(())
    }
}

/// Keeps a transmitter a little ahead of the world clock: the SDR schedules
/// written samples back to back without blocking, so a writer that runs
/// ahead would queue minutes of audio.
#[derive(Clone, Debug)]
pub struct TxPacer {
    pub rate: f64,
    /// How far ahead of the clock to stay (samples).
    pub lead: i64,
    start: Option<i64>,
    sent: i64,
}

impl TxPacer {
    pub fn new(rate: f64, lead_seconds: f64) -> TxPacer {
        TxPacer { rate, lead: (rate * lead_seconds) as i64, start: None, sent: 0 }
    }

    /// Record `n` samples written at device time `now`.
    pub fn sent(&mut self, n: usize, now: i64) {
        if self.start.is_none() {
            self.start = Some(now);
        }
        self.sent += n as i64;
    }

    /// Seconds to wait at device time `now` before writing more.
    pub fn wait_seconds(&self, now: i64) -> f64 {
        let Some(start) = self.start else { return 0.0 };
        let ahead = self.sent - (now - start);
        if ahead > self.lead {
            (ahead - self.lead) as f64 / self.rate
        } else {
            0.0
        }
    }

    /// Seconds of written samples still ahead of the clock at `now`.
    pub fn remaining_seconds(&self, now: i64) -> f64 {
        match self.start {
            Some(start) => (self.sent - (now - start)) as f64 / self.rate,
            None => 0.0,
        }
    }

    /// Samples written so far.
    pub fn total(&self) -> i64 {
        self.sent
    }

    /// Restart (after an underflow or a pause).
    pub fn reset(&mut self) {
        self.start = None;
        self.sent = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_to_device_files() {
        assert_eq!(resolve(""), SdrPaths { data: "/dev/sdr".into(), ctl: "/dev/sdrctl".into() });
        for n in ["0", "sdr0", "sdr_0"] {
            assert_eq!(resolve(n), SdrPaths { data: "/dev/sdr0".into(), ctl: "/dev/sdrctl0".into() });
        }
        assert_eq!(resolve("sdr_12").data, "/dev/sdr12");
        assert_eq!(resolve("left"), SdrPaths { data: "/dev/sdr.left".into(), ctl: "/dev/sdrctl.left".into() });
        assert_eq!(resolve("sdr_standard").ctl, "/dev/sdrctl.sdr_standard");
        assert_eq!(resolve("/dev/sdr.top").ctl, "/dev/sdrctl.top");
        assert_eq!(resolve("/dev/sdrctl1").data, "/dev/sdr1");
    }

    #[test]
    fn status_parses_key_value_lines() {
        let s = Status::parse("tier sdr_standard\nfreq 146520000\nrate 48000\nformat cf32\ntimestamp 9600\ngain 30.0\n");
        assert_eq!(s.freq(), Some(146.52e6));
        assert_eq!(s.rate(), Some(48_000.0));
        assert_eq!(s.timestamp(), Some(9600));
        assert_eq!(s.format(), SampleFormat::Cf32);
        assert_eq!(s.get("tier"), Some("sdr_standard"));
        assert_eq!(Status::parse("").format(), SampleFormat::Cs16);
    }

    #[test]
    fn pacer_holds_writer_near_the_clock() {
        let mut p = TxPacer::new(48_000.0, 0.25);
        assert_eq!(p.wait_seconds(0), 0.0);
        p.sent(48_000, 1000);
        // one second written at t=1000, clock hasn't moved: wait 0.75 s
        assert!((p.wait_seconds(1000) - 0.75).abs() < 1e-9);
        assert_eq!(p.wait_seconds(1000 + 36_000), 0.0);
        assert!((p.remaining_seconds(1000 + 24_000) - 0.5).abs() < 1e-9);
        p.reset();
        assert_eq!(p.total(), 0);
    }

    #[test]
    fn device_files_round_trip_on_plain_files() {
        let dir = std::env::temp_dir().join(format!("ecm-radio-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctl = dir.join("sdrctl0");
        let data = dir.join("sdr0");
        std::fs::write(&ctl, "rate 48000\nformat cs16\ntimestamp 5\n").unwrap();
        let iq: Vec<C32> = (0..100).map(|i| C32::new(i as f32 / 200.0, -0.25)).collect();
        let mut bytes = Vec::new();
        SampleFormat::Cs16.encode(&iq, &mut bytes);
        std::fs::write(&data, &bytes).unwrap();
        let paths = SdrPaths { data: data.to_string_lossy().into(), ctl: ctl.to_string_lossy().into() };
        let mut s = Sdr::with_paths(paths.clone()).unwrap();
        assert_eq!(s.timestamp().unwrap(), 5);
        let got = s.read_n(100, 2).unwrap();
        assert_eq!(got.len(), 100);
        assert!((got[50].re - 0.25).abs() < 1e-3 && (got[50].im + 0.25).abs() < 1e-3);
        s.tune(146.52e6).unwrap();
        assert!(std::fs::read_to_string(&ctl).unwrap().starts_with("freq 146520000\n"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Sdr::with_paths(paths).is_err());
    }
}
