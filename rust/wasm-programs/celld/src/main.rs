//! `celld [--phone] [--modem NAME] [--sim NAME] [--quiet]`: see the library.

#[cfg(target_os = "wasi")]
fn main() {
    celld::host::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("celld runs on an ECM computer (wasm32-wasip1)");
    std::process::exit(1);
}
