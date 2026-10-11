//! `rx_am` — listen to AM (broadcast, aviation) through the Speaker.
//!
//!   rx_am <freq> [--squelch DB] [--gain DB] [--sdr NAME] [--rate SPS]
//!         [--speaker SIDE] [--volume 0-100] [--seconds S] [--wav FILE [--no-speaker]]

fn main() {
    ecm_radio::run::rx_main("rx_am", "am");
}
