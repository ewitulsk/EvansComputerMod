//! CC:Tweaked peripheral host function wrappers.

extern crate alloc;
use alloc::string::String;
use alloc::vec;

extern "C" {
    fn peripheral_list(buf_ptr: i32, buf_len: i32) -> i32;
    fn peripheral_get_methods(name_ptr: i32, name_len: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn peripheral_call(
        name_ptr: i32, name_len: i32,
        method_ptr: i32, method_len: i32,
        args_ptr: i32, args_len: i32,
        result_ptr: i32, result_len: i32,
    ) -> i32;
}

/// List all connected peripherals. Returns JSON string.
pub fn list_raw() -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { peripheral_list(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        None
    } else {
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }
}

/// Get methods for a named peripheral. Returns JSON string.
pub fn get_methods_raw(name: &str) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe {
        peripheral_get_methods(
            name.as_ptr() as i32, name.len() as i32,
            buf.as_mut_ptr() as i32, buf.len() as i32,
        )
    };
    if n <= 0 {
        None
    } else {
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }
}

/// Call a method on a peripheral. `args` and result are JSON strings.
pub fn call_method(name: &str, method: &str, args: &str) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe {
        peripheral_call(
            name.as_ptr() as i32, name.len() as i32,
            method.as_ptr() as i32, method.len() as i32,
            args.as_ptr() as i32, args.len() as i32,
            buf.as_mut_ptr() as i32, buf.len() as i32,
        )
    };
    if n <= 0 {
        None
    } else {
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }
}
