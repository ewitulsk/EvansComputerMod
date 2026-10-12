//! The UI protocol wire format (docs/gui/UI_PROTOCOL.md).
//!
//! Every message that crosses a UI file descriptor, with a strict encoder and
//! decoder. The message types live in `generated.rs`, produced by
//! `scripts/gen-ui-proto.py` from the frozen contract `abi/ui-protocol.toml`
//! (the same script writes the Java codec and the shared test vectors in
//! `abi/ui-test-vectors/`, which `tests/vectors.rs` round-trips).
//!
//! ```text
//! frame = u16 opcode · u16 flags (0) · u32 length · body[length]     little-endian
//! ```
//!
//! [`fonts`] holds the ECM font metrics (§6 "Fonts are part of the protocol").

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub mod fonts;
mod generated;
pub mod wire;

pub use generated::*;
pub use wire::DecodeError;

/// Bytes in a frame header.
pub const HEADER_LEN: usize = 8;
/// Node id of a tree surface's implicit root group.
pub const ROOT_NODE: u32 = 0;
/// `index` value that appends a node after its parent's last child.
pub const APPEND: u32 = u32::MAX;

/// `ui_open` roles and the `ui_upload` kinds (§3).
pub mod host {
    pub const ROLE_APP: i32 = 0;
    pub const ROLE_SHELL: i32 = 1;
    pub const UPLOAD_PNG: i32 = 0;
    pub const UPLOAD_RGBA: i32 = 1;
}

impl Message {
    /// Append this message (header and body) to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&self.opcode().to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        self.enc_body(&mut wire::Writer::new(out));
        let len = (out.len() - start - HEADER_LEN) as u32;
        out[start + 4..start + 8].copy_from_slice(&len.to_le_bytes());
    }

    /// This message as a new byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.encode(&mut v);
        v
    }

    /// Decode the first message in `buf`. Returns it and the bytes it used;
    /// [`DecodeError::Incomplete`] means more bytes are needed (only possible
    /// when the header itself is valid so far).
    pub fn decode(buf: &[u8]) -> Result<(Message, usize), DecodeError> {
        if buf.len() < HEADER_LEN {
            return Err(DecodeError::Incomplete(HEADER_LEN));
        }
        let opcode = u16::from_le_bytes([buf[0], buf[1]]);
        let flags = u16::from_le_bytes([buf[2], buf[3]]);
        let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if flags != 0 {
            return Err(DecodeError::HeaderFlags);
        }
        if !Message::is_known(opcode) {
            return Err(DecodeError::UnknownOpcode(opcode));
        }
        if len > MAX_MESSAGE {
            return Err(DecodeError::TooLarge(len));
        }
        let total = HEADER_LEN + len as usize;
        if buf.len() < total {
            return Err(DecodeError::Incomplete(total));
        }
        let mut r = wire::Reader::new(&buf[HEADER_LEN..total]);
        let msg = Message::dec_body(opcode, &mut r)?;
        r.finish()?;
        Ok((msg, total))
    }

    /// Decode a buffer that must hold exactly one whole message.
    pub fn decode_exact(buf: &[u8]) -> Result<Message, DecodeError> {
        let (m, n) = Message::decode(buf)?;
        if n != buf.len() {
            return Err(DecodeError::Trailing);
        }
        Ok(m)
    }

    /// Decode a buffer of whole messages (what `ui_recv` returns).
    pub fn decode_all(mut buf: &[u8]) -> Result<Vec<Message>, DecodeError> {
        let mut out = Vec::new();
        while !buf.is_empty() {
            let (m, n) = Message::decode(buf)?;
            out.push(m);
            buf = &buf[n..];
        }
        Ok(out)
    }

    /// Canonical JSON of this message (the format of the shared test vectors).
    pub fn to_json(&self) -> String {
        let mut o = String::new();
        self.json_into(&mut o);
        o
    }
}

/// Encode several messages into one buffer (one `ui_send`).
pub fn encode_all<'a>(msgs: impl IntoIterator<Item = &'a Message>) -> Vec<u8> {
    let mut out = Vec::new();
    for m in msgs {
        m.encode(&mut out);
    }
    out
}

/// Accumulates bytes from a stream and yields whole messages.
#[derive(Default)]
pub struct StreamDecoder {
    buf: Vec<u8>,
}

impl StreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, `Ok(None)` if more bytes are needed, or the
    /// protocol error that must end the connection.
    pub fn next_message(&mut self) -> Result<Option<Message>, DecodeError> {
        match Message::decode(&self.buf) {
            Ok((m, n)) => {
                self.buf.drain(..n);
                Ok(Some(m))
            }
            Err(DecodeError::Incomplete(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Theme {
    /// The value of token `key`, if the theme sets it.
    pub fn get(&self, key: &str) -> Option<u32> {
        self.entries
            .binary_search_by(|e| e.key.as_bytes().cmp(key.as_bytes()))
            .ok()
            .map(|i| self.entries[i].value)
    }

    /// Set a token, keeping the entries sorted (as the wire format requires).
    pub fn set(&mut self, key: &str, value: u32) {
        match self.entries.binary_search_by(|e| e.key.as_bytes().cmp(key.as_bytes())) {
            Ok(i) => self.entries[i].value = value,
            Err(i) => self.entries.insert(i, ThemeEntry { key: String::from(key), value }),
        }
    }
}

impl SizeClass {
    /// The size class of a surface `width` lu wide (§4).
    pub fn for_width(width: i32) -> SizeClass {
        if width >= 800 {
            SizeClass::Wide
        } else if width >= 400 {
            SizeClass::Medium
        } else {
            SizeClass::Compact
        }
    }
}
