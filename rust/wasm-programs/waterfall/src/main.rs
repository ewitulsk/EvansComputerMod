//! `waterfall` — live spectrum and waterfall of an SDR.
//!
//!   waterfall <freq> [--rate SPS] [--gain DB] [--sdr NAME] [--min DB] [--max DB]
//!             [--size WxH] [--seconds S] [--screen | --text]
//!
//! Draws on the terminal's graphics display (default 256x160), on the
//! attached Screen cluster (`--screen`, at its native size), or as text
//! lines (`--text`, one row of shade characters per tenth of a second).
//! The trace on top is the latest spectrum; rows scroll down below it.

use ecm_host_abi::gfx_child::{self, FORMAT_INDEXED8};
use ecm_host_abi::video::Target;
use ecm_radio::cli::{parse_waterfall, WaterfallArgs, WATERFALL_USAGE};
use ecm_radio::run::{args, fail, setup_rx};
use ecm_radio::spectrum::{heat_palette, WaterfallView};
use ecm_radio::units::fmt_freq;

fn nfft_for(width: usize) -> usize {
    width.next_power_of_two().clamp(64, 2048)
}

fn main() {
    let a = parse_waterfall(&args()).unwrap_or_else(|e| fail("waterfall", WATERFALL_USAGE, e));
    if let Err(e) = run(&a) {
        eprintln!("waterfall: {e}");
        std::process::exit(1);
    }
}

fn run(a: &WaterfallArgs) -> Result<(), String> {
    let mut sdr = setup_rx(&a.sdr, a.freq, a.rate, a.gain_db)?;
    let total = a.seconds.map(|s| (s * a.rate as f64) as u64);
    let per_row = (a.rate as usize / 10).max(64);
    let span = a.rate as f64 / 2.0;
    println!(
        "waterfall: {} .. {} (centre {}), {} S/s  [Ctrl+T stops]",
        fmt_freq(a.freq - span),
        fmt_freq(a.freq + span),
        fmt_freq(a.freq),
        a.rate
    );

    if a.text {
        let width = a.width.min(78);
        let mut view = WaterfallView::new(width, 16, nfft_for(width), a.min_db, a.max_db);
        let mut done = 0u64;
        while total.map_or(true, |t| done < t) {
            let x = sdr.read(per_row).map_err(|e| format!("receive: {e}"))?;
            if x.is_empty() {
                continue;
            }
            done += x.len() as u64;
            let tail = &x[x.len().saturating_sub(view.nfft())..];
            view.push(tail);
            let (col, db) = view.peak();
            let f = a.freq - span + (col as f64 + 0.5) * a.rate as f64 / width as f64;
            println!("|{}| {:5.0} dB @ {}", view.text_row(), db, fmt_freq(f));
        }
        return Ok(());
    }

    let target = if a.screen { Target::Screen } else { Target::Terminal };
    let (w, h) = if a.screen {
        gfx_child::set_screen_power(true);
        let (w, h) = gfx_child::screen_dims().ok_or("no Screen attached to this computer (use --text or the terminal display)")?;
        (w as usize, h as usize)
    } else {
        (a.width, a.height)
    };
    gfx_child::init2(target, w as i32, h as i32, FORMAT_INDEXED8, 0).map_err(|e| format!("can't open the display: {e:?}"))?;
    let _ = gfx_child::set_palette(target, 0, &heat_palette());
    let mut view = WaterfallView::new(w, h, nfft_for(w), a.min_db, a.max_db);
    let mut done = 0u64;
    let mut rows = 0u64;
    let result = (|| -> Result<(), String> {
        while total.map_or(true, |t| done < t) {
            let x = sdr.read(per_row).map_err(|e| format!("receive: {e}"))?;
            if x.is_empty() {
                continue;
            }
            done += x.len() as u64;
            view.push(&x[x.len().saturating_sub(view.nfft())..]);
            let fb = view.render();
            gfx_child::blit_rect(target, 0, 0, w as i32, h as i32, &fb, FORMAT_INDEXED8).map_err(|_| "display lost".to_string())?;
            rows += 1;
        }
        Ok(())
    })();
    let _ = gfx_child::set_mode(target, 0);
    let (col, db) = view.peak();
    let f = a.freq - span + (col as f64 + 0.5) * a.rate as f64 / w as f64;
    println!("waterfall: {rows} rows; strongest {:.0} dB at {}", db, fmt_freq(f));
    result
}
