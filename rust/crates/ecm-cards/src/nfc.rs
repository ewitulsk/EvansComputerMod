//! The NFC applet's "Sector Lattice" (NFC_MIX.md): `Mix`, the factory key,
//! the 3-pass sector authentication values and the `NF-106` air frames.
//!
//! ```text
//! R → C: AUTH i, R1 (8 random bytes)
//! C → R: R2 (8 random bytes), M1 = Mix(S_i, R1 || R2)
//! R → C: M2 = Mix(S_i, R2 || R1)
//! C: session for sector i open iff M2 matches
//! ```

use alloc::vec::Vec;

/// Every sector of a blank card: ASCII `ECM-NFC-FACTORY0`.
pub const FACTORY_KEY: [u8; 16] = *b"ECM-NFC-FACTORY0";

pub const SECTORS: usize = 8;
pub const SECTOR_BYTES: usize = 64;
pub const KEY_BYTES: usize = 16;

/// `Mix(key, x)`: 8 rounds of add-rotate-xor over four little-endian 32-bit words.
pub fn mix(key: &[u8; 16], x: &[u8; 16]) -> [u8; 16] {
    let w = |b: &[u8; 16], i: usize| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
    let k = [w(key, 0), w(key, 1), w(key, 2), w(key, 3)];
    let (mut v0, mut v1, mut v2, mut v3) = (w(x, 0), w(x, 1), w(x, 2), w(x, 3));
    for r in 0..8u32 {
        let ri = r as usize;
        v0 = v0.wrapping_add(k[ri & 3]);
        v1 ^= v0.rotate_left(7);
        v2 = v2.wrapping_add(v1);
        v3 ^= v2.rotate_left(13);
        v0 = v0.wrapping_add(v3);
        v1 = v1.rotate_left(17).wrapping_add(k[(ri + 1) & 3]);
        v2 ^= v0.rotate_left(5);
        v3 = (v3 ^ r).wrapping_add(k[(ri + 2) & 3]);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&v0.to_le_bytes());
    out[4..8].copy_from_slice(&v1.to_le_bytes());
    out[8..12].copy_from_slice(&v2.to_le_bytes());
    out[12..16].copy_from_slice(&v3.to_le_bytes());
    out
}

fn cat(a: &[u8; 8], b: &[u8; 8]) -> [u8; 16] {
    let mut x = [0u8; 16];
    x[..8].copy_from_slice(a);
    x[8..].copy_from_slice(b);
    x
}

/// The card's answer `M1 = Mix(S_i, R1 || R2)`.
pub fn auth_m1(key: &[u8; 16], r1: &[u8; 8], r2: &[u8; 8]) -> [u8; 16] {
    mix(key, &cat(r1, r2))
}

/// The reader's answer `M2 = Mix(S_i, R2 || R1)`.
pub fn auth_m2(key: &[u8; 16], r1: &[u8; 8], r2: &[u8; 8]) -> [u8; 16] {
    mix(key, &cat(r2, r1))
}

// ---------------------------------------------------------------- air frames

pub const MODULATION: &str = "NF-106";
pub const BIT_RATE: u32 = 106_000;
pub const CENTER_HZ: f64 = 13.56e6;
pub const CHANNEL_HZ: f64 = 14e3;

pub const CMD_POLL: u8 = 0x01;
pub const CMD_SELECT: u8 = 0x02;
pub const CMD_AUTH: u8 = 0x03;
pub const CMD_AUTH2: u8 = 0x04;
pub const CMD_READ: u8 = 0x05;
pub const CMD_WRITE: u8 = 0x06;
pub const CMD_SETKEY: u8 = 0x07;
pub const CMD_ERROR: u8 = 0xFF;
/// A reply's cmd is the request's cmd with this bit set (0x81 answers 0x01).
pub const REPLY_BIT: u8 = 0x80;

pub const ERR_BAD_AUTH: u8 = 1;
pub const ERR_NO_SESSION: u8 = 2;
pub const ERR_BAD_SECTOR: u8 = 3;
pub const ERR_NOT_SELECTED: u8 = 4;
pub const ERR_MALFORMED: u8 = 5;

/// Card kinds bitmask in a POLL reply.
pub const KIND_BANK: u8 = 1;
pub const KIND_NFC: u8 = 2;
pub const KIND_SIM: u8 = 4;

