//! The radio programs' main loops (`rx_fm`, `tx_tone`, `afsk1200`,
//! `radio_station`, `iqrec`, `iqplay`, `scan`). They use only `std` and the
//! device files, so each program's `main` is one call into here. Programs
//! that need the host graphics or socket ABI (`waterfall`, `radiod`) use the
//! helpers from here and keep their own loop.

use crate::blocks::{receiver, transmitter, Buf};
use crate::cli::{self, AfskCmd};
use crate::device::{Sdr, TxPacer};
use crate::modem::{FmPacketRx, FmPacketTx};
use crate::sigmf::{format_from_path, meta_path, Meta};
use crate::spectrum::{find_signals, merge_hits, scan_plan, Hit};
use crate::units::fmt_freq;
use ecm_dsp::iq::SampleFormat;
use ecm_dsp::C32;
use std::io::Write;
use std::time::Duration;

/// Print usage / error and exit like the other ECM programs.
pub fn fail(prog: &str, usage: &str, e: String) -> ! {
    if e == "help" {
        println!("usage: {prog} {usage}");
        std::process::exit(0);
    }
    eprintln!("{prog}: {e}");
    eprintln!("usage: {prog} {usage}");
    std::process::exit(1);
}

pub fn args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn no_sdr(name: &str, e: std::io::Error) -> String {
    let shown = if name.is_empty() { "the first SDR".to_string() } else { format!("SDR {name:?}") };
    format!("can't open {shown} ({e}); place an SDR block next to the computer")
}

/// Open an SDR for receiving: tune, set rate and gain (None = AGC).
pub fn setup_rx(name: &str, freq: f64, rate: u32, gain_db: Option<f64>) -> Result<Sdr, String> {
    let mut s = Sdr::open(name).map_err(|e| no_sdr(name, e))?;
    s.set_format(SampleFormat::Cs16).map_err(|e| format!("format: {e}"))?;
    s.set_rate(rate).map_err(|e| format!("rate {rate}: {e}"))?;
    s.tune(freq).map_err(|e| format!("tune {}: {e}", fmt_freq(freq)))?;
    match gain_db {
        Some(g) => s.set_gain(g).map_err(|e| format!("gain {g}: {e}"))?,
        None => s.set_agc(true).map_err(|e| format!("agc: {e}"))?,
    }
    Ok(s)
}

/// Open an SDR for transmitting at `freq` / `rate` (power None = tier max).
pub fn setup_tx(name: &str, freq: f64, rate: u32, power: Option<f64>) -> Result<Sdr, String> {
    let mut s = Sdr::open(name).map_err(|e| no_sdr(name, e))?;
    s.set_format(SampleFormat::Cs16).map_err(|e| format!("format: {e}"))?;
    s.set_rate(rate).map_err(|e| format!("rate {rate}: {e}"))?;
    s.tune(freq).map_err(|e| format!("tune {}: {e}", fmt_freq(freq)))?;
    s.set_tx(true, power).map_err(|e| format!("can't transmit ({e}); a Basic SDR is receive-only"))?;
    Ok(s)
}

/// Where transmitted samples go: an SDR (paced to the world clock) or an IQ
/// file (`--iq`, with SigMF metadata) for offline checks.
pub enum TxOut {
    Sdr { sdr: Sdr, pacer: TxPacer },
    File { path: String, data: Vec<u8>, rate: f64, freq: f64, description: String },
}

impl TxOut {
    pub fn open(sdr: &str, iq: Option<&str>, freq: f64, rate: u32, power: Option<f64>, what: &str) -> Result<TxOut, String> {
        Ok(match iq {
            Some(p) => TxOut::File { path: p.to_string(), data: Vec::new(), rate: rate as f64, freq, description: what.to_string() },
            None => TxOut::Sdr { sdr: setup_tx(sdr, freq, rate, power)?, pacer: TxPacer::new(rate as f64, 0.3) },
        })
    }

