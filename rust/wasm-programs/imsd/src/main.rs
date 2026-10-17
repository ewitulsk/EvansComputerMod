//! `imsd`: the carrier's IMS core (docs/TELEPHONY.md).
//!
//!   imsd start [--wait]          run it (background it: imsd start &); --wait: wait for cored at boot
//!   imsd stop | status | log [n] the running imsd (admin socket 127.0.0.1:7721)
//!   imsd config | config keys | config set <key> <value>...
//!   imsd peer add <domain> <ip[:port]|srv> <secret> | peer remove <domain> | peers
//!   imsd enable | disable        start at boot (services.cfg)

#[cfg(target_os = "wasi")]
mod prog;

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("imsd runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}
