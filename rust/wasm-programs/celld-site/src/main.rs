//! `celld-site` — the cell-site daemon (Cellular v2, docs/gui/CELLULAR_V2.md).
//!
//!   celld-site start                       run the site (background it: celld-site start &)
//!   celld-site stop | restart              ask the running site to stop / re-read its settings
//!   celld-site status                      the site, its radio, core link and phones
//!   celld-site log [n]                     the last n log lines
//!   celld-site config                      show /etc/celld-site.toml
//!   celld-site config set <key> <value>... change settings (celld-site restart applies them)
//!   celld-site config keys                 every key with its meaning
//!   celld-site enable | disable            start at boot (services.cfg)
//!
//! The running site answers on its admin socket, 127.0.0.1:7710 (the Cell Site app uses it too).

#[cfg(target_os = "wasi")]
mod prog;

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("celld-site runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}
