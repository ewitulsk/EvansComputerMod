//! `iqplay` — transmit a recorded IQ file; rate and frequency come from its
//! .sigmf-meta unless given.
//!
//!   iqplay <file> [freq] [--rate SPS] [--power DBM] [--loop] [--sdr NAME]

fn main() {
    ecm_radio::run::iqplay_main();
}
