//! Command-line parsing for the radio programs (kept here so it is tested on
//! the host). Each `parse_*` takes the arguments after the program name.

use crate::units::{parse_freq, parse_rate};
use std::collections::BTreeMap;

/// Positionals plus `--name value` / `--flag` options.
#[derive(Debug, Default)]
pub struct Opts {
    pub pos: Vec<String>,
    pub vals: BTreeMap<String, String>,
    pub flags: Vec<String>,
}

/// Split `args`: options in `with_value` take the next argument, options in
/// `switches` don't; anything else starting with `--` is an error.
pub fn split(args: &[String], with_value: &[&str], switches: &[&str]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (name, None),
            };
            if with_value.contains(&name) {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i).cloned().ok_or_else(|| format!("--{name} needs a value"))?
                    }
                };
                o.vals.insert(name.to_string(), v);
            } else if switches.contains(&name) && inline.is_none() {
                o.flags.push(name.to_string());
            } else {
                return Err(format!("unknown option --{name}"));
            }
        } else if a == "-h" {
            return Err("help".into());
        } else {
            o.pos.push(a.clone());
        }
        i += 1;
    }
    if o.flags.iter().any(|f| f == "help") {
        return Err("help".into());
    }
    Ok(o)
}

impl Opts {
    pub fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    pub fn str(&self, k: &str) -> Option<String> {
        self.vals.get(k).cloned()
    }
    pub fn freq(&self, k: &str) -> Result<Option<f64>, String> {
        self.vals
            .get(k)
            .map(|v| parse_freq(v).ok_or_else(|| format!("--{k}: bad frequency {v:?}")))
            .transpose()
    }
    pub fn rate(&self, k: &str) -> Result<Option<u32>, String> {
        self.vals
            .get(k)
            .map(|v| parse_rate(v).ok_or_else(|| format!("--{k}: bad rate {v:?}")))
            .transpose()
    }
    pub fn num(&self, k: &str) -> Result<Option<f64>, String> {
        self.vals
            .get(k)
            .map(|v| v.parse::<f64>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("--{k}: bad number {v:?}")))
            .transpose()
    }
}

fn pos_freq(o: &Opts, i: usize, what: &str) -> Result<f64, String> {
    let s = o.pos.get(i).ok_or_else(|| format!("missing {what}"))?;
    parse_freq(s).ok_or_else(|| format!("bad {what} {s:?}"))
}

/// `rx_fm` / `rx_am` / `rx_ssb`.
#[derive(Clone, Debug, PartialEq)]
pub struct RxArgs {
    pub freq: f64,
    pub mode: String,
    pub sdr: String,
    pub rate: u32,
    pub squelch_db: Option<f32>,
    /// `None` = AGC.
    pub gain_db: Option<f64>,
    pub speaker: Option<String>,
    pub volume: Option<u8>,
    pub seconds: Option<f64>,
    /// Also (or, with `--no-speaker`, only) write the audio to this WAV file.
    pub wav: Option<String>,
    pub no_speaker: bool,
    pub audio_rate: u32,
}

pub const RX_USAGE: &str = "<freq> [--sdr NAME] [--rate SPS] [--squelch DB] [--gain DB] [--speaker SIDE] [--volume 0-100] [--seconds S] [--wav FILE] [--no-speaker] [--audio-rate HZ]";

