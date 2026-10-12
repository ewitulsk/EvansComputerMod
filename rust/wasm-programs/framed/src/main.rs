//! `framed` - the SDR's framed PHY mode from the shell: set up LTE / mmWave framed mode,
//! send frames (now, at a slot, with HARQ), receive them with RSSI / SINR / CQI, run beacons.
//!
//!   framed [--sdr NAME] status | config ... | send <dst|bcast> <text...> | recv | beacon <ms|off> [text]
//!
//! See docs/radio/SDR_PROGRAMS.md "Framed PHY mode".

fn main() {
    ecm_radio::framed::main();
}
