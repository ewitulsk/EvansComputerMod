//! `beep` — play a tone on a speaker.
//!
//! Usage: beep [freq_hz] [ms] [--wave square|sine|triangle|saw] [--volume 0-100] [--speaker SIDE]
//!
//! Defaults: 440 Hz, 200 ms, square wave, the first speaker.

use std::f64::consts::PI;

const RATE: u32 = 48000;

fn usage() -> ! {
    eprintln!("usage: beep [freq_hz] [ms] [--wave square|sine|triangle|saw] [--volume 0-100] [--speaker SIDE]");
    std::process::exit(1);
}

fn main() {
    let mut freq = 440.0f64;
    let mut ms = 200u32;
    let mut wave = String::from("square");
    let mut volume: Option<u8> = None;
    let mut side: Option<String> = None;
    let mut positional = 0;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--wave" => wave = args.next().unwrap_or_else(|| usage()),
            "--volume" => volume = Some(args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())),
            "--speaker" => side = Some(args.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            v => {
                match positional {
                    0 => freq = v.parse().unwrap_or_else(|_| usage()),
                    1 => ms = v.parse().unwrap_or_else(|_| usage()),
                    _ => usage(),
                }
                positional += 1;
            }
        }
    }
    if !(20.0..=20000.0).contains(&freq) || ms == 0 || ms > 60_000 {
        usage();
    }

    let mut dev = match ecm_audio::Device::open(side.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("beep: no speaker{}: {}", side.map(|s| format!(" on {}", s)).unwrap_or_default(), e);
            eprintln!("  place a Speaker next to the computer.");
            std::process::exit(1);
        }
    };
    if let Err(e) = dev.set_format(RATE, 16, 1) {
        eprintln!("beep: {}: {}", dev.name(), e);
        std::process::exit(1);
    }
    if let Some(v) = volume {
        let _ = dev.set_volume(v.min(100));
    }

    let n = (RATE as u64 * ms as u64 / 1000) as usize;
    let amp = 12000.0;
    let fade = (RATE / 200) as usize; // 5 ms ramps, no clicks
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            let phase = (i as f64 * freq / RATE as f64).fract();
            let v = match wave.as_str() {
                "sine" => (2.0 * PI * phase).sin(),
                "triangle" => 1.0 - 4.0 * (phase - 0.5).abs(),
                "saw" => 2.0 * phase - 1.0,
                _ => if phase < 0.5 { 1.0 } else { -1.0 },
            };
            let env = (i.min(n - 1 - i) as f64 / fade as f64).min(1.0);
            (v * amp * env) as i16
        })
        .collect();
    if let Err(e) = dev.write_samples(&samples) {
        eprintln!("beep: {}", e);
        std::process::exit(1);
    }
}
