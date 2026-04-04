//! Peripheral module for interacting with ComputerCraft peripherals.
//!
//! This module provides functions to discover and interact with CC:Tweaked
//! peripherals that are adjacent to the terminal block.
//!
//! # Example
//!
//! ```rust
//! use crate::peripheral;
//!
//! // List all connected peripherals
//! let peripherals = peripheral::list();
//! for p in peripherals {
//!     println!("Found: {} ({})", p.name, p.peripheral_type);
//! }
//!
//! // Get methods available on a peripheral
//! let methods = peripheral::get_methods("chat_box_0")?;
//! for method in methods {
//!     println!("  - {}", method);
//! }
//!
//! // Call a method
//! let result = peripheral::call("chat_box_0", "sendMessage", &["Hello!"])?;
//! ```

use crate::terminal;

/// Host functions for peripheral access
extern "C" {
    /// Lists all connected peripherals, writes JSON to buffer.
    /// Returns bytes written, or -1 on error.
    fn peripheral_list(buf_ptr: *mut u8, buf_len: usize) -> i32;
    
    /// Gets methods for a peripheral, writes JSON to buffer.
    /// Returns bytes written, or -1 on error.
    fn peripheral_get_methods(
        name_ptr: *const u8, name_len: usize,
        buf_ptr: *mut u8, buf_len: usize
    ) -> i32;
    
    /// Calls a peripheral method with JSON arguments.
    /// Returns bytes written to result buffer, or -1 on error.
    fn peripheral_call(
        name_ptr: *const u8, name_len: usize,
        method_ptr: *const u8, method_len: usize,
        args_ptr: *const u8, args_len: usize,
        result_ptr: *mut u8, result_len: usize
    ) -> i32;
}

/// Buffer size for receiving JSON responses
const BUFFER_SIZE: usize = 8192;

/// Information about a discovered peripheral
#[derive(Debug, Clone)]
pub struct PeripheralInfo {
    pub name: String,
    pub peripheral_type: String,
    pub side: String,
}

/// Result of a peripheral method call
#[derive(Debug)]
pub enum PeripheralResult {
    Success(String), // JSON string of the result
    Error(String),   // Error message
}

/// Lists all peripherals connected to the terminal.
/// Returns a vector of PeripheralInfo structs.
pub fn list() -> Vec<PeripheralInfo> {
    let mut buffer = [0u8; BUFFER_SIZE];
    
    let bytes_written = unsafe {
        peripheral_list(buffer.as_mut_ptr(), buffer.len())
    };
    
    if bytes_written <= 0 {
        return Vec::new();
    }
    
    let json = match core::str::from_utf8(&buffer[..bytes_written as usize]) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    
    parse_peripheral_list(json)
}

/// Gets the method names available on a peripheral.
/// Returns Ok with a vector of method names, or Err with an error message.
pub fn get_methods(peripheral_name: &str) -> Result<Vec<String>, String> {
    let mut buffer = [0u8; BUFFER_SIZE];
    let name_bytes = peripheral_name.as_bytes();
    
    let bytes_written = unsafe {
        peripheral_get_methods(
            name_bytes.as_ptr(), name_bytes.len(),
            buffer.as_mut_ptr(), buffer.len()
        )
    };
    
    if bytes_written < 0 {
        return Err("Failed to get methods".to_string());
    }
    
    let json = match core::str::from_utf8(&buffer[..bytes_written as usize]) {
        Ok(s) => s,
        Err(_) => return Err("Invalid UTF-8 response".to_string()),
    };
    
    parse_methods_response(json)
}

/// Calls a method on a peripheral.
/// 
/// # Arguments
/// * `peripheral_name` - Name of the peripheral (e.g., "chat_box_0")
/// * `method_name` - Name of the method to call
/// * `args` - Arguments as JSON string (e.g., "[\"Hello!\", 15]")
/// 
/// # Returns
/// * `Ok(result_json)` - JSON string of the result
/// * `Err(error_message)` - Error message if the call failed
pub fn call(peripheral_name: &str, method_name: &str, args: &str) -> Result<String, String> {
    let mut buffer = [0u8; BUFFER_SIZE];
    let name_bytes = peripheral_name.as_bytes();
    let method_bytes = method_name.as_bytes();
    let args_bytes = args.as_bytes();
    
    let bytes_written = unsafe {
        peripheral_call(
            name_bytes.as_ptr(), name_bytes.len(),
            method_bytes.as_ptr(), method_bytes.len(),
            args_bytes.as_ptr(), args_bytes.len(),
            buffer.as_mut_ptr(), buffer.len()
        )
    };
    
    if bytes_written < 0 {
        return Err("Failed to call method".to_string());
    }
    
    let json = match core::str::from_utf8(&buffer[..bytes_written as usize]) {
        Ok(s) => s,
        Err(_) => return Err("Invalid UTF-8 response".to_string()),
    };
    
    parse_call_response(json)
}

/// Convenience function to call a method with a simple string array as arguments.
pub fn call_with_args(peripheral_name: &str, method_name: &str, args: &[&str]) -> Result<String, String> {
    let json_args = args_to_json(args);
    call(peripheral_name, method_name, &json_args)
}