pub fn cmd_name(cmd: u8) -> &'static str {
    match cmd & !REPLY_BIT {
        CMD_POLL => "POLL",
        CMD_SELECT => "SELECT",
        CMD_AUTH => "AUTH",
        CMD_AUTH2 => "AUTH2",
        CMD_READ => "READ",
        CMD_WRITE => "WRITE",
        CMD_SETKEY => "SETKEY",
        0x7F if cmd == CMD_ERROR => "ERROR",
        _ => "?",
    }
}

pub fn reason_name(reason: u8) -> &'static str {
    match reason {
        ERR_BAD_AUTH => "bad auth",
        ERR_NO_SESSION => "no session",
        ERR_BAD_SECTOR => "bad sector",
        ERR_NOT_SELECTED => "not selected",
        ERR_MALFORMED => "malformed",
        _ => "?",
    }
}

/// `cmd(1) len(1) body(len)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub cmd: u8,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer than 2 bytes.
    Short,
    /// `len` says more than the frame holds.
    Truncated,
    /// Bytes after `body`.
    Trailing,
    /// A body longer than 255 bytes can't be encoded.
    TooLong,
}

impl Frame {
    pub fn new(cmd: u8, body: &[u8]) -> Frame {
        Frame { cmd, body: body.to_vec() }
    }
    pub fn poll() -> Frame {
        Frame::new(CMD_POLL, &[])
    }
    pub fn select(uid: &[u8; 8]) -> Frame {
        Frame::new(CMD_SELECT, uid)
    }
    pub fn auth(sector: u8, r1: &[u8; 8]) -> Frame {
        let mut b = Vec::with_capacity(9);
        b.push(sector);
        b.extend_from_slice(r1);
        Frame { cmd: CMD_AUTH, body: b }
    }
    pub fn auth2(m2: &[u8; 16]) -> Frame {
        Frame::new(CMD_AUTH2, m2)
    }
    pub fn read(sector: u8) -> Frame {
        Frame::new(CMD_READ, &[sector])
    }
    pub fn write(sector: u8, data: &[u8; SECTOR_BYTES]) -> Frame {
        let mut b = Vec::with_capacity(1 + SECTOR_BYTES);
        b.push(sector);
        b.extend_from_slice(data);
        Frame { cmd: CMD_WRITE, body: b }
    }
    pub fn set_key(sector: u8, key: &[u8; 16]) -> Frame {
        let mut b = Vec::with_capacity(17);
        b.push(sector);
        b.extend_from_slice(key);
        Frame { cmd: CMD_SETKEY, body: b }
    }
    pub fn error(reason: u8) -> Frame {
        Frame::new(CMD_ERROR, &[reason])
    }
    /// The reply to `self` (cmd with [`REPLY_BIT`]).
    pub fn reply(&self, body: &[u8]) -> Frame {
        Frame::new(self.cmd | REPLY_BIT, body)
    }
    /// The POLL reply of a card: `0x81 UID(8) kinds(1)`.
    pub fn poll_reply(uid: &[u8; 8], kinds: u8) -> Frame {
        let mut b = Vec::with_capacity(9);
        b.extend_from_slice(uid);
        b.push(kinds);
        Frame { cmd: CMD_POLL | REPLY_BIT, body: b }
    }
    /// The AUTH reply of a card: `0x83 R2(8) M1(16)`.
    pub fn auth_reply(r2: &[u8; 8], m1: &[u8; 16]) -> Frame {
        let mut b = Vec::with_capacity(24);
        b.extend_from_slice(r2);
        b.extend_from_slice(m1);
        Frame { cmd: CMD_AUTH | REPLY_BIT, body: b }
    }

