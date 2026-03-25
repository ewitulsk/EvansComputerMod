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
    names
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
    Ok(())
}
