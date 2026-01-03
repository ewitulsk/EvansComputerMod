//! File system bindings for the terminal host functions.
//! Provides safe Rust wrappers around the low-level host functions.

// Host functions for file system operations
extern "C" {
    /// Writes data to a file.
    /// Returns bytes written, or -1 on error.
    fn file_write(path_ptr: *const u8, path_len: usize, data_ptr: *const u8, data_len: usize) -> i32;
    
    /// Reads data from a file into a buffer.
    /// Returns bytes read, or -1 on error.
    fn file_read(path_ptr: *const u8, path_len: usize, buf_ptr: *mut u8, buf_len: usize) -> i32;
    
    /// Gets the size of a file.
    /// Returns file size, or -1 if file doesn't exist.
    fn file_size(path_ptr: *const u8, path_len: usize) -> i32;
    
    /// Checks if a file exists.
    /// Returns 1 if exists, 0 if not.
    fn file_exists(path_ptr: *const u8, path_len: usize) -> i32;
    
    /// Deletes a file.
    /// Returns 1 on success, 0 on failure.
    fn file_delete(path_ptr: *const u8, path_len: usize) -> i32;
    
    /// Lists all files in the computer's storage.
    /// Writes newline-separated filenames to the buffer.
    /// Returns bytes written.
    fn file_list(buf_ptr: *mut u8, buf_len: usize) -> i32;
}

/// Maximum file size we support reading into memory
const MAX_FILE_SIZE: usize = 64 * 1024; // 64KB

/// Buffer for file operations
static mut FILE_BUFFER: [u8; MAX_FILE_SIZE] = [0u8; MAX_FILE_SIZE];

/// Writes content to a file.
/// Returns true on success, false on failure.
pub fn write_file(filename: &str, content: &str) -> bool {
    unsafe {
        let result = file_write(
            filename.as_ptr(),
            filename.len(),
            content.as_ptr(),
            content.len()
        );
        result >= 0
    }
}

/// Reads a file's content.
/// Returns the content as a string, or None if the file doesn't exist or an error occurred.
pub fn read_file(filename: &str) -> Option<&'static str> {
    unsafe {
        // First check if file exists and get its size
        let size = file_size(filename.as_ptr(), filename.len());
        if size < 0 {
            return None;
        }
        
        let size = size as usize;
        if size > MAX_FILE_SIZE {
            return None;
        }
        
        // Read the file into our buffer
        let bytes_read = file_read(
            filename.as_ptr(),
            filename.len(),
            FILE_BUFFER.as_mut_ptr(),
            size
        );
        
        if bytes_read < 0 {
            return None;
        }
        
        // Convert to string
        let slice = &FILE_BUFFER[..bytes_read as usize];
        std::str::from_utf8(slice).ok()
    }
}

/// Checks if a file exists.
pub fn exists(filename: &str) -> bool {
    unsafe {
        file_exists(filename.as_ptr(), filename.len()) == 1
    }
}

/// Deletes a file.
/// Returns true on success, false on failure.
pub fn delete_file(filename: &str) -> bool {
    unsafe {
        file_delete(filename.as_ptr(), filename.len()) == 1
    }
}

/// Gets the size of a file, or None if it doesn't exist.
pub fn get_size(filename: &str) -> Option<usize> {
    unsafe {
        let size = file_size(filename.as_ptr(), filename.len());
        if size >= 0 {
            Some(size as usize)
        } else {
            None
        }
    }
}

/// Buffer for file listing
static mut LIST_BUFFER: [u8; 4096] = [0u8; 4096];

/// Lists all files in the computer's storage.
/// Returns a newline-separated list of filenames.
pub fn list_files() -> &'static str {
    unsafe {
        let bytes_written = file_list(LIST_BUFFER.as_mut_ptr(), LIST_BUFFER.len());
        if bytes_written <= 0 {
            return "";
        }
        
        let slice = &LIST_BUFFER[..bytes_written as usize];
        std::str::from_utf8(slice).unwrap_or("")
    }
}
