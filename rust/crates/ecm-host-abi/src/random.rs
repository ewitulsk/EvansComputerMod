//! Random number generation via host function.

extern "C" {
    fn __getrandom_v03_custom(buf_ptr: i32, buf_len: i32) -> i32;
}

/// Fill a buffer with random bytes from the host.
pub fn getrandom(buf: &mut [u8]) -> i32 {
    unsafe { __getrandom_v03_custom(buf.as_mut_ptr() as i32, buf.len() as i32) }
}
