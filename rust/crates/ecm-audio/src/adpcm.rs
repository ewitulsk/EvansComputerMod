//! IMA ADPCM, 4 bits per sample (4:1 against 16-bit PCM), mono: the same
//! self-contained block format the mod's Java side uses for speaker audio
//! (`speaker/ImaAdpcm.java`), so a block encoded here decodes there and the
//! other way round.
//!
//! A block is `[i16 LE first sample][u8 step index][u8 0][nibbles, low nibble
//! first]`: the first sample is stored verbatim and the remaining `n - 1`
//! samples take one nibble each. A lost or reordered block only costs its own
//! samples.

const INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130,
    143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282,
    1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630,
    9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// Bytes before the nibbles.
pub const HEADER_BYTES: usize = 4;

/// Encoded size of a block of `samples` samples.
pub fn encoded_size(samples: usize) -> usize {
    if samples == 0 {
        0
    } else {
        HEADER_BYTES + samples / 2
    }
}

/// An encoder: keeps the step size adapted from one block to the next.
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    index: i32,
}

impl Encoder {
    pub fn new() -> Encoder {
        Encoder::default()
    }

    /// Encode `pcm` into one block.
    pub fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        if pcm.is_empty() {
            return Vec::new();
        }
        let mut out = vec![0u8; encoded_size(pcm.len())];
        let mut predictor = pcm[0] as i32;
        out[0] = predictor as u8;
        out[1] = (predictor >> 8) as u8;
        out[2] = self.index as u8;
        let mut step = STEP_TABLE[self.index as usize];
        for i in 1..pcm.len() {
            let mut diff = pcm[i] as i32 - predictor;
            let mut nibble = 0;
            if diff < 0 {
                nibble = 8;
                diff = -diff;
            }
            let mut delta = step >> 3;
            if diff >= step {
                nibble |= 4;
                diff -= step;
                delta += step;
            }
            if diff >= step >> 1 {
                nibble |= 2;
                diff -= step >> 1;
                delta += step >> 1;
            }
            if diff >= step >> 2 {
                nibble |= 1;
                delta += step >> 2;
            }
            predictor = clamp16(if nibble & 8 != 0 { predictor - delta } else { predictor + delta });
            self.index = (self.index + INDEX_TABLE[nibble as usize]).clamp(0, 88);
            step = STEP_TABLE[self.index as usize];
            let pos = HEADER_BYTES + (i - 1) / 2;
            if (i - 1) & 1 == 0 {
                out[pos] = nibble as u8;
            } else {
                out[pos] |= (nibble << 4) as u8;
            }
        }
        out
    }
}

/// Decode a block of `count` samples; None if it is malformed.
pub fn decode(block: &[u8], count: usize) -> Option<Vec<i16>> {
    if count == 0 || count > 65536 || block.len() < encoded_size(count) {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut predictor = i16::from_le_bytes([block[0], block[1]]) as i32;
    let mut index = block[2] as i32;
    if index > 88 {
        return None;
    }
    out.push(predictor as i16);
    let mut step = STEP_TABLE[index as usize];
    for i in 1..count {
        let b = block[HEADER_BYTES + (i - 1) / 2];
        let nibble = if (i - 1) & 1 == 0 { b & 0x0f } else { b >> 4 } as i32;
        let mut delta = step >> 3;
        if nibble & 4 != 0 {
            delta += step;
        }
        if nibble & 2 != 0 {
            delta += step >> 1;
        }
        if nibble & 1 != 0 {
            delta += step >> 2;
        }
        predictor = clamp16(if nibble & 8 != 0 { predictor - delta } else { predictor + delta });
        index = (index + INDEX_TABLE[nibble as usize]).clamp(0, 88);
        step = STEP_TABLE[index as usize];
        out.push(predictor as i16);
    }
    Some(out)
}

fn clamp16(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, rate: f64, hz: f64, amp: f64) -> Vec<i16> {
        (0..n).map(|i| ((i as f64 * hz * std::f64::consts::TAU / rate).sin() * amp) as i16).collect()
    }

    #[test]
    fn round_trip_is_close_and_sizes_match() {
        let pcm = tone(640, 16000.0, 440.0, 12000.0);
        let mut enc = Encoder::new();
        let block = enc.encode(&pcm);
        assert_eq!(block.len(), encoded_size(640));
        assert_eq!(block.len(), 4 + 320);
        // The second block starts with the step size the first one adapted to.
        let block2 = enc.encode(&pcm);
        let out = decode(&block2, 640).unwrap();
        let err: f64 = pcm.iter().zip(&out).map(|(a, b)| ((*a as f64) - (*b as f64)).abs()).sum::<f64>() / 640.0;
        assert!(err < 400.0, "mean error {err}");
        assert_eq!(out[0], pcm[0], "the first sample is verbatim");
    }

    #[test]
    fn malformed_blocks_are_refused() {
        assert!(decode(&[0, 0, 0, 0], 0).is_none());
        assert!(decode(&[0, 0, 0, 0], 100).is_none(), "too short");
        assert!(decode(&[0, 0, 99, 0, 0], 3).is_none(), "bad step index");
        assert!(Encoder::new().encode(&[]).is_empty());
    }

    #[test]
    fn header_and_predictor_track() {
        let mut enc = Encoder::new();
        let b = enc.encode(&[0, 1000, 2000, -500, 0]);
        assert_eq!(&b[..4], &[0, 0, 0, 0]);
        assert_eq!(b.len(), 6);
        // Decoding our own block reproduces the encoder's predictor track.
        let d = decode(&b, 5).unwrap();
        assert_eq!(d[0], 0);
        assert!(d[1] > 0 && d[2] > d[1] && d[3] < d[2]);
    }
}
