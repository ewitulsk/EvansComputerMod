//! `scan` — step across a band and log the frequencies with activity.
//!
//!   scan <start> <stop> [--threshold DB] [--dwell MS] [--passes N] [--gain DB]
//!        [--sdr NAME] [--rate SPS] [--log FILE]

fn main() {
    ecm_radio::run::scan_main();
}