/// `default_mode`: `fm`, `am` or `usb`. `rx_fm --wide` is broadcast FM;
/// `rx_ssb <freq> lsb` picks the sideband.
pub fn parse_rx(args: &[String], default_mode: &str) -> Result<RxArgs, String> {
    let o = split(
        args,
        &["sdr", "rate", "squelch", "gain", "speaker", "volume", "seconds", "wav", "audio-rate", "mode"],
        &["wide", "no-speaker", "help"],
    )?;
    let freq = pos_freq(&o, 0, "frequency")?;
    let mut mode = o.str("mode").unwrap_or_else(|| default_mode.to_string()).to_ascii_lowercase();
    if let Some(sb) = o.pos.get(1) {
        mode = sb.to_ascii_lowercase();
    }
    if o.flag("wide") {
        mode = "wbfm".into();
    }
    if o.pos.len() > 2 {
        return Err(format!("unexpected argument {:?}", o.pos[2]));
    }
    let ok = match default_mode {
        "fm" => matches!(mode.as_str(), "fm" | "nbfm" | "wbfm"),
        "am" => mode == "am",
        _ => matches!(mode.as_str(), "usb" | "lsb"),
    };
    if !ok {
        return Err(format!("mode {mode:?} doesn't fit this receiver"));
    }
    let default_rate = if mode == "wbfm" { 240_000 } else { 48_000 };
    let volume = o.num("volume")?.map(|v| v.clamp(0.0, 100.0) as u8);
    let no_speaker = o.flag("no-speaker");
    if no_speaker && o.str("wav").is_none() {
        return Err("--no-speaker needs --wav FILE".into());
    }
    let audio_rate = o.rate("audio-rate")?.unwrap_or(if mode == "wbfm" { 48_000 } else { 24_000 });
    if !(8_000..=48_000).contains(&audio_rate) {
        return Err("--audio-rate must be 8000-48000".into());
    }
    Ok(RxArgs {
        freq,
        mode,
        sdr: o.str("sdr").unwrap_or_default(),
        rate: o.rate("rate")?.unwrap_or(default_rate),
        squelch_db: o.num("squelch")?.map(|v| v as f32),
        gain_db: o.num("gain")?,
        speaker: o.str("speaker"),
        volume,
        seconds: o.num("seconds")?,
        wav: o.str("wav"),
        no_speaker,
        audio_rate,
    })
}

/// `waterfall`.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterfallArgs {
    pub freq: f64,
    pub sdr: String,
    pub rate: u32,
    pub screen: bool,
    pub text: bool,
    pub min_db: f32,
    pub max_db: f32,
    pub seconds: Option<f64>,
    pub width: usize,
    pub height: usize,
    pub gain_db: Option<f64>,
}

pub const WATERFALL_USAGE: &str =
    "<freq> [--sdr NAME] [--rate SPS] [--screen | --text] [--min DB] [--max DB] [--size WxH] [--gain DB] [--seconds S]";

pub fn parse_waterfall(args: &[String]) -> Result<WaterfallArgs, String> {
    let o = split(args, &["sdr", "rate", "min", "max", "size", "seconds", "gain"], &["screen", "text", "help"])?;
    let freq = pos_freq(&o, 0, "frequency")?;
    if o.flag("screen") && o.flag("text") {
        return Err("--screen and --text don't mix".into());
    }
    let (width, height) = match o.str("size") {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size is WxH")?;
            let w: usize = w.parse().map_err(|_| "--size is WxH")?;
            let h: usize = h.parse().map_err(|_| "--size is WxH")?;
            if !(16..=1024).contains(&w) || !(16..=1024).contains(&h) {
                return Err("--size must be 16..1024 each way".into());
            }
            (w, h)
        }
        None => (256, 160),
    };
    let min_db = o.num("min")?.unwrap_or(-110.0) as f32;
    let max_db = o.num("max")?.unwrap_or(-20.0) as f32;
    if max_db <= min_db {
        return Err("--max must be above --min".into());
    }
    Ok(WaterfallArgs {
        freq,
        sdr: o.str("sdr").unwrap_or_default(),
        rate: o.rate("rate")?.unwrap_or(48_000),
        screen: o.flag("screen"),
        text: o.flag("text"),
        min_db,
        max_db,
        seconds: o.num("seconds")?,
        width,
        height,
        gain_db: o.num("gain")?,
    })
}

/// `scan`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanArgs {
    pub start: f64,
    pub stop: f64,
    pub sdr: String,
    pub rate: u32,
    pub threshold_db: f32,
    pub dwell_ms: u32,
    pub passes: u32,
    pub log: Option<String>,
    pub gain_db: f64,
}

pub const SCAN_USAGE: &str =
    "<start> <stop> [--sdr NAME] [--rate SPS] [--threshold DB] [--dwell MS] [--passes N] [--gain DB] [--log FILE]";

pub fn parse_scan(args: &[String]) -> Result<ScanArgs, String> {
    let o = split(args, &["sdr", "rate", "threshold", "dwell", "passes", "log", "gain"], &["help"])?;
    let start = pos_freq(&o, 0, "start frequency")?;
    let stop = pos_freq(&o, 1, "stop frequency")?;
    if stop <= start {
        return Err("stop must be above start".into());
    }
    let dwell = o.num("dwell")?.unwrap_or(100.0);
    if !(10.0..=10_000.0).contains(&dwell) {
        return Err("--dwell must be 10-10000 ms".into());
    }
    Ok(ScanArgs {
        start,
        stop,
        sdr: o.str("sdr").unwrap_or_default(),
        rate: o.rate("rate")?.unwrap_or(48_000),
        threshold_db: o.num("threshold")?.unwrap_or(15.0) as f32,
        dwell_ms: dwell as u32,
        passes: o.num("passes")?.unwrap_or(1.0).max(1.0) as u32,
        log: o.str("log"),
        gain_db: o.num("gain")?.unwrap_or(30.0),
    })
}

