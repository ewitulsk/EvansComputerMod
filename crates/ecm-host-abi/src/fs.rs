//! Filesystem host function wrappers.

extern "C" {
    fn file_read(path_ptr: i32, path_len: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn file_write(path_ptr: i32, path_len: i32, data_ptr: i32, data_len: i32) -> i32;
    fn file_exists(path_ptr: i32, path_len: i32) -> i32;
    fn file_size(path_ptr: i32, path_len: i32) -> i32;
    fn file_mkdir(path_ptr: i32, path_len: i32) -> i32;
}

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Read a file's contents as bytes. Returns None if the file doesn't exist.
pub fn read_file_bytes(path: &str) -> Option<Vec<u8>> {
    let size = unsafe { file_size(path.as_ptr() as i32, path.len() as i32) };
    if size < 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let n = unsafe {
        file_read(
            path.as_ptr() as i32,
            path.len() as i32,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
        )
    };
    if n < 0 {
        None
    } else {
        buf.truncate(n as usize);
        Some(buf)
    }
}

/// Read a file as a UTF-8 string.
pub fn read_file(path: &str) -> Option<String> {
    read_file_bytes(path).and_then(|b| String::from_utf8(b).ok())
}

/// Write bytes to a file. Returns 0 on success, -1 on error.
pub fn write_file_bytes(path: &str, data: &[u8]) -> i32 {
    unsafe {
        file_write(
            path.as_ptr() as i32,
            path.len() as i32,
            data.as_ptr() as i32,
            data.len() as i32,
        )
    }
}

/// Write a string to a file.
pub fn write_file(path: &str, data: &str) -> i32 {
    write_file_bytes(path, data.as_bytes())
}

/// Check if a file exists. Returns true if it does.
pub fn exists(path: &str) -> bool {
    unsafe { file_exists(path.as_ptr() as i32, path.len() as i32) == 1 }
}

/// Create a directory. Returns 0 on success.
pub fn mkdir(path: &str) -> i32 {
    unsafe { file_mkdir(path.as_ptr() as i32, path.len() as i32) }
}
