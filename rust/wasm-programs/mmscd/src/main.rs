//! `mmscd`: the carrier's MMSC (docs/TELEPHONY.md). The machine is `ecm_tel::mmsc`.
//!
//!   mmscd start [--wait] | stop | status | stored | log [n]
//!   mmscd config | config keys | config set <key> <value>... | enable | disable

#[cfg(target_os = "wasi")]
mod prog;

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("mmscd runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}
