//! KISS TNC framing (FEND-delimited, with FESC escaping).

pub const FEND: u8 = 0xC0;
pub const FESC: u8 = 0xDB;
pub const TFEND: u8 = 0xDC;
pub const TFESC: u8 = 0xDD;

/// KISS command nibble values.
pub mod cmd {
    pub const DATA: u8 = 0x0;
    pub const TX_DELAY: u8 = 0x1;
    pub const PERSISTENCE: u8 = 0x2;
    pub const SLOT_TIME: u8 = 0x3;
    pub const TX_TAIL: u8 = 0x4;
    pub const FULL_DUPLEX: u8 = 0x5;
    pub const SET_HARDWARE: u8 = 0x6;
    pub const RETURN: u8 = 0xF;
}

/// One KISS frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KissFrame {
    pub port: u8,
    pub command: u8,
    pub data: Vec<u8>,
}

impl KissFrame {
    pub fn data(port: u8, data: &[u8]) -> KissFrame {
        KissFrame { port, command: cmd::DATA, data: data.to_vec() }
    }

    /// Serialise as `FEND type escaped-data FEND`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() + 4);
        out.push(FEND);
        push_escaped(&mut out, ((self.port & 0x0F) << 4) | (self.command & 0x0F));
        for &b in &self.data {
            push_escaped(&mut out, b);
        }
        out.push(FEND);
        out
    }
}

fn push_escaped(out: &mut Vec<u8>, b: u8) {
    match b {
        FEND => out.extend_from_slice(&[FESC, TFEND]),
        FESC => out.extend_from_slice(&[FESC, TFESC]),
        _ => out.push(b),
    }
}

/// Streaming KISS decoder (byte at a time).
#[derive(Clone, Debug, Default)]
pub struct KissDecoder {
    buf: Vec<u8>,
    esc: bool,
    in_frame: bool,
}

impl KissDecoder {
    pub fn new() -> KissDecoder {
        KissDecoder::default()
    }

    pub fn push(&mut self, b: u8) -> Option<KissFrame> {
        if b == FEND {
            let out = if self.in_frame && !self.buf.is_empty() {
                let t = self.buf[0];
                Some(KissFrame { port: t >> 4, command: t & 0x0F, data: self.buf[1..].to_vec() })
            } else {
                None
            };
            self.buf.clear();
            self.esc = false;
            self.in_frame = true;
            return out;
        }
        if !self.in_frame {
            return None;
        }
        if self.esc {
            self.esc = false;
            match b {
                TFEND => self.buf.push(FEND),
                TFESC => self.buf.push(FESC),
                _ => self.buf.push(b), // protocol violation: pass through
            }
        } else if b == FESC {
            self.esc = true;
        } else {
            self.buf.push(b);
        }
        None
    }

    pub fn push_bytes(&mut self, data: &[u8]) -> Vec<KissFrame> {
        data.iter().filter_map(|&b| self.push(b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_round_trip() {
        let data = vec![0x00, FEND, 0x11, FESC, FESC, FEND, TFEND, TFESC, 0xFF];
        let f = KissFrame::data(3, &data);
        let enc = f.encode();
        // no raw FEND inside the frame
        assert!(enc[1..enc.len() - 1].iter().all(|&b| b != FEND));
        assert_eq!(enc[1], 0x30);
        let mut d = KissDecoder::new();
        let mut stream = vec![0x55, 0x66]; // garbage before first FEND is ignored
        stream.extend(&enc);
        stream.extend(KissFrame { port: 0, command: cmd::TX_DELAY, data: vec![50] }.encode());
        let frames = d.push_bytes(&stream);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], f);
        assert_eq!(frames[1].command, cmd::TX_DELAY);
        assert_eq!(frames[1].data, vec![50]);
        // split across calls
        let mut d2 = KissDecoder::new();
        let (a, b) = enc.split_at(5);
        assert!(d2.push_bytes(a).is_empty());
        assert_eq!(d2.push_bytes(b), vec![f]);
    }
}
