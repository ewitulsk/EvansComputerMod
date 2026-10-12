//! Cryptographically secure random bytes for WASI programs.
//!
//! Child programs (`wasm32-wasip1`) take entropy from WASI `random_get`, the
//! one entropy import every child host provides (the Java host's
//! `WasiFunctions` backs it with `SecureRandom`). This used to import
//! `env.__getrandom_v03_custom`, a kernel-only host function: children were
//! never given it, so the runtime auto-stubbed it to "return 0, write nothing"
//! and every buffer came back all zeros (Wi-Fi SNonces, traceroute ids).
//!
//! The kernel (`wasm32-unknown-unknown`) has no WASI and keeps using its own
//! `__getrandom_v03_custom` host import (it normally goes through `hal.rs`).


#[cfg(target_os = "wasi")]
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    #[link_name = "random_get"]
    fn wasi_random_get(buf_ptr: i32, buf_len: i32) -> i32;
}

/// Fill `buf` with random bytes from the host. Returns 0 on success or a
/// WASI errno. Prefer [`fill`], which cannot be ignored by accident.
#[must_use = "a failed getrandom leaves the buffer unchanged; use random::fill"]
pub fn getrandom(buf: &mut [u8]) -> i32 {
    if buf.is_empty() {
        return 0;
    }
    imp(buf)
}

#[cfg(target_os = "wasi")]
fn imp(buf: &mut [u8]) -> i32 {
    unsafe { wasi_random_get(buf.as_mut_ptr() as i32, buf.len() as i32) }
}

/// The kernel (`wasm32-unknown-unknown`, no WASI) keeps its own host import.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
extern "C" {
    fn __getrandom_v03_custom(buf_ptr: i32, buf_len: i32) -> i32;
}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
fn imp(buf: &mut [u8]) -> i32 {
    unsafe { __getrandom_v03_custom(buf.as_mut_ptr() as i32, buf.len() as i32) }
}

/// Native builds (host-side `cargo test` of programs) get entropy from the OS
/// through std's randomly keyed hasher, which is enough for tests. Never zeros.
#[cfg(not(target_arch = "wasm32"))]
fn imp(buf: &mut [u8]) -> i32 {
    extern crate std;
    use core::sync::atomic::{AtomicU64, Ordering};
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    static CTR: AtomicU64 = AtomicU64::new(0);
    for chunk in buf.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(CTR.fetch_add(1, Ordering::Relaxed));
        let v = h.finish().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    0
}

/// Fill `buf` with random bytes, aborting the program if the host has no
/// entropy. Security code (nonces, keys) must never continue with a buffer
/// that was not filled.
pub fn fill(buf: &mut [u8]) {
    let rc = getrandom(buf);
    if rc != 0 {
        panic!("random_get failed (errno {rc}); refusing to continue without entropy");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_is_nonzero_and_differs() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        fill(&mut a);
        fill(&mut b);
        assert!(a.iter().any(|&x| x != 0), "all-zero entropy");
        assert!(b.iter().any(|&x| x != 0), "all-zero entropy");
        assert_ne!(a, b, "two draws returned the same bytes");
    }

    #[test]
    fn odd_lengths_and_empty() {
        let mut e: [u8; 0] = [];
        assert_eq!(getrandom(&mut e), 0);
        let mut c = [0u8; 13];
        fill(&mut c);
        assert!(c.iter().any(|&x| x != 0));
    }
}
