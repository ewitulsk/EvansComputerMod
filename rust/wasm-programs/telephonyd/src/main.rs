//! `telephonyd`: the phone's IMS client (docs/TELEPHONY.md).
//!
//!   telephonyd [--phone] [--no-mms] [--quiet]
//!
//! Started by the ePhone beside celld (`--phone`: the status bar, notifications,
//! ringing); on a Terminal with a modem: `telephonyd &` (or a services.cfg line).
//! Settings in /etc/telephonyd.toml: `sim`, `mms` (auto|off), `wideband`, `speaker`,
//! `mic`, `ringtone` (on|off). Control socket 127.0.0.1:7720 (ecm_tel::ctl).

#[cfg(target_os = "wasi")]
mod prog;
#[cfg(target_os = "wasi")]
mod store {
    pub use telephonyd::store::*;
}

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("telephonyd runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}
