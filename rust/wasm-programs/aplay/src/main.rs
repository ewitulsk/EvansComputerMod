//! `aplay` — play a sound file on a speaker.
//!
//! Usage:
//!   aplay FILE.wav [--speaker SIDE] [--volume 0-100]
//!   aplay FILE.raw --rate N [--bits 8|16] [--channels 1|2] [--speaker SIDE]
//!
//! WAV files (PCM, 8/16-bit, mono/stereo) carry their own format; raw files
//! are headerless PCM (default 48000 Hz, 16-bit signed little-endian, mono).
//! Playback is paced by the speaker: writes wait while its queue is full.

fn usage() -> ! {
    eprintln!("usage: aplay FILE.wav [--speaker SIDE] [--volume 0-100]");
    eprintln!("       aplay FILE.raw --rate N [--bits 8|16] [--channels 1|2] [--speaker SIDE]");
    std::process::exit(1);
}

fn main() {
    let mut path: Option<String> = None;
    let (mut rate, mut bits, mut channels) = (None::<u32>, None::<u8>, None::<u8>);
    let mut side: Option<String> = None;
    let mut volume: Option<u8> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut num = || args.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or_else(|| usage());
        match a.as_str() {
            "--rate" => rate = Some(num()),
            "--bits" => bits = Some(num() as u8),
            "--channels" => channels = Some(num() as u8),
            "--volume" => volume = Some(num().min(100) as u8),
            "--speaker" => side = Some(args.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            f if path.is_none() => path = Some(f.to_string()),
            _ => usage(),
        }
    }
    let path = path.unwrap_or_else(|| usage());
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("aplay: {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let (r, b, c, data) = match ecm_audio::parse_wav(&bytes) {
        Some(w) => (w.rate, w.bits, w.channels, &bytes[w.data_offset..w.data_offset + w.data_len]),
        None => (rate.unwrap_or(48000), bits.unwrap_or(16), channels.unwrap_or(1), &bytes[..]),
    };

    let mut dev = match ecm_audio::Device::open(side.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("aplay: no speaker: {}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = dev.set_format(r, b, c) {
        eprintln!("aplay: {} can't play {} Hz, {}-bit, {} channel(s): {}", dev.name(), r, b, c, e);
        std::process::exit(1);
    }
    if let Some(v) = volume {
        let _ = dev.set_volume(v);
    }
    println!("aplay: {} -> {} ({} Hz, {}-bit, {} ch, {:.1} s)", path, dev.name(), r, b, c,
             data.len() as f64 / (r as f64 * (b as f64 / 8.0) * c as f64));
    for chunk in data.chunks(4096) {
        if let Err(e) = dev.write_bytes(chunk) {
            eprintln!("aplay: {}", e);
            std::process::exit(1);
        }
    }
}
