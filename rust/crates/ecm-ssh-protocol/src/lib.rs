//! SSH protocol implementation.
//!
//! Pure protocol logic with no I/O or host-function dependencies.
//! All I/O is mediated through byte buffers and callback parameters.

#![no_std]

extern crate alloc;

pub mod packet;
pub mod transport;
pub mod kex;
pub mod auth;
pub mod channel;
