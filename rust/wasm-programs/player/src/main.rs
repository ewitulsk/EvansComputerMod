//! `player` — plays an MP4 video file through one of the computer's
//! graphics framebuffers. H.264 decoding happens on the host (Java +
//! bytedeco FFmpeg); this program just picks a target, drives the
//! decode loop, and paces presentation against wall-clock time.
//!
//! Usage:
//!   player <video.mp4> [--size WxH]     output to the terminal (default 320x200)
//!   player screen <video.mp4>           output to the attached in-world Screen cluster

use ecm_host_abi::video::{PixelFormat, Target};
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
    eprintln!("       player screen <video.mp4>");
    eprintln!("  WxH defaults to 320x200 for the terminal target.");
    eprintln!("  In screen mode the cluster's native pixel size is used.");
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
    target: Target,
    path: String,
    /// Only populated for [`Target::Terminal`]; screen mode sizes itself
    /// from the cluster.
    terminal_size: (i32, i32),
}

fn parse_args() -> Result<Args, ()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        print_usage();
        return Err(());
    }

    // The first positional either is the literal word `screen` (which
    // selects the screen target and pushes the path to argv[2]), or it
    // is the video path (in which case the target defaults to the
    // terminal's built-in gfx framebuffer).
    let target = if argv.get(1).map(|s| s.as_str()) == Some("screen") {
        Target::Screen
    } else {
        Target::Terminal
    };

    let positional_start = match target {
        Target::Screen => 2,
        Target::Terminal => 1,
    };

    let mut path: Option<String> = None;
    let mut terminal_size: (i32, i32) = (320, 200);

    let mut i = positional_start;
    while i < argv.len() {
        match argv[i].as_str() {
            "--size" => {
                if target == Target::Screen {
                    eprintln!("player: --size is not allowed in screen mode");
                    return Err(());
                }
                i += 1;
                if i >= argv.len() {
                    eprintln!("player: --size requires an argument");
                    return Err(());
                }
                match parse_size(&argv[i]) {
                    Some(wh) => terminal_size = wh,
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
        Some(p) => Ok(Args { target, path: p, terminal_size }),
        None => { print_usage(); Err(()) }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(()) => std::process::exit(1),
    };

    // Determine render size. Terminal uses its CLI default; screen uses
    // the attached cluster's native dimensions.
    //
    // For the screen target we also power the cluster on here and
    // switch it into RGBA8888 so the player runs in full color. The
    // terminal target stays on the indexed RGB332 path — it's a small
    // 320×200 plane that doesn't need the bandwidth of a full-color
    // framebuffer.
    let (width, height, pixel_format) = match args.target {
        Target::Terminal => {
            let (w, h) = args.terminal_size;
            (w, h, PixelFormat::Indexed8)
        }
        Target::Screen => {
            gfx_child::set_screen_power(true);
            let (w, h) = match gfx_child::screen_dims() {
                Some((w, h)) => (w as i32, h as i32),
                None => {
                    eprintln!("player: no screen cluster attached");
                    std::process::exit(1);
                }
            };
            // Switch the screen into RGBA8888 mode so subsequent
            // video_decode_to_gfx calls are routed through the rgba
            // staging path. `set_pixel_format_host` zeros the pixel
            // region for the new format on the host side.
            gfx_child::set_screen_pixel_format(1);
            (w, h, PixelFormat::Rgba8888)
        }
    };

    let handle = match video::open(&args.path, width, height, pixel_format) {
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
    let target_name = match args.target {
        Target::Terminal => "terminal",
        Target::Screen => "screen",
    };
    println!(
        "playing {} -> {} ({}x{} -> {}x{}, {:.2} fps, {} ms)",
        args.path, target_name,
        info.width, info.height, width, height,
        fps, info.duration_ms,
    );

    if gfx_child::init(args.target, width, height).is_err() {
        eprintln!("player: gfx_init failed for target {}", target_name);
        let _ = video::close(handle);
        std::process::exit(1);
    }
    let _ = gfx_child::set_mode(args.target, 1);

    let start_wall = now_ms();
    let mut frames_decoded: u64 = 0;
    let mut last_pts_ms: i64 = 0;

    // Budget before we consider the player "behind" and skip sleeping.
    // Matches the host's 33 ms gfx sync cap rounded up for jitter tolerance.
    const CATCH_UP_BUDGET_MS: i64 = 50;

    loop {
        match video::decode_to_gfx(handle, args.target) {
            video::DecodeStep::Frame(pts) => {
                frames_decoded += 1;
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

    // Terminal target: return to text mode so the shell is visible again.
    // Screen target: reset the format back to indexed8 so the next program
    // starts on a clean cluster. The pixel-region clear that comes with
    // the format switch wipes the final frame too — acceptable for v1.
    if args.target == Target::Terminal {
        let _ = gfx_child::set_mode(args.target, 0);
    } else {
        gfx_child::set_screen_pixel_format(0);
    }
    let _ = video::close(handle);

    let wall_elapsed = now_ms() - start_wall;
    println!(
        "done: {} frames, last pts {} ms, wall time {} ms",
        frames_decoded, last_pts_ms, wall_elapsed,
    );
}
