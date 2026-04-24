//! `browser` — embed a real Chromium instance inside the computer's
//! terminal graphics framebuffer.
//!
//! Pipeline:
//! 1. Switch the terminal display into graphics mode at [`W`]×[`H`] and
//!    select the RGBA8888 pixel format (CEF produces full-color frames).
//! 2. Enable mouse capture so the on-screen mouse cursor maps to
//!    framebuffer pixel coords while the terminal UI is open.
//! 3. Ask the host to start an off-screen CEF browser sized to match.
//! 4. Each tick, drain the mouse-capture ring and forward every event to
//!    the browser, then ask the host to stage the latest paint into the
//!    framebuffer. Sleep ~16 ms to cap the loop at ~60 Hz.
//!
//! Exit paths:
//! * Ctrl+T at the shell aborts the child — the next host call returns an
//!   error and we break out of the loop.
//! * If mouse capture gets disabled by the host (e.g. the terminal UI
//!   closes), we stop forwarding input and exit.
//!
//! Scope in v1: no keyboard forwarding, single browser at a time,
//! integrated-server only. See the browser plan for the deferred work.
//!
//! Usage: `browser [url]` (defaults to <https://example.com>).
//!
//! Controls:
//! * Move / click / scroll — passthrough to the page.
//! * Ctrl+T — terminate the child (standard shell reset path).

use ecm_host_abi::browser::Browser;
use ecm_host_abi::gfx_child::{self, FORMAT_RGBA8888};
use ecm_host_abi::mouse;
use ecm_host_abi::video::Target;
use std::env;
use std::thread;
use std::time::Duration;

/// Browser / framebuffer width in pixels.
const W: i32 = 640;
/// Browser / framebuffer height in pixels.
const H: i32 = 480;

fn main() {
    let url = env::args().nth(1).unwrap_or_else(|| "https://example.com".into());

    if gfx_child::init(Target::Terminal, W, H).is_err() {
        eprintln!("browser: gfx_init failed");
        std::process::exit(1);
    }
    if gfx_child::set_mode(Target::Terminal, 1).is_err() {
        eprintln!("browser: set_mode(gfx) failed");
        std::process::exit(1);
    }
    // RGBA frames from CEF require the display's pixel format to be
    // RGBA8888 first; otherwise the staging path rejects the frame.
    gfx_child::set_screen_pixel_format(FORMAT_RGBA8888);

    if !mouse::enable() {
        eprintln!("browser: mouse capture not available.");
        eprintln!("  open the terminal UI while the display is in gfx mode.");
        let _ = gfx_child::set_mode(Target::Terminal, 0);
        std::process::exit(1);
    }

    println!("browser: loading {}", url);
    println!("  drive the page with the mouse. Ctrl+T to exit.");

    let browser = match Browser::open(&url, W, H) {
        Some(b) => b,
        None => {
            eprintln!("browser: host refused to open CEF — is jcef installed?");
            mouse::disable();
            let _ = gfx_child::set_mode(Target::Terminal, 0);
            std::process::exit(1);
        }
    };

    loop {
        // Drain the mouse-capture ring and forward each event. The host
        // drop-oldest-on-overflow policy means this loop is bounded.
        while let Some(ev) = mouse::poll() {
            let _ = browser.send_mouse(&ev);
        }

        // Stage the latest paint if one's ready. Ok(false) just means CEF
        // hasn't produced a new frame since we last rendered — normal when
        // the page is idle, so we skip the blit and keep the loop cheap.
        match browser.render(Target::Terminal) {
            Ok(_had_frame) => {}
            Err(()) => break, // handle gone / abort requested
        }

        thread::sleep(Duration::from_millis(16));

        if !mouse::is_active() {
            break;
        }
    }

    mouse::disable();
    let _ = gfx_child::set_mode(Target::Terminal, 0);
    // Browser closes on drop.
}
