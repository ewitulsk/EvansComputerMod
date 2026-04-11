//! Host function registration.
//!
//! Each submodule implements one group of host functions and follows the same
//! pattern: a `FUNCTIONS` constant listing the names it registers, and a
//! `register()` function that adds them to the wasmtime Linker.
//!
//! ## Adding new host functions
//!
//! 1. Create a new file in `src/host/` (e.g., `my_feature.rs`)
//! 2. Define `pub const FUNCTIONS: &[&str]` with the function names
//! 3. Define `pub fn register(linker: &mut Linker<HostState>) -> Result<()>`
//! 4. Add `mod my_feature;` below and wire it into `known_names()` + `register_all()`
//!
//! See `sleep.rs` for the simplest example, or `redstone.rs` for one that
//! uses WASM memory helpers.

pub mod memory;
mod terminal;
mod filesystem;
mod redstone;
mod sleep;
mod interrupts;
mod peripherals;
mod getrandom;
mod network;
mod fd_ops;
mod tty;
mod process;
mod ipc;
pub mod sock_ipc;
pub mod wasi_io;
pub mod wasi_stubs;

use wasmtime::*;
use crate::wasm_host::HostState;

/// All known host function names (used to skip wasm-bindgen stub generation).
pub fn known_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    names.extend_from_slice(terminal::FUNCTIONS);
    names.extend_from_slice(filesystem::FUNCTIONS);
    names.extend_from_slice(redstone::FUNCTIONS);
    names.extend_from_slice(sleep::FUNCTIONS);
    names.extend_from_slice(interrupts::FUNCTIONS);
    names.extend_from_slice(peripherals::FUNCTIONS);
    names.extend_from_slice(getrandom::FUNCTIONS);
    names.extend_from_slice(network::FUNCTIONS);
    names.extend_from_slice(fd_ops::FUNCTIONS);
    names.extend_from_slice(tty::FUNCTIONS);
    names.extend_from_slice(process::FUNCTIONS);
    names.extend_from_slice(sock_ipc::FUNCTIONS);
    names.extend_from_slice(ipc::FUNCTIONS);
    names
}

/// Register env-namespace host functions needed by WASI programs.
///
/// Unlike `register_all` (which includes kernel-only functions like terminal,
/// redstone, peripherals, process management), this registers only the subset
/// needed by standalone WASI programs: networking, sleep, getrandom, filesystem, and IPC.
pub fn register_env_for_wasi(linker: &mut Linker<HostState>) -> Result<()> {
    network::register(linker)?;
    sleep::register(linker)?;
    getrandom::register(linker)?;
    filesystem::register(linker)?;
    ipc::register(linker)?;
    sock_ipc::register(linker)?;
    peripherals::register(linker)?;
    redstone::register(linker)?;
    Ok(())
}

/// Register all host functions on the linker.
///
/// Each submodule's `register()` is called in order. To add a new group of
/// host functions, add your module's `register()` call here.
pub fn register_all(linker: &mut Linker<HostState>) -> Result<()> {
    terminal::register(linker)?;
    filesystem::register(linker)?;
    redstone::register(linker)?;
    sleep::register(linker)?;
    interrupts::register(linker)?;
    peripherals::register(linker)?;
    getrandom::register(linker)?;
    network::register(linker)?;
    fd_ops::register(linker)?;
    tty::register(linker)?;
    process::register(linker)?;
    sock_ipc::register(linker)?;
    ipc::register(linker)?;
    wasi_io::register(linker)?;
    Ok(())
}
