//! Channel coding and framing.

pub mod ax25;
pub mod conv;
pub mod crc;
pub mod hamming;
pub mod hdlc;
pub mod kiss;
pub mod rs;

/// Unpack bytes to bits, LSB first.
pub fn bytes_to_bits_lsb(data: &[u8]) -> Vec<u8> {
    data.iter().flat_map(|&b| (0..8).map(move |i| (b >> i) & 1)).collect()
}

/// Pack bits (LSB first) to bytes; a trailing partial byte is zero-padded.
pub fn bits_to_bytes_lsb(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | ((b & 1) << i)))
        .collect()
}

/// Unpack bytes to bits, MSB first.
pub fn bytes_to_bits_msb(data: &[u8]) -> Vec<u8> {
    data.iter().flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1)).collect()
}

/// Pack bits (MSB first) to bytes; a trailing partial byte is zero-padded.
pub fn bits_to_bytes_msb(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | ((b & 1) << (7 - i))))
        .collect()
}
