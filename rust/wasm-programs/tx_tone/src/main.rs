//! `tx_tone` — transmit a test carrier (or an FM tone) for a few seconds, e.g.
//! to tune an antenna or check a receiver.
//!
//!   tx_tone <freq> [--offset HZ] [--fm AUDIO_HZ] [--seconds S] [--power DBM]
//!           [--sdr NAME] [--rate SPS] [--iq FILE]

fn main() {
    ecm_radio::run::tx_tone_main();
}
