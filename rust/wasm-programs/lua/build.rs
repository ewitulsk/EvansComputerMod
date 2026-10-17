//! Lua's C code links wasi-sdk's libsetjmp and libwasi-emulated-signal, which
//! Rust's bundled wasi libc doesn't ship: point the linker at wasi-sdk's sysroot.
fn main() {
    println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("wasi") {
        return;
    }
    match std::env::var("WASI_SDK_PATH") {
        Ok(sdk) => println!("cargo:rustc-link-search=native={sdk}/share/wasi-sysroot/lib/wasm32-wasip1"),
        Err(_) => println!(
            "cargo:warning=WASI_SDK_PATH is not set: building Lua for WASM needs wasi-sdk \
             (https://github.com/WebAssembly/wasi-sdk/releases)"
        ),
    }
}
