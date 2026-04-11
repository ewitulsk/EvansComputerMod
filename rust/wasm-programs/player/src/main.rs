//! `player` — plays an MP4 video file through the terminal's graphics
//! framebuffer. H.264 decoding happens on the host (Java + bytedeco FFmpeg);
//! this program just drives the decode loop and paces presentation against
//! wall-clock time.

use ecm_host_abi::{gfx_child, video};
use std::thread;
use std::time::Duration;

#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
}

fn now_ms() -> i64 {
    unsafe { get_time_ms() }
}

fn print_usage() {
    eprintln!("usage: player <video.mp4> [--size WxH]");
    eprintln!("  WxH defaults to 320x200");
}

fn parse_size(s: &str) -> Option<(i32, i32)> {
    let mut parts = s.split('x');
    let w: i32 = parts.next()?.parse().ok()?;
    let h: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
        return None;
    }
    Some((w, h))
}

struct Args {
    path: String,
    width: i32,
    height: i32,
}

fn parse_args() -> Result<Args, ()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        print_usage();
        return Err(());
    }

    let mut path: Option<String> = None;
    let mut size: (i32, i32) = (320, 200);

    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--size" => {
                i += 1;
                if i >= argv.len() {
                    eprintln!("player: --size requires an argument");
                    return Err(());
                }
                match parse_size(&argv[i]) {
                    Some(wh) => size = wh,
                    None => {
                        eprintln!("player: invalid size '{}'", argv[i]);
                        return Err(());
                    }
                }
            }
            "-h" | "--help" => {
                print_usage();
                return Err(());
            }
            other if other.starts_with('-') => {
                eprintln!("player: unknown option '{}'", other);
                return Err(());
            }
            other => {
                if path.is_some() {
                    eprintln!("player: unexpected argument '{}'", other);
                    return Err(());
                }
                path = Some(other.to_string());
            }
        }
        i += 1;
    }

    match path {
        Some(p) => Ok(Args { path: p, width: size.0, height: size.1 }),
        None => { print_usage(); Err(()) }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(()) => std::process::exit(1),
    };

    let handle = match video::open(&args.path, args.width, args.height) {
        Ok(h) => h,
        Err(()) => {
            eprintln!("player: failed to open '{}'", args.path);
            eprintln!("  hint: path is resolved against the computer root (no cwd).");
            eprintln!("  check the server log for 'bridgeVideoOpen' for the full reason.");
            std::process::exit(1);
        }
    };

    let info = match video::get_info(handle) {
        Ok(i) => i,
        Err(()) => {
            eprintln!("player: failed to read video metadata");
            let _ = video::close(handle);
            std::process::exit(1);
        }
    };

    let fps = if info.fps_den > 0 {
        info.fps_num as f64 / info.fps_den as f64
    } else {
        0.0
    };
    println!(
        "playing {} ({}x{} -> {}x{}, {:.2} fps, {} ms)",
        args.path, info.width, info.height, args.width, args.height,
        fps, info.duration_ms,
    );

    if gfx_child::init(args.width, args.height).is_err() {
        eprintln!("player: gfx_init failed");
        let _ = video::close(handle);
        std::process::exit(1);
    }
    let _ = gfx_child::set_mode(1);

    let start_wall = now_ms();
    let mut frames_decoded: u64 = 0;
    let mut frames_presented: u64 = 0;
    let mut last_pts_ms: i64 = 0;

    // Budget before we consider the player "behind" and skip sleeping.
    // Matches the host's 33 ms gfx sync cap rounded up for jitter tolerance.
    const CATCH_UP_BUDGET_MS: i64 = 50;

    loop {
        match video::decode_to_gfx(handle) {
            video::DecodeStep::Frame(pts) => {
                frames_decoded += 1;
                frames_presented += 1;
                last_pts_ms = pts;

                let target_wall = start_wall + pts;
                let now = now_ms();
                let delta = target_wall - now;

                if delta > 0 {
                    // Ahead of schedule — sleep until the frame's presentation
                    // window opens.
                    let ms = core::cmp::min(delta, 1_000) as u64;
                    thread::sleep(Duration::from_millis(ms));
                } else if delta < -CATCH_UP_BUDGET_MS {
                    // Behind schedule. We've already pushed this frame (the
                    // host-side decode_to_gfx did the blit), but don't sleep;
                    // the next iteration immediately requests the next frame
                    // so we can catch up. The host sync cap throttles what
                    // actually reaches the client, so pushing faster is
                    // harmless.
                }
            }
            video::DecodeStep::Eof => break,
            video::DecodeStep::Error => {
                eprintln!("player: decode error, stopping");
                break;
            }
        }
    }

    // Clean up: return the display to text mode so the shell is visible again.
    let _ = gfx_child::set_mode(0);
    let _ = video::close(handle);

    let wall_elapsed = now_ms() - start_wall;
    println!(
        "done: {} frames, last pts {} ms, wall time {} ms",
        frames_decoded, last_pts_ms, wall_elapsed,
    );
    let _ = frames_presented; // unused in v1; reserved for skip accounting
}
