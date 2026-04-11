//! Shared kernel-level primitives reused between terminal-os and switch-os.
//!
//! - `framebuffer`: memory-mapped framebuffer layout constants + header I/O.
//! - `vte`: VT100/ANSI terminal emulator writing into the framebuffer.
//! - `fs`: filesystem bindings to the host `file_*` functions.
//! - `interrupt`: small IRQ dispatch table with Rust + Python handler slots.
//! - `netlink`: rtnetlink message handler that queries/mutates `ecm_net::NetStack`.
//! - `shell_parse`: command-line tokenizer and pipeline parser (no execution).

extern crate alloc;

pub mod framebuffer;
pub mod fs;
pub mod interrupt;
pub mod netlink;
pub mod shell_parse;
pub mod vte;