    pub fn send(&mut self, x: &[C32]) -> Result<(), String> {
        match self {
            TxOut::File { data, .. } => {
                SampleFormat::Cf32.encode(x, data);
                Ok(())
            }
            TxOut::Sdr { sdr, pacer } => send_paced(sdr, pacer, x),
        }
    }

    /// Wait for the queued samples to go out, then switch off / write the file.
    pub fn finish(self) -> Result<(), String> {
        match self {
            TxOut::File { path, data, rate, freq, description } => {
                std::fs::write(&path, &data).map_err(|e| format!("{path}: {e}"))?;
                let m = Meta { format: SampleFormat::Cf32, sample_rate: rate, frequency: freq, description, start_timestamp: None, hw: "ecm".into() };
                std::fs::write(meta_path(&path), m.to_json()).map_err(|e| format!("{}: {e}", meta_path(&path)))?;
                Ok(())
            }
            TxOut::Sdr { mut sdr, pacer } => {
                wait_sent(&mut sdr, &pacer)?;
                sdr.set_tx(false, None).map_err(|e| e.to_string())
            }
        }
    }
}

/// Write samples to a transmitting SDR, staying at most `pacer.lead` ahead
/// of the world clock.
pub fn send_paced(sdr: &mut Sdr, pacer: &mut TxPacer, x: &[C32]) -> Result<(), String> {
    for chunk in x.chunks(4800) {
        let now = sdr.timestamp().map_err(|e| e.to_string())?;
        let wait = pacer.wait_seconds(now);
        if wait > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(wait));
        }
        let now = sdr.timestamp().map_err(|e| e.to_string())?;
        sdr.write(chunk).map_err(|e| format!("transmit: {e}"))?;
        pacer.sent(chunk.len(), now);
    }
    Ok(())
}

/// Wait (bounded) until everything written has been sent on the clock.
pub fn wait_sent(sdr: &mut Sdr, pacer: &TxPacer) -> Result<(), String> {
    for _ in 0..40 {
        let now = sdr.timestamp().map_err(|e| e.to_string())?;
        let left = pacer.remaining_seconds(now);
        if left <= 0.0 {
            break;
        }
        std::thread::sleep(Duration::from_secs_f64(left.min(0.25) + 0.01));
    }
    Ok(())
}

// ---------------------------------------------------------------- receivers

/// `rx_fm`, `rx_am`, `rx_ssb`.
pub fn rx_main(prog: &str, default_mode: &str) {
    let a = cli::parse_rx(&args(), default_mode).unwrap_or_else(|e| fail(prog, cli::RX_USAGE, e));
    if let Err(e) = rx(&a) {
        eprintln!("{prog}: {e}");
        std::process::exit(1);
    }
}

