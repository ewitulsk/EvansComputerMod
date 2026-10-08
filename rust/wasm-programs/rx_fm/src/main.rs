//! `rx_fm` — listen to narrowband (or, with `--wide`, broadcast) FM through the Speaker.
//!
//!   rx_fm <freq> [--wide] [--squelch DB] [--gain DB] [--sdr NAME] [--rate SPS]
//!         [--speaker SIDE] [--volume 0-100] [--seconds S] [--wav FILE [--no-speaker]]

fn main() {
    ecm_radio::run::rx_main("rx_fm", "fm");
}