/// `tx_tone`.
#[derive(Clone, Debug, PartialEq)]
pub struct TxToneArgs {
    pub freq: f64,
    pub sdr: String,
    pub rate: u32,
    /// Offset of the carrier from the tuned frequency (Hz).
    pub offset: f64,
    /// FM-modulate an audio tone of this frequency instead of a bare carrier.
    pub fm_audio: Option<f64>,
    pub seconds: f64,
    pub power_dbm: Option<f64>,
    pub iq_out: Option<String>,
}

pub const TX_TONE_USAGE: &str =
    "<freq> [--offset HZ] [--fm AUDIO_HZ] [--seconds S] [--power DBM] [--sdr NAME] [--rate SPS] [--iq FILE]";

pub fn parse_tx_tone(args: &[String]) -> Result<TxToneArgs, String> {
    let o = split(args, &["sdr", "rate", "offset", "fm", "seconds", "power", "iq"], &["help"])?;
    let freq = pos_freq(&o, 0, "frequency")?;
    let rate = o.rate("rate")?.unwrap_or(48_000);
    let offset = match o.str("offset") {
        Some(s) => {
            let neg = s.starts_with('-');
            let v = parse_freq(s.trim_start_matches('-')).ok_or_else(|| format!("--offset: bad value {s:?}"))?;
            if neg { -v } else { v }
        }
        None => 0.0,
    };
    if offset.abs() >= rate as f64 / 2.0 {
        return Err("--offset must be inside +-rate/2".into());
    }
    let seconds = o.num("seconds")?.unwrap_or(5.0);
    if !(seconds > 0.0 && seconds <= 3600.0) {
        return Err("--seconds must be 0-3600".into());
    }
    Ok(TxToneArgs {
        freq,
        sdr: o.str("sdr").unwrap_or_default(),
        rate,
        offset,
        fm_audio: o.freq("fm")?,
        seconds,
        power_dbm: o.num("power")?,
        iq_out: o.str("iq"),
    })
}

