//! `arecord` — record from a microphone into a WAV file.
//!
//! Usage:
//!   arecord FILE.wav [-d SECONDS] [-r RATE] [-b 8|16] [-c 1|2] [--mic SIDE] [--gain 0-100]
//!   arecord --level [-d SECONDS] [--mic SIDE]      (a live level meter, nothing saved)
//!
//! Defaults: 16000 Hz, 16-bit, mono, until killed (Ctrl-C) unless `-d` is given.
//! The file is a valid WAV after every block, so stopping early keeps what was
//! recorded. At the end it prints the length, the peak level and any overruns.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

fn usage() -> ! {
    eprintln!("usage: arecord FILE.wav [-d SECONDS] [-r RATE] [-b 8|16] [-c 1|2] [--mic SIDE] [--gain 0-100]");
    eprintln!("       arecord --level [-d SECONDS] [--mic SIDE]");
    std::process::exit(1);
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("arecord: {}", msg);
    std::process::exit(1);
}

fn bar(db: i32) -> String {
    let n = ((db + 60).clamp(0, 60) / 3) as usize;
    format!("{}{}", "#".repeat(n), ".".repeat(20 - n))
}

fn main() {
    let mut path: Option<String> = None;
    let (mut rate, mut bits, mut channels) = (16000u32, 16u8, 1u8);
    let mut seconds: Option<f64> = None;
    let mut side: Option<String> = None;
    let mut gain: Option<u8> = None;
    let mut level = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "-d" | "--duration" => seconds = Some(val().parse().unwrap_or_else(|_| usage())),
            "-r" | "--rate" => rate = val().parse().unwrap_or_else(|_| usage()),
            "-b" | "--bits" => bits = val().parse().unwrap_or_else(|_| usage()),
            "-c" | "--channels" => channels = val().parse().unwrap_or_else(|_| usage()),
            "--mic" => side = Some(val()),
            "--gain" => gain = Some(val().parse::<u8>().unwrap_or_else(|_| usage()).min(100)),
            "--level" => level = true,
            "-h" | "--help" => usage(),
            f if path.is_none() && !f.starts_with('-') => path = Some(f.to_string()),
            _ => usage(),
        }
    }
    if path.is_none() && !level {
        usage();
    }
    if level {
        // The meter reads 16-bit mono whatever was asked.
        bits = 16;
        channels = 1;
    }

    let mut mic = ecm_audio::Mic::open(side.as_deref()).unwrap_or_else(|e| fail(format!("no microphone: {}", e)));
    if let Err(e) = mic.set_format(rate, bits, channels) {
        fail(format!("{} can't record {} Hz, {}-bit, {} channel(s): {}", mic.name(), rate, bits, channels, e));
    }
    if let Some(g) = gain {
        if let Err(e) = mic.set_gain(g) {
            fail(e);
        }
    }
    let frame = (bits as usize / 8) * channels as usize;
    let block_frames = (rate / 10) as usize; // 100 ms
    let total_frames = seconds.map(|s| (s * rate as f64).round() as usize);

    let mut file = path.as_ref().map(|p| {
        let mut f = File::create(p).unwrap_or_else(|e| fail(format!("{}: {}", p, e)));
        f.write_all(&ecm_audio::wav_header(rate, bits, channels, 0)).unwrap_or_else(|e| fail(e));
        f
    });
    if let Some(p) = &path {
        println!("arecord: {} -> {} ({} Hz, {}-bit, {} ch{})", mic.name(), p, rate, bits, channels,
                 seconds.map(|s| format!(", {:.1} s", s)).unwrap_or_default());
    } else {
        println!("arecord: level of {} (dBFS)", mic.name());
    }
    let _ = std::io::stdout().flush();

    let mut got = 0usize;
    let mut peak = -99;
    let mut buf = vec![0u8; block_frames * frame];
    loop {
        let want = match total_frames {
            Some(t) if got >= t => break,
            Some(t) => (t - got).min(block_frames),
            None => block_frames,
        };
        let n = mic.read_bytes(&mut buf[..want * frame]).unwrap_or_else(|e| fail(e));
        if n == 0 {
            break;
        }
        let data = &buf[..n];
        let samples: Vec<i16> = if bits == 16 {
            data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
        } else {
            data.iter().map(|b| ((*b as i16) - 128) << 8).collect()
        };
        let db = ecm_audio::peak_dbfs(&samples);
        peak = peak.max(db);
        got += n / frame;
        if let Some(f) = file.as_mut() {
            let ok = f.write_all(data).is_ok()
                && f.seek(SeekFrom::Start(0)).is_ok()
                && f.write_all(&ecm_audio::wav_header(rate, bits, channels, (got * frame) as u32)).is_ok()
                && f.seek(SeekFrom::End(0)).is_ok();
            if !ok {
                fail("can't write the file");
            }
        } else {
            println!("{:>4} {}", db, bar(db));
            let _ = std::io::stdout().flush();
        }
    }
    let overruns = mic.stat("overruns").ok().flatten().unwrap_or(0);
    println!("arecord: recorded {:.1} s, peak {} dBFS, overruns {}", got as f64 / rate as f64, peak, overruns);
}
