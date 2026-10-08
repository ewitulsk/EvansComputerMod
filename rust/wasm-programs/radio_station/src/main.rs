//! `radio_station` — broadcast an audio file (WAV, or raw 16-bit PCM) as AM or
//! FM, e.g. for handheld receivers.
//!
//!   radio_station <file> <freq> [--mode fm|am|wbfm] [--power DBM] [--loop]
//!                 [--sdr NAME] [--rate SPS] [--raw-rate HZ] [--iq FILE]

fn main() {
    ecm_radio::run::station_main();
}
