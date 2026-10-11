//! `radio_station` — broadcast audio files (WAV, or raw 16-bit PCM) as AM or
//! FM, e.g. for handheld receivers. Several files, playlists (`.m3u`/`.txt`,
//! one path per line) and directories play in order, printing
//! "now playing: <title>" for each song (the WAV's INFO title, else its
//! file name); `--loop` repeats the whole list.
//!
//!   radio_station <file|playlist|dir>... <freq> [--mode fm|am|wbfm] [--power DBM]
//!                 [--loop] [--gap S] [--sdr NAME] [--rate SPS] [--raw-rate HZ] [--iq FILE]

fn main() {
    ecm_radio::run::station_main();
}
