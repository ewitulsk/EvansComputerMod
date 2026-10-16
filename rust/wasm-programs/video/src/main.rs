//! `video` — video files from the command line (host-side FFmpeg decoding;
//! abi/child-abi.toml `[imports.video]`).
//!
//!   video info FILE                  size, frame rate, length and sound of a video
//!   video ls [DIR...]                the videos in DIRs (default /home and /home/videos)
//!   video play FILE [options]        play it on the Terminal's screen, with sound
//!       --screen        on the attached Screen wall instead (at the wall's size)
//!       --size WxH      the box the picture is fitted into (Terminal; default 320x200)
//!       --start SECONDS start there
//!       --loop          start again at the end (Ctrl+T stops)
//!       --mute          no sound
//!
//! Sound goes to the computer's speaker (`/dev/audio`) when there is one.
//! The GUI has the same player as an app: Video Player (`videoplayer`).

use ecm_host_abi::gfx_child;
use ecm_host_abi::video::{self, DecodeStep, PixelFormat, Target};
use std::time::{Duration, Instant};
use videoplayer::{find_videos, fit, format_size, format_time, AudioPump, AudioSink, AudioSource, Clock, LIBRARY_DIRS};

const RATE: u32 = 22050;

fn usage() -> ! {
    eprintln!("usage: video info FILE");
    eprintln!("       video ls [DIR...]");
    eprintln!("       video play FILE [--screen] [--size WxH] [--start SECONDS] [--loop] [--mute]");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(|s| s.as_str()) {
        Some("info") if args.len() == 2 => info(&args[1]),
        Some("ls") => ls(&args[1..]),
        Some("play") if args.len() >= 2 => play(&args[1..]),
        Some("-h") | Some("--help") | Some("help") => {
            usage();
        }
        _ => usage(),
    };
    std::process::exit(code);
}

/// The file's metadata and sound format (a tiny decoder is opened and closed).
fn probe(path: &str) -> Result<(video::VideoInfo, Option<(u32, u8)>), String> {
    let h = video::open(path, 16, 16, PixelFormat::Indexed8).map_err(|_| format!("{path}: not found, or not a video the host can decode"))?;
    let info = video::get_info(h);
    let sound = video::audio_source(h);
    let _ = video::close(h);
    let info = info.map_err(|_| format!("{path}: can't read its details"))?;
    Ok((info, sound))
}

fn channels_name(c: u8) -> String {
    match c {
        1 => "mono".into(),
        2 => "stereo".into(),
        n => format!("{n} channels"),
    }
}

fn info(path: &str) -> i32 {
    let (i, sound) = match probe(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("video: {e}");
            return 1;
        }
    };
    println!("file:     {path}");
    println!("size:     {}x{}", i.width, i.height);
    println!("fps:      {:.2}", i.fps());
    if i.frame_count > 0 {
        println!("frames:   {}", i.frame_count);
    }
    println!("duration: {} ({} ms)", format_time(i.duration_ms as i64), i.duration_ms);
    match sound {
        Some((rate, ch)) => println!("sound:    {rate} Hz, {}", channels_name(ch)),
        None => println!("sound:    none"),
    }
    0
}

fn ls(dirs: &[String]) -> i32 {
    let dirs: Vec<&str> = if dirs.is_empty() { LIBRARY_DIRS.to_vec() } else { dirs.iter().map(|s| s.trim_end_matches('/')).collect() };
    let videos = find_videos(&dirs);
    if videos.is_empty() {
        println!("no videos in {}", dirs.join(", "));
        return 0;
    }
    for e in &videos {
        let detail = match probe(&e.path) {
            Ok((i, s)) => format!("{:>7}  {}x{}  {}", format_time(i.duration_ms as i64), i.width, i.height, if s.is_some() { "sound" } else { "no sound" }),
            Err(_) => "  can't decode".to_string(),
        };
        println!("{:<40} {:>9} {}", e.path, format_size(e.bytes), detail);
    }
    0
}

fn parse_size(s: &str) -> Option<(i32, i32)> {
    let (w, h) = s.split_once('x')?;
    let (w, h): (i32, i32) = (w.parse().ok()?, h.parse().ok()?);
    if (2..=4096).contains(&w) && (2..=4096).contains(&h) {
        Some((w, h))
    } else {
        None
    }
}

struct HostAudio(video::Handle);

impl AudioSource for HostAudio {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        video::read_audio(self.0, buf).unwrap_or(0)
    }
}

struct Speaker(ecm_audio::Device);

impl AudioSink for Speaker {
    fn write(&mut self, pcm: &[u8]) {
        let _ = self.0.write_bytes(pcm);
    }
}

