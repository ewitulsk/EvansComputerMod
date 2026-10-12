//! Entropy self-check for the host's child ABI (KernelHostIntegrationTest).
//!
//! Prints two 32-byte draws through `ecm_host_abi::random` (what wifi and
//! wpa_supplicant use for SNonces) and two through the raw
//! `env.__getrandom_v03_custom` import (what getrandom 0.3's custom backend,
//! e.g. ahash, links against in child programs). Every line must be nonzero
//! and the two draws of a pair must differ.

#[link(wasm_import_module = "env")]
extern "C" {
    fn __getrandom_v03_custom(buf_ptr: i32, buf_len: i32) -> i32;
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    for i in 1..=2 {
        let mut b = [0u8; 32];
        let rc = ecm_host_abi::random::getrandom(&mut b);
        println!("rng wrapper{i} rc={rc} {}", hex(&b));
    }
    for i in 1..=2 {
        let mut b = [0u8; 32];
        let rc = unsafe { __getrandom_v03_custom(b.as_mut_ptr() as i32, b.len() as i32) };
        println!("rng raw{i} rc={rc} {}", hex(&b));
    }
}