/// `afsk1200 send|recv`.
#[derive(Clone, Debug, PartialEq)]
pub enum AfskCmd {
    Send { src: String, dest: String, text: String, repeat: u32 },
    Recv { seconds: Option<f64>, count: Option<u32> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AfskArgs {
    pub freq: f64,
    pub cmd: AfskCmd,
    pub sdr: String,
    pub rate: u32,
    pub power_dbm: Option<f64>,
    /// Read / write IQ (cf32) from a file instead of the SDR.
    pub iq: Option<String>,
}

pub const AFSK_USAGE: &str = "send <freq> <SRC> <DEST> <text...> [--repeat N] [--power DBM] | recv <freq> [--seconds S] [--count N]   (both: [--sdr NAME] [--rate SPS] [--iq FILE])";

pub fn parse_afsk(args: &[String]) -> Result<AfskArgs, String> {
    let o = split(args, &["sdr", "rate", "power", "repeat", "seconds", "count", "iq"], &["help"])?;
    let verb = o.pos.first().ok_or("missing send or recv")?.as_str();
    let freq = pos_freq(&o, 1, "frequency")?;
    let cmd = match verb {
        "send" => {
            let src = o.pos.get(2).ok_or("missing source callsign")?.to_ascii_uppercase();
            let dest = o.pos.get(3).ok_or("missing destination callsign")?.to_ascii_uppercase();
            for c in [&src, &dest] {
                ecm_dsp::coding::ax25::Address::parse(c).map_err(|_| format!("bad callsign {c:?}"))?;
            }
            let text = o.pos[4..].join(" ");
            if text.is_empty() {
                return Err("missing text".into());
            }
            if text.len() > 256 {
                return Err("text is limited to 256 bytes".into());
            }
            AfskCmd::Send { src, dest, text, repeat: o.num("repeat")?.unwrap_or(1.0).clamp(1.0, 100.0) as u32 }
        }
        "recv" => {
            if o.pos.len() > 2 {
                return Err(format!("unexpected argument {:?}", o.pos[2]));
            }
            AfskCmd::Recv { seconds: o.num("seconds")?, count: o.num("count")?.map(|c| c.max(1.0) as u32) }
        }
        other => return Err(format!("unknown command {other:?} (send or recv)")),
    };
    let rate = o.rate("rate")?.unwrap_or(48_000);
    if rate < 9_600 {
        return Err("afsk1200 needs at least 9600 samples/s".into());
    }
    Ok(AfskArgs { freq, cmd, sdr: o.str("sdr").unwrap_or_default(), rate, power_dbm: o.num("power")?, iq: o.str("iq") })
}

/// `radio_station`.
#[derive(Clone, Debug, PartialEq)]
pub struct StationArgs {
    pub file: String,
    pub freq: f64,
    pub mode: String,
    pub sdr: String,
    pub rate: u32,
    pub power_dbm: Option<f64>,
    pub repeat: bool,
    pub raw_rate: u32,
    pub iq_out: Option<String>,
}

pub const STATION_USAGE: &str =
    "<file.wav|file.pcm> <freq> [--mode fm|am|wbfm] [--power DBM] [--loop] [--sdr NAME] [--rate SPS] [--raw-rate HZ] [--iq FILE]";

pub fn parse_station(args: &[String]) -> Result<StationArgs, String> {
    let o = split(args, &["mode", "power", "sdr", "rate", "raw-rate", "iq"], &["loop", "help"])?;
    let file = o.pos.first().ok_or("missing audio file")?.clone();
    let freq = pos_freq(&o, 1, "frequency")?;
    let mode = o.str("mode").unwrap_or_else(|| "fm".into()).to_ascii_lowercase();
    if !matches!(mode.as_str(), "fm" | "am" | "wbfm") {
        return Err(format!("--mode must be fm, am or wbfm, got {mode:?}"));
    }
    let rate = o.rate("rate")?.unwrap_or(if mode == "wbfm" { 240_000 } else { 48_000 });
    Ok(StationArgs {
        file,
        freq,
        mode,
        sdr: o.str("sdr").unwrap_or_default(),
        rate,
        power_dbm: o.num("power")?,
        repeat: o.flag("loop"),
        raw_rate: o.rate("raw-rate")?.unwrap_or(8_000),
        iq_out: o.str("iq"),
    })
}

/// `iqrec`.
#[derive(Clone, Debug, PartialEq)]
pub struct IqrecArgs {
    pub freq: f64,
    pub file: String,
    pub seconds: f64,
    pub sdr: String,
    pub rate: u32,
    pub gain_db: Option<f64>,
    pub format: ecm_dsp::iq::SampleFormat,
}

pub const IQREC_USAGE: &str = "<freq> <file.cf32|file.cs16> [--seconds S] [--rate SPS] [--gain DB] [--sdr NAME]";

pub fn parse_iqrec(args: &[String]) -> Result<IqrecArgs, String> {
    let o = split(args, &["seconds", "rate", "gain", "sdr", "format"], &["help"])?;
    let freq = pos_freq(&o, 0, "frequency")?;
    let file = o.pos.get(1).ok_or("missing output file")?.clone();
    let format = match o.str("format") {
        Some(f) => crate::sigmf::format_from_datatype(&f).ok_or_else(|| format!("bad --format {f:?}"))?,
        None => crate::sigmf::format_from_path(&file).unwrap_or(ecm_dsp::iq::SampleFormat::Cf32),
    };
    let seconds = o.num("seconds")?.unwrap_or(5.0);
    if !(seconds > 0.0 && seconds <= 600.0) {
        return Err("--seconds must be 0-600".into());
    }
    Ok(IqrecArgs {
        freq,
        file,
        seconds,
        sdr: o.str("sdr").unwrap_or_default(),
        rate: o.rate("rate")?.unwrap_or(48_000),
        gain_db: o.num("gain")?,
        format,
    })
}

/// `iqplay`.
#[derive(Clone, Debug, PartialEq)]
pub struct IqplayArgs {
    pub file: String,
    pub freq: Option<f64>,
    pub rate: Option<u32>,
    pub sdr: String,
    pub power_dbm: Option<f64>,
    pub repeat: bool,
}

pub const IQPLAY_USAGE: &str = "<file.cf32|file.cs16|file.sigmf-data> [freq] [--rate SPS] [--power DBM] [--loop] [--sdr NAME]";

pub fn parse_iqplay(args: &[String]) -> Result<IqplayArgs, String> {
    let o = split(args, &["rate", "power", "sdr"], &["loop", "help"])?;
    let file = o.pos.first().ok_or("missing IQ file")?.clone();
    let freq = match o.pos.get(1) {
        Some(_) => Some(pos_freq(&o, 1, "frequency")?),
        None => None,
    };
    Ok(IqplayArgs { file, freq, rate: o.rate("rate")?, sdr: o.str("sdr").unwrap_or_default(), power_dbm: o.num("power")?, repeat: o.flag("loop") })
}

/// `radiod`.
#[derive(Clone, Debug, PartialEq)]
pub struct RadiodArgs {
    pub iface: String,
    pub sdr: String,
    pub freq: f64,
    pub call: String,
    /// Address and prefix for the interface.
    pub ip: Option<([u8; 4], u8)>,
    pub rate: u32,
    pub power_dbm: Option<f64>,
    pub txdelay_ms: u32,
    /// Exit after this many seconds (tests); default: run until killed.
    pub seconds: Option<f64>,
    /// Log every frame sent and received.
    pub verbose: bool,
    /// Fixed receive gain (dB); `None` = AGC.
    pub gain_db: Option<f64>,
}

pub const RADIOD_USAGE: &str = "<iface> up <sdr> <freq> --call CALL[-SSID] [--ip A.B.C.D/N] [--rate SPS] [--power DBM] [--txdelay MS] [--gain DB] [--seconds S] [-v]";

pub fn parse_ipv4_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let (ip, prefix) = match s.split_once('/') {
        Some((a, p)) => (a, p.parse::<u8>().ok().filter(|p| *p <= 32)?),
        None => (s, 24),
    };
    let parts: Vec<u8> = ip.split('.').map(|p| p.parse().ok()).collect::<Option<Vec<u8>>>()?;
    (parts.len() == 4).then(|| ([parts[0], parts[1], parts[2], parts[3]], prefix))
}

pub fn parse_radiod(args: &[String]) -> Result<RadiodArgs, String> {
    let verbose = args.iter().any(|a| a == "-v");
    let args: Vec<String> = args.iter().filter(|a| *a != "-v").cloned().collect();
    let o = split(&args, &["call", "ip", "rate", "power", "txdelay", "seconds", "gain"], &["verbose", "help"])?;
    let iface = o.pos.first().ok_or("missing interface name (e.g. radio0)")?.clone();
    if iface.is_empty() || iface.len() > 15 || !iface.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("bad interface name {iface:?}"));
    }
    if o.pos.get(1).map(String::as_str) != Some("up") {
        return Err("expected `up` after the interface name".into());
    }
    let sdr = o.pos.get(2).ok_or("missing SDR name (e.g. sdr_0)")?.clone();
    let freq = pos_freq(&o, 3, "frequency")?;
    let call = o.str("call").ok_or("missing --call CALLSIGN (your station's AX.25 address)")?.to_ascii_uppercase();
    ecm_dsp::coding::ax25::Address::parse(&call).map_err(|_| format!("bad callsign {call:?}"))?;
    let ip = match o.str("ip") {
        Some(s) => Some(parse_ipv4_cidr(&s).ok_or_else(|| format!("bad --ip {s:?} (A.B.C.D/N)"))?),
        None => None,
    };
    let rate = o.rate("rate")?.unwrap_or(48_000);
    if rate < 9_600 {
        return Err("radiod needs at least 9600 samples/s".into());
    }
    Ok(RadiodArgs {
        iface,
        sdr,
        freq,
        call,
        ip,
        rate,
        power_dbm: o.num("power")?,
        txdelay_ms: o.num("txdelay")?.unwrap_or(300.0).clamp(10.0, 2000.0) as u32,
        seconds: o.num("seconds")?,
        verbose: verbose || o.flag("verbose"),
        gain_db: o.num("gain")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn rx_args() {
        let r = parse_rx(&a("146.52M --squelch -30 --volume 150 --sdr left"), "fm").unwrap();
        assert_eq!(r.freq, 146.52e6);
        assert_eq!(r.mode, "fm");
        assert_eq!(r.rate, 48_000);
        assert_eq!(r.squelch_db, Some(-30.0));
        assert_eq!(r.volume, Some(100));
        assert_eq!(r.sdr, "left");
        assert_eq!(r.gain_db, None);
        let w = parse_rx(&a("98.1e6 --wide"), "fm").unwrap();
        assert_eq!((w.mode.as_str(), w.rate, w.audio_rate), ("wbfm", 240_000, 48_000));
        assert_eq!(parse_rx(&a("7.1M lsb"), "usb").unwrap().mode, "lsb");
        assert_eq!(parse_rx(&a("14.2M"), "usb").unwrap().mode, "usb");
        assert!(parse_rx(&a("7.1M am"), "usb").is_err());
        assert!(parse_rx(&a("--squelch -30"), "fm").is_err());
        assert!(parse_rx(&a("1M --bogus 1"), "am").is_err());
        assert!(parse_rx(&a("1M --no-speaker"), "am").is_err());
        assert_eq!(parse_rx(&a("1M --help"), "am").unwrap_err(), "help");
        let r = parse_rx(&a("1M --wav out.wav --no-speaker --seconds=2"), "am").unwrap();
        assert_eq!((r.wav.as_deref(), r.no_speaker, r.seconds), (Some("out.wav"), true, Some(2.0)));
    }

    #[test]
    fn other_program_args() {
        let w = parse_waterfall(&a("433.92M --text --size 80x24 --min -90")).unwrap();
        assert_eq!((w.width, w.height, w.text, w.min_db), (80, 24, true, -90.0));
        assert!(parse_waterfall(&a("1M --screen --text")).is_err());
        assert!(parse_waterfall(&a("1M --size 9x9")).is_err());

        let s = parse_scan(&a("144M 146M --threshold 20 --log hits.txt")).unwrap();
        assert_eq!((s.start, s.stop, s.threshold_db, s.log.as_deref()), (144e6, 146e6, 20.0, Some("hits.txt")));
        assert!(parse_scan(&a("146M 144M")).is_err());

        let t = parse_tx_tone(&a("433.92M --offset -5k --seconds 2 --power 20")).unwrap();
        assert_eq!((t.offset, t.seconds, t.power_dbm), (-5000.0, 2.0, Some(20.0)));
        assert!(parse_tx_tone(&a("433.92M --offset 30k")).is_err());

        let f = parse_afsk(&a("send 144.39M n0call-1 aprs hello there --repeat 3")).unwrap();
        assert_eq!(
            f.cmd,
            AfskCmd::Send { src: "N0CALL-1".into(), dest: "APRS".into(), text: "hello there".into(), repeat: 3 }
        );
        assert_eq!(parse_afsk(&a("recv 144.39M --seconds 10")).unwrap().cmd, AfskCmd::Recv { seconds: Some(10.0), count: None });
        assert!(parse_afsk(&a("send 144.39M TOOLONGCALL APRS hi")).is_err());
        assert!(parse_afsk(&a("send 144.39M A B")).is_err());
        assert!(parse_afsk(&a("listen 144.39M")).is_err());

        let st = parse_station(&a("song.wav 100.1M --mode am --loop")).unwrap();
        assert_eq!((st.mode.as_str(), st.repeat, st.rate), ("am", true, 48_000));
        assert!(parse_station(&a("song.wav 100.1M --mode ssb")).is_err());

        let r = parse_iqrec(&a("433.92M cap.cs16 --seconds 2")).unwrap();
        assert_eq!(r.format, ecm_dsp::iq::SampleFormat::Cs16);
        assert_eq!(parse_iqrec(&a("433.92M cap.sigmf-data")).unwrap().format, ecm_dsp::iq::SampleFormat::Cf32);
        let p = parse_iqplay(&a("cap.cf32 --loop")).unwrap();
        assert_eq!((p.freq, p.repeat), (None, true));
        assert_eq!(parse_iqplay(&a("cap.cf32 145M")).unwrap().freq, Some(145e6));

        let d = parse_radiod(&a("radio0 up sdr_0 144.39e6 --call n0call-1 --ip 10.44.0.1/24")).unwrap();
        assert_eq!((d.iface.as_str(), d.sdr.as_str(), d.freq, d.call.as_str()), ("radio0", "sdr_0", 144.39e6, "N0CALL-1"));
        assert_eq!(d.ip, Some(([10, 44, 0, 1], 24)));
        assert!(!d.verbose);
        assert_eq!(d.gain_db, None);
        assert!(parse_radiod(&a("radio0 up sdr_0 144.39e6 --call n0call-1 -v")).unwrap().verbose);
        assert!(parse_radiod(&a("radio0 up sdr_0 144.39e6")).is_err());
        assert!(parse_radiod(&a("radio0 down")).is_err());
        assert!(parse_radiod(&a("radio0 up sdr_0 144.39e6 --call X --ip 10.0.0.300")).is_err());
    }
}