/// Converts an array of string arguments to a JSON array.
fn args_to_json(args: &[&str]) -> String {
    let mut json = String::from("[");
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push('"');
        // Simple escaping
        for c in arg.chars() {
            match c {
                '"' => json.push_str("\\\""),
                '\\' => json.push_str("\\\\"),
                '\n' => json.push_str("\\n"),
                '\r' => json.push_str("\\r"),
                '\t' => json.push_str("\\t"),
                _ => json.push(c),
            }
        }
        json.push('"');
    }
    json.push(']');
    json
}

// === Simple JSON parsing ===
// (We don't have serde in WASM, so we do minimal parsing)

/// Parses the peripheral list JSON into a vector of PeripheralInfo.
fn parse_peripheral_list(json: &str) -> Vec<PeripheralInfo> {
    let mut result = Vec::new();
    let json = json.trim();
    
    // Expecting: [{"name":"...", "type":"...", "side":"..."}, ...]
    if !json.starts_with('[') || !json.ends_with(']') {
        return result;
    }
    
    let inner = &json[1..json.len()-1];
    if inner.trim().is_empty() {
        return result;
    }
    
    // Split by },{ to get individual objects
    for obj_str in split_json_objects(inner) {
        if let Some(info) = parse_peripheral_object(obj_str.trim()) {
            result.push(info);
        }
    }
    
    result
}

/// Splits a string containing JSON objects separated by commas.
fn split_json_objects(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut in_string = false;
    let mut escaped = false;
    
    let chars: Vec<char> = s.chars().collect();
    
    for (i, &c) in chars.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        
        if c == '\\' {
            escaped = true;
            continue;
        }
        
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        
        if !in_string {
            match c {
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    result.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    
    if start < s.len() {
        result.push(&s[start..]);
    }
    
    result
}

/// Parses a single peripheral object JSON.
fn parse_peripheral_object(json: &str) -> Option<PeripheralInfo> {
    let json = json.trim();
    if !json.starts_with('{') || !json.ends_with('}') {
        return None;
    }
    
    let inner = &json[1..json.len()-1];
    let mut name = String::new();
    let mut ptype = String::new();
    let mut side = String::new();
    
    for kv in split_json_objects(inner) {
        let kv = kv.trim();
        if let Some(colon_pos) = kv.find(':') {
            let key = kv[..colon_pos].trim().trim_matches('"');
            let value = kv[colon_pos+1..].trim().trim_matches('"');
            
            match key {
                "name" => name = value.to_string(),
                "type" => ptype = value.to_string(),
                "side" => side = value.to_string(),
                _ => {}
            }
        }
    }
    
    if !name.is_empty() && !ptype.is_empty() {
        Some(PeripheralInfo {
            name,
            peripheral_type: ptype,
            side,
        })
    } else {
        None
    }
}

/// Parses the get_methods response JSON.
fn parse_methods_response(json: &str) -> Result<Vec<String>, String> {
    let json = json.trim();
    
    // Check for error response: {"ok":false,"error":"..."}
    if json.contains("\"ok\":false") {
        if let Some(start) = json.find("\"error\":\"") {
            let start = start + 9;
            if let Some(end) = json[start..].find('"') {
                return Err(json[start..start+end].to_string());
            }
        }
        return Err("Unknown error".to_string());
    }
    
    // Parse success response: {"ok":true,"result":["method1","method2",...]}
    if let Some(start) = json.find("\"result\":[") {
        let start = start + 10;
        if let Some(end) = json[start..].find(']') {
            let array_content = &json[start..start+end];
            let methods: Vec<String> = array_content
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return Ok(methods);
        }
    }
    
    Err("Invalid response format".to_string())
}

/// Parses the call response JSON.
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
        // The result can be any JSON value, so we return everything after "result":
        // up to the closing brace
        let result_content = &json[result_start..json.len()-1];
        return Ok(result_content.to_string());
    }
    
    // If ok:true but no result, return null
    if json.contains("\"ok\":true") {
        return Ok("null".to_string());
    }
    
    Ok(json.to_string())
}

// === High-level convenience functions ===

/// Checks if CC:Tweaked integration is available by listing peripherals.
pub fn is_available() -> bool {
    let peripherals = list();
    // If we got an empty list, CC might not be installed, but that's not an error
    true // The host function exists, so integration is available
}

/// Finds a peripheral by type.
pub fn find_by_type(peripheral_type: &str) -> Option<PeripheralInfo> {
    list().into_iter().find(|p| p.peripheral_type == peripheral_type)
}

/// Gets the names of all connected peripherals.
pub fn get_names() -> Vec<String> {
    list().into_iter().map(|p| p.name).collect()
}

/// Wraps a peripheral for easier method calling.
pub struct Peripheral {
    pub name: String,
    pub peripheral_type: String,
}

impl Peripheral {
    /// Wraps an existing peripheral by name.
    pub fn wrap(name: &str) -> Option<Self> {
        list().into_iter().find(|p| p.name == name).map(|p| Peripheral {
            name: p.name,
            peripheral_type: p.peripheral_type,
        })
    }
    
    /// Gets the methods available on this peripheral.
    pub fn get_methods(&self) -> Result<Vec<String>, String> {
        get_methods(&self.name)
    }
    
    /// Calls a method on this peripheral.
    pub fn call(&self, method: &str, args: &str) -> Result<String, String> {
        call(&self.name, method, args)
    }
    
    /// Calls a method with string arguments.
    pub fn call_args(&self, method: &str, args: &[&str]) -> Result<String, String> {
        call_with_args(&self.name, method, args)
    }
}
