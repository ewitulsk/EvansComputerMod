//! Redstone I/O host function wrappers.

extern "C" {
    fn redstone_set_output(side: i32, power: i32) -> i32;
    fn redstone_get_input(side: i32) -> i32;
    fn redstone_get_all_input(buf_ptr: i32) -> i32;
}

/// Set redstone output power (0-15) on a side (0-5).
pub fn set_output(side: i32, power: i32) -> i32 {
    unsafe { redstone_set_output(side, power) }
}

/// Get redstone input power on a side (0-5). Returns 0-15.
pub fn get_input(side: i32) -> i32 {
    unsafe { redstone_get_input(side) }
}

/// Get all 6 sides' input power levels. Writes 6 i32 values to buffer.
pub fn get_all_input(buf: &mut [i32; 6]) -> i32 {
    unsafe { redstone_get_all_input(buf.as_mut_ptr() as i32) }
}
