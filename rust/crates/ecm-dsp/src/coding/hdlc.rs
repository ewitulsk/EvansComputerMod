//! HDLC framing as used by AX.25 packet radio: `0x7E` flags, zero-bit
//! stuffing after five ones, LSB-first bytes, CRC-16/X-25 FCS, plus NRZI line
//! coding (a `0` bit toggles the line, a `1` keeps it).
//!
//! Bits are `u8` values 0/1.

use super::crc::{crc16_x25, Crc16X25};

/// HDLC flag byte.
pub const FLAG: u8 = 0x7E;

/// Build the bit stream of one frame: `preamble` flags, the stuffed payload +
/// FCS, then `postamble` flags. (NRZI is applied separately.)
pub fn encode_frame(payload: &[u8], preamble: usize, postamble: usize) -> Vec<u8> {
    let mut bits = Vec::with_capacity((payload.len() + 2) * 10 + (preamble + postamble) * 8);
    let push_flag = |bits: &mut Vec<u8>| {
        for i in 0..8 {
            bits.push((FLAG >> i) & 1);
        }
    };
    for _ in 0..preamble.max(1) {
        push_flag(&mut bits);
    }
    let fcs = crc16_x25(payload).to_le_bytes();
    let mut ones = 0;
    for &byte in payload.iter().chain(fcs.iter()) {
        for i in 0..8 {
            let b = (byte >> i) & 1;
            bits.push(b);
            if b == 1 {
                ones += 1;
                if ones == 5 {
                    bits.push(0);
                    ones = 0;
                }
            } else {
                ones = 0;
            }
        }
    }
    for _ in 0..postamble.max(1) {
        push_flag(&mut bits);
    }
    bits
}

/// NRZI line coder/decoder state.
#[derive(Clone, Debug, Default)]
pub struct Nrzi {
    level: u8,
}

impl Nrzi {
    pub fn new() -> Nrzi {
        Nrzi::default()
    }

    /// Data bit -> line level (0 toggles, 1 holds).
    #[inline]
    pub fn encode(&mut self, bit: u8) -> u8 {
        if bit & 1 == 0 {
            self.level ^= 1;
        }
        self.level
    }

    /// Line level -> data bit.
    #[inline]
    pub fn decode(&mut self, level: u8) -> u8 {
        let bit = (level & 1 == self.level) as u8;
        self.level = level & 1;
        bit
    }

    pub fn encode_block(&mut self, bits: &[u8]) -> Vec<u8> {
        bits.iter().map(|&b| self.encode(b)).collect()
    }

    pub fn decode_block(&mut self, levels: &[u8]) -> Vec<u8> {
        levels.iter().map(|&l| self.decode(l)).collect()
    }
}

/// Streaming HDLC deframer. Feed de-NRZI'd bits; returns payloads (FCS
/// removed) of frames whose FCS checks.
#[derive(Clone, Debug)]
pub struct HdlcDecoder {
    ones: u32,
    in_frame: bool,
    bits: Vec<u8>,
    min_len: usize,
    max_len: usize,
    /// Frames rejected for a bad FCS (diagnostics).
    pub bad_fcs: u64,
}

impl Default for HdlcDecoder {
    fn default() -> Self {
        Self::new(1, 4096)
    }
}

impl HdlcDecoder {
    /// Accept payloads of `min_len..=max_len` bytes (excluding the 2-byte FCS).
    pub fn new(min_len: usize, max_len: usize) -> HdlcDecoder {
        HdlcDecoder { ones: 0, in_frame: false, bits: Vec::new(), min_len, max_len, bad_fcs: 0 }
    }

    /// Push one bit; returns a frame when a closing flag ends a valid one.
    pub fn push_bit(&mut self, bit: u8) -> Option<Vec<u8>> {
        if bit & 1 == 1 {
            self.ones += 1;
            if self.ones >= 7 {
                // abort / idle
                self.in_frame = false;
                self.bits.clear();
                return None;
            }
            if self.in_frame {
                self.bits.push(1);
            }
            return None;
        }
        let ones = self.ones;
        self.ones = 0;
        if ones == 6 {
            // flag: the six 1s (and the preceding 0) were appended; remove them.
            let mut result = None;
            if self.in_frame && self.bits.len() >= 7 {
                let n = self.bits.len() - 7;
                self.bits.truncate(n);
                result = self.finish();
            }
            self.in_frame = true;
            self.bits.clear();
            return result;
        }
        if ones == 5 {
            return None; // stuffed zero
        }
        if self.in_frame {
            self.bits.push(0);
            if self.bits.len() > (self.max_len + 2) * 8 + 8 {
                self.in_frame = false;
                self.bits.clear();
            }
        }
        None
    }

    fn finish(&mut self) -> Option<Vec<u8>> {
        if self.bits.len() % 8 != 0 {
            return None;
        }
        let nbytes = self.bits.len() / 8;
        if nbytes < self.min_len + 2 || nbytes > self.max_len + 2 {
            return None;
        }
        let bytes: Vec<u8> = self
            .bits
            .chunks(8)
            .map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (b << i)))
            .collect();
        let mut crc = Crc16X25::new();
        crc.update(&bytes[..nbytes - 2]);
        if crc.finish().to_le_bytes() == [bytes[nbytes - 2], bytes[nbytes - 1]] {
            Some(bytes[..nbytes - 2].to_vec())
        } else {
            self.bad_fcs += 1;
            None
        }
    }

    /// Push many bits, collecting frames.
    pub fn push_bits(&mut self, bits: &[u8]) -> Vec<Vec<u8>> {
        bits.iter().filter_map(|&b| self.push_bit(b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn stuffing_round_trip_with_nrzi() {
        // payload full of 0xFF / 0x7E to force stuffing
        let payload = vec![0xFF, 0x7E, 0x7E, 0xFF, 0x00, 0x3F, 0xFC, 0x1F];
        let bits = encode_frame(&payload, 3, 2);
        // no run of 6 ones inside the stuffed region
        let body = &bits[24..bits.len() - 16];
        let mut run = 0;
        for &b in body {
            run = if b == 1 { run + 1 } else { 0 };
            assert!(run <= 5);
        }
        let mut tx = Nrzi::new();
        let line = tx.encode_block(&bits);
        let mut rx = Nrzi::new();
        let dec = rx.decode_block(&line);
        let mut d = HdlcDecoder::default();
        let frames = d.push_bits(&dec);
        assert_eq!(frames, vec![payload]);
    }

    #[test]
    fn back_to_back_frames_and_corruption() {
        let mut bits = encode_frame(b"first", 2, 1);
        bits.extend(encode_frame(b"second frame", 0, 1));
        let mut bad = encode_frame(b"third", 1, 1);
        bad[20] ^= 1;
        bits.extend(bad);
        let mut d = HdlcDecoder::default();
        let frames = d.push_bits(&bits);
        assert_eq!(frames, vec![b"first".to_vec(), b"second frame".to_vec()]);
    }

    #[test]
    fn random_bits_do_not_produce_frames() {
        let mut rng = Rng::new(77);
        let mut d = HdlcDecoder::new(15, 330);
        let bits: Vec<u8> = (0..200_000).map(|_| rng.bit()).collect();
        assert!(d.push_bits(&bits).is_empty());
    }
}
