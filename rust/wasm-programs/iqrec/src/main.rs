//! `iqrec` — record IQ to a .cf32 / .cs16 file with SigMF metadata
//! (.sigmf-meta), readable by inspectrum, GQRX and Universal Radio Hacker.
//!
//!   iqrec <freq> <file.cf32|file.cs16> [--seconds S] [--rate SPS] [--gain DB] [--sdr NAME]

fn main() {
    ecm_radio::run::iqrec_main();
}