    pub fn is_reply(&self) -> bool {
        self.cmd & REPLY_BIT != 0 && self.cmd != CMD_ERROR
    }
    /// The reason of an ERROR frame (0 when it has no body).
    pub fn error_reason(&self) -> Option<u8> {
        (self.cmd == CMD_ERROR).then(|| self.body.first().copied().unwrap_or(0))
    }
    /// The anticollision slot (0..16) a card with `uid` answers POLL in.
    pub fn poll_slot(uid: &[u8; 8]) -> u8 {
        uid[7] & 15
    }

    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        if self.body.len() > 255 {
            return Err(FrameError::TooLong);
        }
        let mut out = Vec::with_capacity(2 + self.body.len());
        out.push(self.cmd);
        out.push(self.body.len() as u8);
        out.extend_from_slice(&self.body);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
        if bytes.len() < 2 {
            return Err(FrameError::Short);
        }
        let len = bytes[1] as usize;
        if bytes.len() < 2 + len {
            return Err(FrameError::Truncated);
        }
        if bytes.len() > 2 + len {
            return Err(FrameError::Trailing);
        }
        Ok(Frame { cmd: bytes[0], body: bytes[2..].to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex;

    fn k16(s: &str) -> [u8; 16] {
        hex::decode_array(s).unwrap()
    }

    /// NFC_MIX.md "Mix vectors".
    #[test]
    fn mix_vectors() {
        let f = hex::encode(&FACTORY_KEY);
        assert_eq!(f, "45434d2d4e46432d464143544f525930");
        for (key, x, want) in [
            (f.as_str(), "00000000000000000000000000000000", "3e2e68d5726a6858461251213df31d59"),
            (f.as_str(), "000102030405060708090a0b0c0d0e0f", "e8938dca0b939828c0b5e18c388b9702"),
            ("00112233445566778899aabbccddeeff", "01020304050607081112131415161718", "f2668ac938caea225f16cf78a07cfe81"),
            ("00000000000000000000000000000000", "00000000000000000000000000000000", "5d4fb41a84bab1a9fa2416dcc38b9978"),
            ("ffffffffffffffffffffffffffffffff", "ffffffffffffffffffffffffffffffff", "355b79459dc55581fc26ab3eb2eb6ce6"),
        ] {
            assert_eq!(hex::encode(&mix(&k16(key), &k16(x))), want, "Mix({key}, {x})");
        }
    }

    #[test]
    fn auth_values_are_mix_of_the_concatenations() {
        let r1 = [1u8; 8];
        let r2 = [2u8; 8];
        assert_eq!(auth_m1(&FACTORY_KEY, &r1, &r2), mix(&FACTORY_KEY, &k16("01010101010101010202020202020202")));
        assert_eq!(auth_m2(&FACTORY_KEY, &r1, &r2), mix(&FACTORY_KEY, &k16("02020202020202020101010101010101")));
        assert_ne!(auth_m1(&FACTORY_KEY, &r1, &r2), auth_m2(&FACTORY_KEY, &r1, &r2));
    }

    #[test]
    fn frames_round_trip() {
        let uid = [1, 2, 3, 4, 5, 6, 7, 0x1a];
        let frames = [
            Frame::poll(),
            Frame::select(&uid),
            Frame::auth(3, &[9; 8]),
            Frame::auth2(&[7; 16]),
            Frame::read(1),
            Frame::write(2, &[0xab; SECTOR_BYTES]),
            Frame::set_key(7, &FACTORY_KEY),
            Frame::error(ERR_NO_SESSION),
            Frame::poll_reply(&uid, KIND_NFC | KIND_BANK),
            Frame::auth_reply(&[5; 8], &[6; 16]),
            Frame::read(1).reply(&[0; 64]),
        ];
        for f in &frames {
            let bytes = f.encode().unwrap();
            assert_eq!(bytes[0], f.cmd);
            assert_eq!(bytes[1] as usize, f.body.len());
            assert_eq!(Frame::decode(&bytes).unwrap(), *f);
        }
        assert_eq!(Frame::poll().encode().unwrap(), [0x01, 0x00]);
        assert_eq!(Frame::error(ERR_BAD_AUTH).encode().unwrap(), [0xFF, 0x01, 0x01]);
        assert_eq!(Frame::poll_reply(&uid, KIND_NFC).cmd, 0x81);
        assert_eq!(Frame::poll_slot(&uid), 10);
        assert_eq!(Frame::error(ERR_BAD_SECTOR).error_reason(), Some(ERR_BAD_SECTOR));
        assert_eq!(Frame::poll().error_reason(), None);
        assert!(Frame::poll_reply(&uid, 0).is_reply() && !Frame::error(1).is_reply());
        assert_eq!(Frame::decode(&[0x05]), Err(FrameError::Short));
        assert_eq!(Frame::decode(&[0x05, 0x02, 0x01]), Err(FrameError::Truncated));
        assert_eq!(Frame::decode(&[0x05, 0x01, 0x01, 0x00]), Err(FrameError::Trailing));
        assert_eq!(Frame::new(1, &[0; 256]).encode(), Err(FrameError::TooLong));
        assert_eq!(cmd_name(0x83), "AUTH");
        assert_eq!(cmd_name(CMD_ERROR), "ERROR");
        assert_eq!(reason_name(ERR_NOT_SELECTED), "not selected");
    }
}