fn play(args: &[String]) -> i32 {
    let mut path: Option<String> = None;
    let mut target = Target::Terminal;
    let mut box_size = (320, 200);
    let mut start_ms: i64 = 0;
    let mut looping = false;
    let mut mute = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screen" => target = Target::Screen,
            "--size" => match it.next().and_then(|s| parse_size(s)) {
                Some(s) => box_size = s,
                None => {
                    eprintln!("video: --size takes WxH, e.g. 320x180");
                    return 2;
                }
            },
            "--start" => match it.next().and_then(|s| s.parse::<f64>().ok()) {
                Some(s) if s >= 0.0 => start_ms = (s * 1000.0) as i64,
                _ => {
                    eprintln!("video: --start takes seconds");
                    return 2;
                }
            },
            "--loop" => looping = true,
            "--mute" => mute = true,
            o if o.starts_with('-') => {
                eprintln!("video: unknown option {o}");
                return 2;
            }
            p if path.is_none() => path = Some(p.to_string()),
            _ => usage(),
        }
    }
    let Some(path) = path else { usage() };
    let (src_info, sound) = match probe(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("video: {e}");
            return 1;
        }
    };

    // The box: the Terminal's --size, or the Screen wall's own resolution.
    if target == Target::Screen {
        gfx_child::set_screen_power(true);
        match gfx_child::screen_dims() {
            Some((w, h)) => box_size = (w as i32, h as i32),
            None => {
                eprintln!("video: no Screen wall attached to this computer");
                return 1;
            }
        }
    }
    let pic = fit((0, 0, box_size.0, box_size.1), src_info.width as i32, src_info.height as i32);
    let (w, h) = ((pic.2 / 2 * 2).max(2), (pic.3 / 2 * 2).max(2));
    let handle = match video::open(&path, w, h, PixelFormat::Rgb565) {
        Ok(h) => h,
        Err(()) => {
            eprintln!("video: {path}: can't open it at {w}x{h}");
            return 1;
        }
    };
    if let Err(e) = gfx_child::init2(target, w, h, gfx_child::FORMAT_RGB565, 0) {
        eprintln!("video: can't take the {} display: {e:?}", if target == Target::Screen { "Screen" } else { "Terminal" });
        let _ = video::close(handle);
        return 1;
    }

    // Sound: the file's, on the first speaker.
    let mut audio: Option<(HostAudio, AudioPump, Speaker)> = None;
    let mut sound_note = "no sound".to_string();
    if sound.is_some() && !mute {
        match ecm_audio::Device::open(None) {
            Ok(mut d) => {
                if d.set_format(RATE, 16, 1).and_then(|_| d.set_latency_ms(400)).is_ok() && video::audio_open(handle, RATE, 1).is_ok() {
                    audio = Some((HostAudio(handle), AudioPump::new(RATE, 1), Speaker(d)));
                    sound_note = "sound".into();
                }
            }
            Err(_) => sound_note = "no speaker".into(),
        }
    } else if mute {
        sound_note = "muted".into();
    }

    let fps = src_info.fps();
    let frame_ms = if fps > 0.5 { (1000.0 / fps).round() as i64 } else { 33 };
    let duration = src_info.duration_ms as i64;
    println!(
        "playing {path} -> {} ({}x{} -> {w}x{h}, {fps:.2} fps, {}, {sound_note}){}",
        if target == Target::Screen { "screen" } else { "terminal" },
        src_info.width,
        src_info.height,
        format_time(duration),
        if looping { ", looping (Ctrl+T stops)" } else { "" }
    );

    let t0 = Instant::now();
    let now = || t0.elapsed().as_millis() as i64;
    let mut clock = Clock::new(now(), 0, true);
    let restart = |at: i64, clock: &mut Clock, audio: &mut Option<(HostAudio, AudioPump, Speaker)>| {
        let _ = video::seek(handle, at);
        clock.seek(now(), at);
        if let Some((_, pump, spk)) = audio.as_mut() {
            pump.reset(at);
            let _ = spk.0.flush_queue();
        }
    };
    if start_ms > 0 {
        restart(start_ms, &mut clock, &mut audio);
    }
    let (mut frames, mut late, mut loops) = (0u64, 0u64, 0u32);
    let mut next_due = start_ms;
    let mut failed = false;
    loop {
        // Until the next frame is due: keep the sound fed (20 ms steps).
        loop {
            let pos = clock.pos(now());
            if let Some((src, pump, spk)) = audio.as_mut() {
                pump.pump(pos, src, spk);
            }
            if pos >= next_due {
                break;
            }
            std::thread::sleep(Duration::from_millis((next_due - pos).clamp(1, 20) as u64));
        }
        match video::decode_to_gfx(handle, target) {
            DecodeStep::Frame(pts) => {
                frames += 1;
                let pos = clock.pos(now());
                if pos > pts + frame_ms * 2 {
                    late += 1;
                }
                next_due = pts + frame_ms;
            }
            DecodeStep::Eof if looping => {
                loops += 1;
                restart(0, &mut clock, &mut audio);
                next_due = 0;
            }
            DecodeStep::Eof => break,
            DecodeStep::Error => {
                // Ctrl+T (the host refuses a stopped program's calls) or a broken file.
                failed = true;
                break;
            }
        }
    }
    // Let the last of the sound play out.
    if !failed {
        if let Some((_, pump, _)) = audio.as_ref() {
            let left = pump.pos_ms() - clock.pos(now());
            if left > 0 {
                std::thread::sleep(Duration::from_millis(left.min(1000) as u64));
            }
        }
    }
    let _ = gfx_child::set_mode(target, 0);
    let _ = video::close(handle);
    println!(
        "done: {frames} frames ({late} late){}, {} played",
        if loops > 0 { format!(", {loops} loops") } else { String::new() },
        format_time(t0.elapsed().as_millis() as i64)
    );
    if failed {
        1
    } else {
        0
    }
}
