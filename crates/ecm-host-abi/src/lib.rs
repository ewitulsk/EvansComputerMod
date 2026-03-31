//! Host function ABI declarations for ECM WASM programs.
//!
//! This crate provides safe Rust wrappers around the `extern "C"` host
//! functions that the simulator / Java runtime expose to WASM programs.
//! Programs compiled to `wasm32-wasip1` depend on this crate for socket,
//! IPC, filesystem, and other host-provided services.

#![no_std]

extern crate alloc;

pub mod ipc;
pub mod net_ipc;
pub mod fs;
pub mod random;

/// Status codes returned by IPC session queries.
pub mod ipc_status {
    pub const RUNNING: i32 = 0;
    pub const EXITED: i32 = 1;
    pub const INVALID: i32 = -1;
}

/// Error code for failed host calls.
pub const ERR: i32 = -1;
