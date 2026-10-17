//! `mrfd`: the carrier's media server (docs/TELEPHONY.md). The machine is `ecm_tel::mrf`.
//!
//!   mrfd start [--wait] | stop | status | boxes | log [n]
//!   mrfd config | config keys | config set <key> <value>... | enable | disable

#[cfg(target_os = "wasi")]
mod prog;

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("mrfd runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}