pub fn rx(a: &cli::RxArgs) -> Result<(), String> {
    let mut chain = receiver(&a.mode, a.rate as f64, a.audio_rate as f64, a.squelch_db)?;
    let mut sdr = setup_rx(&a.sdr, a.freq, a.rate, a.gain_db)?;
    let mut spk = if a.no_speaker {
        None
    } else {
        let mut d = ecm_audio::Device::open(a.speaker.as_deref()).map_err(|e| format!("no speaker ({e}); place a Speaker next to the computer or use --wav FILE --no-speaker"))?;
        d.set_format(a.audio_rate, 16, 1).map_err(|e| format!("speaker: {e}"))?;
        if let Some(v) = a.volume {
            let _ = d.set_volume(v);
        }
        Some(d)
    };
    println!(
        "{} {} ({}), {} S/s -> {}{}  [Ctrl+T stops]",
        a.mode.to_ascii_uppercase(),
        fmt_freq(a.freq),
        chain.describe(),
        a.rate,
        if spk.is_some() { "speaker" } else { "" },
        a.wav.as_ref().map(|w| format!(" wav {w}")).unwrap_or_default()
    );
    let total = a.seconds.map(|s| (s * a.rate as f64) as u64);
    let mut done = 0u64;
    let mut wav: Vec<f32> = Vec::new();
    let mut since_status = 0u64;
    let mut level = Vec::new();
    while total.map_or(true, |t| done < t) {
        // ~20 ms reads: the SDR's AGC steps once per read, so it settles in ~0.2 s.
        let left = total.map_or(usize::MAX, |t| (t - done) as usize);
        let x = sdr.read_at_least((a.rate as usize / 50).min(left), (a.rate as usize / 10).min(left)).map_err(|e| format!("receive: {e}"))?;
        if x.is_empty() {
            continue;
        }
        done += x.len() as u64;
        since_status += x.len() as u64;
        level.push(ecm_dsp::complex::mean_power(&x));
        let Buf::R(audio) = chain.process(Buf::C(x))? else { unreachable!("receiver outputs audio") };
        if let Some(d) = spk.as_mut() {
            d.write_bytes(&crate::audio::to_pcm16(&audio)).map_err(|e| format!("speaker: {e}"))?;
        }
        // Keep audio only when the run is bounded (summary) or recorded.
        if a.wav.is_some() || a.seconds.is_some() {
            wav.extend_from_slice(&audio);
        }
        if since_status >= a.rate as u64 * 2 {
            let p = level.iter().sum::<f32>() / level.len().max(1) as f32;
            println!("  signal {:6.1} dBFS", ecm_dsp::complex::to_db(p));
            level.clear();
            since_status = 0;
        }
    }
    if let Some(w) = &a.wav {
        std::fs::write(w, crate::audio::wav_file(a.audio_rate, &wav)).map_err(|e| format!("{w}: {e}"))?;
        println!("wrote {w} ({:.1} s)", wav.len() as f64 / a.audio_rate as f64);
    }
    if a.seconds.is_some() {
        let peak = wav.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        match crate::spectrum::strongest_tone(&wav, a.audio_rate as f64) {
            Some((f, snr)) => println!(
                "{}: strongest audio tone {:.0} Hz, {:.0} dB over the noise; peak level {:.2}",
                a.mode, f, snr, peak
            ),
            None => println!("{}: no audio", a.mode),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- tx_tone

pub fn tx_tone_main() {
    let a = cli::parse_tx_tone(&args()).unwrap_or_else(|e| fail("tx_tone", cli::TX_TONE_USAGE, e));
    if let Err(e) = tx_tone(&a) {
        eprintln!("tx_tone: {e}");
        std::process::exit(1);
    }
}

pub fn tx_tone(a: &cli::TxToneArgs) -> Result<(), String> {
    let rate = a.rate as f64;
    let n = (a.seconds * rate) as usize;
    let mut out = TxOut::open(&a.sdr, a.iq_out.as_deref(), a.freq, a.rate, a.power_dbm, "tx_tone")?;
    let carrier = a.freq + a.offset;
    match a.fm_audio {
        Some(f) => println!("tx_tone: {} FM, {f} Hz audio, {:.1} s", fmt_freq(carrier), a.seconds),
        None => println!("tx_tone: carrier at {}, {:.1} s", fmt_freq(carrier), a.seconds),
    }
    let mut tone = crate::blocks::Tone::new(a.offset, rate, 1.0);
    let mut audio = crate::blocks::Tone::new(a.fm_audio.unwrap_or(0.0), rate, 0.8);
    let mut fm = a.fm_audio.map(|_| {
        crate::blocks::Chain::build(vec![crate::blocks::Spec::FmMod { deviation: 5_000.0, tau: 0.0 }, crate::blocks::Spec::Shift { hz: a.offset }], crate::blocks::Kind::Real, rate)
    });
    let mut left = n;
    while left > 0 {
        let k = left.min(4800);
        let x = match fm.as_mut() {
            Some(Ok(c)) => match c.process(Buf::R(audio.real(k)))? {
                Buf::C(v) => v,
                _ => unreachable!(),
            },
            Some(Err(e)) => return Err(e.clone()),
            None => tone.complex(k),
        };
        out.send(&x)?;
        left -= k;
    }
    out.finish()?;
    println!("tx_tone: done");
    Ok(())
}

// ---------------------------------------------------------------- radio_station

pub fn station_main() {
    let a = cli::parse_station(&args()).unwrap_or_else(|e| fail("radio_station", cli::STATION_USAGE, e));
    if let Err(e) = station(&a) {
        eprintln!("radio_station: {e}");
        std::process::exit(1);
    }
}

pub fn station(a: &cli::StationArgs) -> Result<(), String> {
    let songs = expand_sources(&a.sources)?;
    if songs.is_empty() {
        return Err("no audio files to play".into());
    }
    let mut out = TxOut::open(&a.sdr, a.iq_out.as_deref(), a.freq, a.rate, a.power_dbm, &format!("radio_station {}", a.sources.join(" ")))?;
    println!(
        "radio_station: {} song{} on {} {}{}",
        songs.len(),
        if songs.len() == 1 { "" } else { "s" },
        fmt_freq(a.freq),
        a.mode.to_ascii_uppercase(),
        if a.repeat { ", looping" } else { "" }
    );
    // Silent carrier between songs (and while a file is read).
    let gap = vec![0.0f32; (a.gap * 8_000.0) as usize];
    let mut played = 0usize;
    loop {
        for (i, path) in songs.iter().enumerate() {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    // A missing song skips; the station keeps going.
                    eprintln!("radio_station: {path}: {e}");
                    continue;
                }
            };
            let audio = crate::audio::read_audio(&bytes, a.raw_rate);
            if audio.samples.is_empty() {
                eprintln!("radio_station: {path}: no audio");
                continue;
            }
            let title = crate::audio::wav_title(&bytes).unwrap_or_else(|| cli::title_from_path(path));
            drop(bytes);
            println!(
                "now playing: {title} ({}/{}, {:.0} s, {} Hz)",
                i + 1,
                songs.len(),
                audio.samples.len() as f64 / audio.rate as f64,
                audio.rate
            );
            let _ = std::io::stdout().flush();
            // Normalise to a little under full scale so the modulation is full.
            let peak = audio.samples.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
            let g = 0.9 / peak;
            let mut chain = transmitter(&a.mode, audio.rate as f64, a.rate as f64)?;
            for c in audio.samples.chunks((audio.rate as usize / 10).max(1)) {
                let scaled: Vec<f32> = c.iter().map(|v| v * g).collect();
                if let Buf::C(x) = chain.process(Buf::R(scaled))? {
                    out.send(&x)?;
                }
            }
            if !gap.is_empty() {
                let mut quiet = transmitter(&a.mode, 8_000.0, a.rate as f64)?;
                for c in gap.chunks(800) {
                    if let Buf::C(x) = quiet.process(Buf::R(c.to_vec()))? {
                        out.send(&x)?;
                    }
                }
            }
            played += 1;
        }
        if played == 0 {
            return Err("none of the songs could be played".into());
        }
        if !a.repeat || matches!(out, TxOut::File { .. }) {
            break;
        }
    }
    out.finish()?;
    println!("radio_station: done");
    Ok(())
}

/// The songs `sources` name, in order: audio files as given, playlists
/// expanded line by line, directories as their audio files in name order.
pub fn expand_sources(sources: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in sources {
        let meta = std::fs::metadata(s).map_err(|e| format!("{s}: {e}"))?;
        if meta.is_dir() {
            let mut names: Vec<String> = std::fs::read_dir(s)
                .map_err(|e| format!("{s}: {e}"))?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| cli::is_audio(n))
                .collect();
            names.sort();
            let dir = s.trim_end_matches('/');
            out.extend(names.into_iter().map(|n| format!("{dir}/{n}")));
        } else if cli::is_playlist(s) {
            let text = std::fs::read_to_string(s).map_err(|e| format!("{s}: {e}"))?;
            out.extend(cli::parse_playlist(&text, s));
        } else {
            out.push(s.clone());
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- afsk1200

pub fn afsk_main() {
    let a = cli::parse_afsk(&args()).unwrap_or_else(|e| fail("afsk1200", cli::AFSK_USAGE, e));
    let r = match &a.cmd {
        AfskCmd::Send { .. } => afsk_send(&a),
        AfskCmd::Recv { .. } => afsk_recv(&a, &mut |line| println!("{line}")).map(|n| println!("afsk1200: {n} frame{} decoded", if n == 1 { "" } else { "s" })),
    };
    if let Err(e) = r {
        eprintln!("afsk1200: {e}");
        std::process::exit(1);
    }
}

pub fn afsk_send(a: &cli::AfskArgs) -> Result<(), String> {
    let AfskCmd::Send { src, dest, text, repeat } = &a.cmd else { return Err("not a send".into()) };
    let frame = ecm_dsp::coding::ax25::UiFrame::new(
        ecm_dsp::coding::ax25::Address::parse(dest).map_err(|e| e.to_string())?,
        ecm_dsp::coding::ax25::Address::parse(src).map_err(|e| e.to_string())?,
        text.as_bytes(),
    )
    .encode()
    .map_err(|e| e.to_string())?;
    let mut out = TxOut::open(&a.sdr, a.iq.as_deref(), a.freq, a.rate, a.power_dbm, "afsk1200")?;
    let mut tx = FmPacketTx::new(a.rate as f32);
    // 0.4 s of carrier first: receivers settle their AGC on it.
    let quiet = vec![C32::new(1.0, 0.0); a.rate as usize * 2 / 5];
    for i in 0..*repeat {
        out.send(&quiet)?;
        out.send(&tx.send(&frame))?;
        println!("afsk1200: sent {src}>{dest}: {text}{}", if *repeat > 1 { format!(" ({}/{repeat})", i + 1) } else { String::new() });
    }
    out.send(&quiet)?;
    out.finish()
}

/// Receive and print frames. Returns how many were decoded.
pub fn afsk_recv(a: &cli::AfskArgs, print: &mut dyn FnMut(String)) -> Result<usize, String> {
    let AfskCmd::Recv { seconds, count } = &a.cmd else { return Err("not a recv".into()) };
    let mut rx = FmPacketRx::new(a.rate as f32);
    let mut n = 0usize;
    let show = |f: Vec<u8>, print: &mut dyn FnMut(String)| match ecm_dsp::coding::ax25::UiFrame::parse(&f) {
        Ok(u) => print(format!("{u}")),
        Err(_) => print(format!("[{} bytes, not AX.25] {:02x?}", f.len(), &f[..f.len().min(32)])),
    };
    if let Some(path) = &a.iq {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let fmt = format_from_path(path).unwrap_or(SampleFormat::Cf32);
        let mut x = Vec::new();
        fmt.decode(&bytes, &mut x);
        for c in x.chunks(4800) {
            for f in rx.push(c) {
                show(f, print);
                n += 1;
            }
        }
        return Ok(n);
    }
    let mut sdr = setup_rx(&a.sdr, a.freq, a.rate, None)?;
    println!("afsk1200: listening on {} [Ctrl+T stops]", fmt_freq(a.freq));
    let total = seconds.map(|s| (s * a.rate as f64) as u64);
    let mut done = 0u64;
    while total.map_or(true, |t| done < t) && count.map_or(true, |c| (n as u32) < c) {
        // ~20 ms reads let the SDR's AGC (one step per read) settle during a packet's lead-in.
        let x = sdr.read_at_least(a.rate as usize / 50, a.rate as usize / 10).map_err(|e| format!("receive: {e}"))?;
        done += x.len() as u64;
        for f in rx.push(&x) {
            show(f, print);
            n += 1;
        }
    }
    if let Ok(st) = sdr.status() {
        let o = st.num("overflows").unwrap_or(0.0);
        if o > 0.0 || rx.bad_fcs() > 0 {
            println!("afsk1200: {} receive overflows, {} frames with bad FCS", o, rx.bad_fcs());
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------- iqrec / iqplay

pub fn iqrec_main() {
    let a = cli::parse_iqrec(&args()).unwrap_or_else(|e| fail("iqrec", cli::IQREC_USAGE, e));
    if let Err(e) = iqrec(&a) {
        eprintln!("iqrec: {e}");
        std::process::exit(1);
    }
}

pub fn iqrec(a: &cli::IqrecArgs) -> Result<(), String> {
    let mut sdr = setup_rx(&a.sdr, a.freq, a.rate, a.gain_db)?;
    let st = sdr.status().map_err(|e| e.to_string())?;
    let n = (a.seconds * a.rate as f64) as usize;
    let mut f = std::fs::File::create(&a.file).map_err(|e| format!("{}: {e}", a.file))?;
    let start = st.timestamp();
    let mut got = 0usize;
    let mut buf = Vec::new();
    let mut empty = 0;
    while got < n && empty < 3 {
        let x = sdr.read((n - got).min(a.rate as usize / 10)).map_err(|e| format!("receive: {e}"))?;
        if x.is_empty() {
            empty += 1;
            continue;
        }
        empty = 0;
        got += x.len();
        buf.clear();
        a.format.encode(&x, &mut buf);
        f.write_all(&buf).map_err(|e| format!("{}: {e}", a.file))?;
    }
    let m = Meta {
        format: a.format,
        sample_rate: a.rate as f64,
        frequency: a.freq,
        description: format!("iqrec {:.1} s", got as f64 / a.rate as f64),
        start_timestamp: start,
        hw: st.get("tier").unwrap_or("sdr").to_string(),
    };
    std::fs::write(meta_path(&a.file), m.to_json()).map_err(|e| e.to_string())?;
    println!("iqrec: {} samples ({}) -> {} + {}", got, a.format.name(), a.file, meta_path(&a.file));
    Ok(())
}

pub fn iqplay_main() {
    let a = cli::parse_iqplay(&args()).unwrap_or_else(|e| fail("iqplay", cli::IQPLAY_USAGE, e));
    if let Err(e) = iqplay(&a) {
        eprintln!("iqplay: {e}");
        std::process::exit(1);
    }
}

/// Load an IQ file and its metadata: (samples, rate, freq).
pub fn load_iq(path: &str, rate: Option<u32>, freq: Option<f64>) -> Result<(Vec<C32>, f64, f64), String> {
    let meta = std::fs::read_to_string(meta_path(path)).ok().and_then(|t| Meta::parse(&t).ok());
    let fmt = meta.as_ref().map(|m| m.format).or_else(|| format_from_path(path)).unwrap_or(SampleFormat::Cf32);
    let rate = rate.map(|r| r as f64).or(meta.as_ref().map(|m| m.sample_rate)).ok_or_else(|| format!("{path} has no .sigmf-meta: give --rate"))?;
    let freq = freq.or(meta.as_ref().map(|m| m.frequency)).filter(|f| *f > 0.0).ok_or_else(|| format!("{path} has no frequency in its .sigmf-meta: give one"))?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let mut x = Vec::new();
    fmt.decode(&bytes, &mut x);
    Ok((x, rate, freq))
}

pub fn iqplay(a: &cli::IqplayArgs) -> Result<(), String> {
    let (x, rate, freq) = load_iq(&a.file, a.rate, a.freq)?;
    if x.is_empty() {
        return Err(format!("{}: no samples", a.file));
    }
    let mut out = TxOut::open(&a.sdr, None, freq, rate.round() as u32, a.power_dbm, "iqplay")?;
    println!("iqplay: {} ({} samples, {} S/s) on {}", a.file, x.len(), rate, fmt_freq(freq));
    loop {
        out.send(&x)?;
        if !a.repeat {
            break;
        }
    }
    out.finish()?;
    println!("iqplay: done");
    Ok(())
}

// ---------------------------------------------------------------- scan

pub fn scan_main() {
    let a = cli::parse_scan(&args()).unwrap_or_else(|e| fail("scan", cli::SCAN_USAGE, e));
    if let Err(e) = scan(&a) {
        eprintln!("scan: {e}");
        std::process::exit(1);
    }
}

pub fn scan(a: &cli::ScanArgs) -> Result<Vec<Hit>, String> {
    let mut sdr = setup_rx(&a.sdr, a.start, a.rate, Some(a.gain_db))?;
    let plan = scan_plan(a.start, a.stop, a.rate as f64);
    println!(
        "scan: {} - {} in {} steps, {} ms each, threshold {} dB",
        fmt_freq(a.start),
        fmt_freq(a.stop),
        plan.len(),
        a.dwell_ms,
        a.threshold_db
    );
    let n = ((a.rate as u64 * a.dwell_ms as u64) / 1000).max(1024) as usize;
    let mut all = Vec::new();
    for pass in 0..a.passes {
        for &c in &plan {
            sdr.tune(c).map_err(|e| e.to_string())?;
            // the first read after retuning may hold samples from before it
            let _ = sdr.read(a.rate as usize / 50);
            let x = sdr.read_n(n, 3).map_err(|e| e.to_string())?;
            let hits = find_signals(&x, a.rate as f64, c, 1024, a.threshold_db);
            for h in &hits {
                if (h.freq - c).abs() <= a.rate as f64 * 0.4 && h.freq >= a.start && h.freq <= a.stop {
                    println!("  {}  {:6.1} dB  (+{:.0} dB over noise){}", fmt_freq(h.freq), h.power_db, h.snr_db, if a.passes > 1 { format!("  pass {}", pass + 1) } else { String::new() });
                    all.push(*h);
                }
            }
        }
    }
    let merged = merge_hits(all, a.rate as f64 / 100.0);
    println!("scan: {} active frequenc{}", merged.len(), if merged.len() == 1 { "y" } else { "ies" });
    if let Some(log) = &a.log {
        let mut text = String::new();
        for h in &merged {
            text.push_str(&format!("{:.0} {:.1} {:.1}\n", h.freq, h.power_db, h.snr_db));
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).map_err(|e| format!("{log}: {e}"))?;
        f.write_all(text.as_bytes()).map_err(|e| format!("{log}: {e}"))?;
        println!("scan: logged to {log}");
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// A playlist plays its songs in order with the gap after each, from a
    /// directory or an M3U file; a missing entry is skipped.
    #[test]
    fn radio_station_plays_a_playlist() {
        let dir = std::env::temp_dir().join(format!("ecm-radio-playlist-{}", std::process::id()));
        let songs = dir.join("songs");
        std::fs::create_dir_all(&songs).unwrap();
        let tone: Vec<f32> = crate::blocks::Tone::new(500.0, 8000.0, 0.5).real(4000);
        std::fs::write(songs.join("b_second.wav"), crate::audio::wav_file(8000, &tone)).unwrap();
        std::fs::write(songs.join("a_first.wav"), crate::audio::wav_file(8000, &tone)).unwrap();
        std::fs::write(songs.join("notes.md"), "not audio").unwrap();
        let d = songs.to_string_lossy().replace('\\', "/");
        let found = expand_sources(&[d.clone()]).unwrap();
        assert_eq!(found, vec![format!("{d}/a_first.wav"), format!("{d}/b_second.wav")]);
        let m3u = dir.join("list.m3u");
        std::fs::write(&m3u, "#EXTM3U\nsongs/b_second.wav\nsongs/missing.wav\nsongs/a_first.wav\n").unwrap();
        let m = m3u.to_string_lossy().replace('\\', "/");
        let iq = dir.join("out.cf32");
        let args = cli::parse_station(&a(&format!("{m} 9.58M --mode am --rate 24000 --gap 0.5 --iq {}", iq.display()))).unwrap();
        station(&args).unwrap();
        let (x, rate, _) = load_iq(&iq.to_string_lossy(), None, None).unwrap();
        assert_eq!(rate, 24_000.0);
        // Two 0.5 s songs, each followed by a 0.5 s gap: 2 s.
        assert!((x.len() as i64 - 48_000).abs() < 200, "{}", x.len());
        // AM: a carrier is on the air the whole time (songs and gaps), at a sensible level.
        for (i, c) in x.chunks(2_400).enumerate() {
            let p = c.iter().map(|v| v.norm_sqr()).sum::<f32>() / c.len() as f32;
            assert!(p > 0.05 && p < 20.0, "chunk {i}: mean power {p}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `radio_station --iq` writes what an SDR would transmit: an FM signal
    /// that `rx`'s receiver chain turns back into the audio file's tone, and a
    /// SigMF meta file `iqplay` can load.
    #[test]
    fn radio_station_iq_output_demodulates_to_the_tone() {
        let dir = std::env::temp_dir().join(format!("ecm-radio-station-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("tone.wav");
        let iq = dir.join("station.cf32");
        let tone: Vec<f32> = crate::blocks::Tone::new(800.0, 8000.0, 0.5).real(8000);
        std::fs::write(&wav, crate::audio::wav_file(8000, &tone)).unwrap();
        let args = cli::parse_station(&a(&format!("{} 100.1M --gap 0 --iq {}", wav.display(), iq.display()))).unwrap();
        station(&args).unwrap();
        let (x, rate, freq) = load_iq(&iq.to_string_lossy(), None, None).unwrap();
        assert_eq!((rate, freq), (48_000.0, 100.1e6));
        assert!((x.len() as i64 - 48_000).abs() < 100, "{}", x.len());
        let mut rx = receiver("fm", 48_000.0, 8_000.0, None).unwrap();
        let Buf::R(audio) = rx.process(Buf::C(x)).unwrap() else { panic!() };
        let s = &audio[1000..7000];
        let g = |f: f64| {
            let w = std::f64::consts::TAU * f / 8000.0;
            let (mut re, mut im) = (0.0, 0.0);
            for (n, &v) in s.iter().enumerate() {
                re += v as f64 * (w * n as f64).cos();
                im += v as f64 * (w * n as f64).sin();
            }
            re * re + im * im
        };
        assert!(g(800.0) > 100.0 * g(1600.0), "{} vs {}", g(800.0), g(1600.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn afsk1200_send_and_recv_through_iq_files() {
        let dir = std::env::temp_dir().join(format!("ecm-radio-afsk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let iq = dir.join("pkt.cf32");
        let send = cli::parse_afsk(&a(&format!("send 144.39M N0CALL-1 APRS hello world --repeat 2 --iq {}", iq.display()))).unwrap();
        afsk_send(&send).unwrap();
        let recv = cli::parse_afsk(&a(&format!("recv 144.39M --iq {}", iq.display()))).unwrap();
        let mut lines = Vec::new();
        let n = afsk_recv(&recv, &mut |l| lines.push(l)).unwrap();
        assert_eq!(n, 2, "{lines:?}");
        assert_eq!(lines[0], "N0CALL-1>APRS:hello world");
        let meta = Meta::parse(&std::fs::read_to_string(dir.join("pkt.sigmf-meta")).unwrap()).unwrap();
        assert_eq!(meta.frequency, 144.39e6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tx_tone_iq_has_the_carrier_at_the_offset() {
        let dir = std::env::temp_dir().join(format!("ecm-radio-tone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let iq = dir.join("tone.cf32");
        tx_tone(&cli::parse_tx_tone(&a(&format!("433.92M --offset 5k --seconds 0.5 --iq {}", iq.display()))).unwrap()).unwrap();
        let (x, rate, freq) = load_iq(&iq.to_string_lossy(), None, None).unwrap();
        assert_eq!(x.len(), 24_000);
        let hits = find_signals(&x, rate, freq, 1024, 20.0);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!((hits[0].freq - 433.925e6).abs() < 100.0);
        // FM variant: still centred at the offset, but spread by the deviation
        let iq2 = dir.join("fm.cf32");
        tx_tone(&cli::parse_tx_tone(&a(&format!("433.92M --fm 1000 --seconds 0.5 --iq {}", iq2.display()))).unwrap()).unwrap();
        let (y, _, _) = load_iq(&iq2.to_string_lossy(), None, None).unwrap();
        let Buf::R(audio) = receiver("fm", 48_000.0, 8_000.0, None).unwrap().process(Buf::C(y)).unwrap() else { panic!() };
        assert!(audio[1000..].iter().fold(0.0f32, |m, v| m.max(v.abs())) > 0.3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
