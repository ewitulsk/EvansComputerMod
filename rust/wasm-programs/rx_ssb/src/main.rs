//! `rx_ssb` — listen to single sideband (USB by default, or `lsb`) through the Speaker.
//!
//!   rx_ssb <freq> [usb|lsb] [--gain DB] [--sdr NAME] [--rate SPS]
//!          [--speaker SIDE] [--volume 0-100] [--seconds S] [--wav FILE [--no-speaker]]

fn main() {
    ecm_radio::run::rx_main("rx_ssb", "usb");
}
