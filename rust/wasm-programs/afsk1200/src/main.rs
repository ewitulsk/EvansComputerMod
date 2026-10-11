//! `afsk1200` — 1200-baud AX.25 packet modem (Bell 202 AFSK over NBFM, APRS style).
//!
//!   afsk1200 send <freq> <SRC> <DEST> <text...> [--repeat N] [--power DBM]
//!   afsk1200 recv <freq> [--seconds S] [--count N]
//!   both: [--sdr NAME] [--rate SPS] [--iq FILE]  (--iq: write/read a .cf32 instead of the SDR)

fn main() {
    ecm_radio::run::afsk_main();
}
