//! SSH binary packet protocol (RFC 4253 section 6).
//!
//! Packet format (before encryption):
//!   [u32 packet_length] [u8 padding_length] [payload...] [padding...]
//!
//! packet_length = padding_length + payload_len + padding_len
//! Minimum padding: 4 bytes
//! Total (padding_length + payload + padding) must be multiple of block_size (8 or cipher block)
//!
//! After encryption, a MAC may be appended.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// SSH message type constants.
pub mod msg {
    pub const DISCONNECT: u8 = 1;
    pub const IGNORE: u8 = 2;
    pub const UNIMPLEMENTED: u8 = 3;
    pub const SERVICE_REQUEST: u8 = 5;
    pub const SERVICE_ACCEPT: u8 = 6;
    pub const KEXINIT: u8 = 20;
    pub const NEWKEYS: u8 = 21;
    pub const KEX_ECDH_INIT: u8 = 30;
    pub const KEX_ECDH_REPLY: u8 = 31;
    pub const USERAUTH_REQUEST: u8 = 50;
    pub const USERAUTH_FAILURE: u8 = 51;
    pub const USERAUTH_SUCCESS: u8 = 52;
    pub const CHANNEL_OPEN: u8 = 90;
    pub const CHANNEL_OPEN_CONFIRMATION: u8 = 91;
    pub const CHANNEL_OPEN_FAILURE: u8 = 92;
    pub const CHANNEL_WINDOW_ADJUST: u8 = 93;
    pub const CHANNEL_DATA: u8 = 94;
    pub const CHANNEL_EOF: u8 = 96;
    pub const CHANNEL_CLOSE: u8 = 97;
    pub const CHANNEL_REQUEST: u8 = 98;
    pub const CHANNEL_SUCCESS: u8 = 99;
    pub const CHANNEL_FAILURE: u8 = 100;
}

/// Encode a payload into an SSH binary packet (unencrypted).
pub fn encode_packet(payload: &[u8]) -> Vec<u8> {
    encode_packet_with_block_size(payload, 8)
}

/// Encode with a specific cipher block size.
pub fn encode_packet_with_block_size(payload: &[u8], block_size: usize) -> Vec<u8> {
    let block_size = block_size.max(8);

    let unpadded = 1 + payload.len();
    let mut padding_len = block_size - (unpadded % block_size);
    if padding_len < 4 {
        padding_len += block_size;
    }

    let packet_length = 1 + payload.len() + padding_len;
    let mut packet = Vec::with_capacity(4 + packet_length);

    packet.extend_from_slice(&(packet_length as u32).to_be_bytes());
    packet.push(padding_len as u8);
    packet.extend_from_slice(payload);

    let mut padding = vec![0u8; padding_len];
    let _ = getrandom::getrandom(&mut padding);
    packet.extend_from_slice(&padding);

    packet
}

/// Decode an SSH binary packet from a byte buffer (unencrypted).
/// Returns (payload, total_bytes_consumed) or None if not enough data.
pub fn decode_packet(data: &[u8]) -> Option<(Vec<u8>, usize)> {
    if data.len() < 5 {
        return None;
    }

    let packet_length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;

    if packet_length < 1 || packet_length > 35000 {
        return None;
    }

    let total = 4 + packet_length;
    if data.len() < total {
        return None;
    }

    let padding_length = data[4] as usize;

    if padding_length < 4 || padding_length >= packet_length {
        return None;
    }

    let payload_length = packet_length - 1 - padding_length;
    let payload = data[5..5 + payload_length].to_vec();

    Some((payload, total))
}

/// SSH string encoding: [u32 length] [bytes...]
pub fn encode_string(s: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(4 + s.len());
    result.extend_from_slice(&(s.len() as u32).to_be_bytes());
    result.extend_from_slice(s);
    result
}

/// SSH string decoding from a buffer at offset.
/// Returns (string_bytes, new_offset) or None.
pub fn decode_string(data: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    if offset + 4 > data.len() {
        return None;
    }
    let len = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    let start = offset + 4;
    let end = start + len;
    if end > data.len() {
        return None;
    }
    Some((&data[start..end], end))
}

/// SSH uint32 encoding (big-endian).
pub fn encode_u32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// SSH uint32 decoding from buffer at offset.
pub fn decode_u32(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    if offset + 4 > data.len() {
        return None;
    }
    let v = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    Some((v, offset + 4))
}

/// SSH mpint encoding (big-endian, with sign-extension handling).
pub fn encode_mpint(value: &[u8]) -> Vec<u8> {
    let mut start = 0;
    while start < value.len() && value[start] == 0 {
        start += 1;
    }

    if start == value.len() {
        return encode_string(&[]);
    }

    let trimmed = &value[start..];

    if trimmed[0] & 0x80 != 0 {
        let mut padded = Vec::with_capacity(1 + trimmed.len());
        padded.push(0);
        padded.extend_from_slice(trimmed);
        encode_string(&padded)
    } else {
        encode_string(trimmed)
    }
}

/// Build an SSH name-list (comma-separated).
pub fn encode_name_list(names: &[&str]) -> Vec<u8> {
    let joined: String = names.iter().copied().collect::<Vec<&str>>().join(",");
    encode_string(joined.as_bytes())
}

/// Decode an SSH name-list.
pub fn decode_name_list<'a>(data: &'a [u8], offset: usize) -> Option<(Vec<&'a str>, usize)> {
    let (bytes, new_offset) = decode_string(data, offset)?;
    let s = core::str::from_utf8(bytes).ok()?;
    if s.is_empty() {
        Some((Vec::new(), new_offset))
    } else {
        Some((s.split(',').collect(), new_offset))
    }
}
