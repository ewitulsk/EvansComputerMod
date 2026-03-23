//! Module bridge for calling Java-registered computer modules.
//!
//! This module provides a generic bridge to invoke methods registered via
//! `@ComputerModule` annotations on the Java side. It uses the same
//! extern-C-with-buffer pattern as the peripheral module.
//!
//! # Example
//!
//! ```rust
//! use crate::modules;
//!
//! // List all registered modules and their functions
//! let metadata = modules::list_modules();
//!
//! // Call a method on a registered module
//! let result = modules::call("golem", "summon", "[\"iron\"]")?;
//! ```

/// Host functions for module bridge access
extern "C" {
    /// Calls a registered module method with JSON arguments.
    /// Returns bytes written to result buffer, or -1 on error.
    fn module_call(
        module_ptr: *const u8, module_len: usize,
        method_ptr: *const u8, method_len: usize,
        args_ptr: *const u8, args_len: usize,
        result_ptr: *mut u8, result_len: usize
    ) -> i32;

    /// Lists all registered modules as JSON metadata.
    /// Returns bytes written to buffer, or -1 on error.
    fn module_list(buf_ptr: *mut u8, buf_len: usize) -> i32;
}

/// Buffer size for receiving JSON responses
const BUFFER_SIZE: usize = 16384;

/// Calls a method on a registered computer module.
///
/// # Arguments
/// * `module_name` - Name of the module (e.g., "golem")
/// * `method_name` - Name of the method to call (e.g., "summon")
/// * `args` - Arguments as JSON array string (e.g., "[\"iron\"]")
///
/// # Returns
/// * `Ok(result_json)` - JSON string of the result
/// * `Err(error_message)` - Error message if the call failed
pub fn call(module_name: &str, method_name: &str, args: &str) -> Result<String, String> {
    let mut buffer = [0u8; BUFFER_SIZE];
    let mod_bytes = module_name.as_bytes();
    let method_bytes = method_name.as_bytes();
    let args_bytes = args.as_bytes();

    let bytes_written = unsafe {
        module_call(
            mod_bytes.as_ptr(), mod_bytes.len(),
            method_bytes.as_ptr(), method_bytes.len(),
            args_bytes.as_ptr(), args_bytes.len(),
            buffer.as_mut_ptr(), buffer.len()
        )
    };

    if bytes_written < 0 {
        return Err("Failed to call module method".to_string());
    }

    let json = match core::str::from_utf8(&buffer[..bytes_written as usize]) {
        Ok(s) => s,
        Err(_) => return Err("Invalid UTF-8 response".to_string()),
    };

    parse_call_response(json)
}

/// Lists all registered modules as a JSON metadata string.
/// Returns the raw JSON metadata describing all modules and their functions.
pub fn list_modules() -> String {
    let mut buffer = [0u8; BUFFER_SIZE];

    let bytes_written = unsafe {
        module_list(buffer.as_mut_ptr(), buffer.len())
    };

    if bytes_written <= 0 {
        return "{\"modules\":{}}".to_string();
    }

    match core::str::from_utf8(&buffer[..bytes_written as usize]) {
        Ok(s) => s.to_string(),
        Err(_) => "{\"modules\":{}}".to_string(),
    }
}

/// Parses a call response JSON, extracting the result or error.
fn parse_call_response(json: &str) -> Result<String, String> {
    let json = json.trim();

    // Check for error response
    if json.contains("\"ok\":false") {
        if let Some(start) = json.find("\"error\":\"") {
            let start = start + 9;
            if let Some(end) = json[start..].find('"') {
                return Err(json[start..start+end].to_string());
            }
        }
        return Err("Unknown error".to_string());
    }

    // Return the result portion of the response
    if let Some(start) = json.find("\"result\":") {
        let result_start = start + 9;
        let result_content = &json[result_start..json.len()-1];
        return Ok(result_content.to_string());
    }

    // If ok:true but no result, return null
    if json.contains("\"ok\":true") {
        return Ok("null".to_string());
    }

    Ok(json.to_string())
}
