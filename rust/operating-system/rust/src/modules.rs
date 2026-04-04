//! Module bridge for calling Java-registered computer modules.
//!
//! Uses a binary protocol for argument passing and result serialization,
//! eliminating JSON overhead. The `module_list` function still uses JSON
//! for metadata discovery (called once at startup).
//!
//! # Binary Wire Format
//!
//! Arguments: `[u8 arg_count] ([u8 type_tag] [payload])*`
//! Result:    `[u8 status(0=ok,1=err)] [u8 type_tag] [payload]`
//!
//! Type tags:
//! - 0x00 = null
//! - 0x01 = string: [u32 LE len] [utf8 bytes]
//! - 0x02 = i32:    [i32 LE]
//! - 0x03 = i64:    [i64 LE]
//! - 0x04 = f64:    [f64 LE]
//! - 0x05 = bool:   [u8 0|1]

/// Host functions for module bridge access
extern "C" {
    /// Calls a registered module method with binary-encoded arguments.
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

// Type tags — must match Java side (ModuleMethodInvoker)
pub const TAG_NULL: u8   = 0x00;
pub const TAG_STRING: u8 = 0x01;
pub const TAG_I32: u8    = 0x02;
pub const TAG_I64: u8    = 0x03;
pub const TAG_F64: u8    = 0x04;
pub const TAG_BOOL: u8   = 0x05;

// Result status
pub const STATUS_OK: u8    = 0x00;
pub const STATUS_ERROR: u8 = 0x01;

/// Buffer size for receiving binary responses
const BUFFER_SIZE: usize = 16384;

/// A typed value deserialized from the binary protocol.
#[derive(Debug)]
pub enum BinaryValue {
    Null,
    Str(String),
    I32(i32),
    I64(i64),
    F64(f64),
    Bool(bool),
}

/// Calls a method on a registered computer module using binary protocol.
///
/// # Arguments
/// * `module_name` - Name of the module (e.g., "golem")
/// * `method_name` - Name of the method to call (e.g., "summon")
/// * `args_binary` - Binary-encoded arguments
///
/// # Returns
/// * `Ok(BinaryValue)` - The typed result
/// * `Err(error_message)` - Error message if the call failed
pub fn call(module_name: &str, method_name: &str, args_binary: &[u8]) -> Result<BinaryValue, String> {
    let mut buffer = [0u8; BUFFER_SIZE];
    let mod_bytes = module_name.as_bytes();
    let method_bytes = method_name.as_bytes();

    let bytes_written = unsafe {
        module_call(
            mod_bytes.as_ptr(), mod_bytes.len(),
            method_bytes.as_ptr(), method_bytes.len(),
            args_binary.as_ptr(), args_binary.len(),
            buffer.as_mut_ptr(), buffer.len()
        )
    };

    if bytes_written < 0 {
        return Err("Failed to call module method".to_string());
    }

    let data = &buffer[..bytes_written as usize];
    parse_binary_result(data)
}

/// Parses a binary-encoded result from the Java side.
pub fn parse_binary_result(data: &[u8]) -> Result<BinaryValue, String> {
    if data.is_empty() {
        return Err("Empty result".to_string());
    }

    let status = data[0];
    let payload = &data[1..];

    if status == STATUS_ERROR {
        // Error: read string payload
        if payload.len() >= 5 && payload[0] == TAG_STRING {
            let len = read_u32_le(&payload[1..]) as usize;
            if payload.len() >= 5 + len {
                let msg = core::str::from_utf8(&payload[5..5 + len])
                    .unwrap_or("Invalid UTF-8 error");
                return Err(msg.to_string());
            }
        }
        return Err("Unknown error".to_string());
    }

    // Success: read typed value
    if payload.is_empty() {
        return Ok(BinaryValue::Null);
    }

    read_tagged_value(payload).map(|(val, _)| val)
}

/// Reads a single tagged value from a byte slice. Returns the value and bytes consumed.
fn read_tagged_value(data: &[u8]) -> Result<(BinaryValue, usize), String> {
    if data.is_empty() {
        return Err("Unexpected end of data".to_string());
    }

    let tag = data[0];
    let payload = &data[1..];

    match tag {
        TAG_NULL => Ok((BinaryValue::Null, 1)),
        TAG_STRING => {
            if payload.len() < 4 {
                return Err("Truncated string length".to_string());
            }
            let len = read_u32_le(payload) as usize;
            if payload.len() < 4 + len {
                return Err("Truncated string data".to_string());
            }
            let s = core::str::from_utf8(&payload[4..4 + len])
                .map_err(|_| "Invalid UTF-8 string".to_string())?;
            Ok((BinaryValue::Str(s.to_string()), 1 + 4 + len))
        }
        TAG_I32 => {
            if payload.len() < 4 {
                return Err("Truncated i32".to_string());
            }
            Ok((BinaryValue::I32(read_i32_le(payload)), 1 + 4))
        }
        TAG_I64 => {
            if payload.len() < 8 {
                return Err("Truncated i64".to_string());
            }
            Ok((BinaryValue::I64(read_i64_le(payload)), 1 + 8))
        }
        TAG_F64 => {
            if payload.len() < 8 {
                return Err("Truncated f64".to_string());
            }
            Ok((BinaryValue::F64(read_f64_le(payload)), 1 + 8))
        }
        TAG_BOOL => {
            if payload.is_empty() {
                return Err("Truncated bool".to_string());
            }
            Ok((BinaryValue::Bool(payload[0] != 0), 1 + 1))
        }
        _ => Err(format!("Unknown type tag: 0x{:02x}", tag)),
    }
}

// ==================== Binary Encoding Helpers ====================

/// Encodes arguments into the binary wire format.
/// Returns the encoded bytes: [u8 arg_count] ([u8 type_tag] [payload])*
pub fn encode_args(args: &[BinaryValue]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(args.len() as u8);

    for arg in args {
        match arg {
            BinaryValue::Null => buf.push(TAG_NULL),
            BinaryValue::Str(s) => {
                buf.push(TAG_STRING);
                let bytes = s.as_bytes();
                write_u32_le(&mut buf, bytes.len() as u32);
                buf.extend_from_slice(bytes);
            }
            BinaryValue::I32(v) => {
                buf.push(TAG_I32);
                write_i32_le(&mut buf, *v);
            }
            BinaryValue::I64(v) => {
                buf.push(TAG_I64);
                write_i64_le(&mut buf, *v);
            }
            BinaryValue::F64(v) => {
                buf.push(TAG_F64);
                write_f64_le(&mut buf, *v);
            }
            BinaryValue::Bool(b) => {
                buf.push(TAG_BOOL);
                buf.push(if *b { 1 } else { 0 });
            }
        }
    }

    buf
}

// ==================== Little-Endian Read/Write Helpers ====================

fn read_u32_le(data: &[u8]) -> u32 {
    (data[0] as u32) | ((data[1] as u32) << 8) | ((data[2] as u32) << 16) | ((data[3] as u32) << 24)
}

fn read_i32_le(data: &[u8]) -> i32 {
    read_u32_le(data) as i32
}

fn read_i64_le(data: &[u8]) -> i64 {
    let lo = read_u32_le(data) as u64;
    let hi = read_u32_le(&data[4..]) as u64;
    (lo | (hi << 32)) as i64
}

fn read_f64_le(data: &[u8]) -> f64 {
    f64::from_bits(read_i64_le(data) as u64)
}

fn write_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.push((v & 0xFF) as u8);
    buf.push(((v >> 8) & 0xFF) as u8);
    buf.push(((v >> 16) & 0xFF) as u8);
    buf.push(((v >> 24) & 0xFF) as u8);
}

fn write_i32_le(buf: &mut Vec<u8>, v: i32) {
    write_u32_le(buf, v as u32);
}

fn write_i64_le(buf: &mut Vec<u8>, v: i64) {
    write_u32_le(buf, v as u32);
    write_u32_le(buf, (v >> 32) as u32);
}

fn write_f64_le(buf: &mut Vec<u8>, v: f64) {
    write_i64_le(buf, v.to_bits() as i64);
}

/// Lists all registered modules as a JSON metadata string.
/// Returns the raw JSON metadata describing all modules and their functions.
/// This is still JSON because it's complex nested data called once at startup.
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
