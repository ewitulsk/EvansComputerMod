//! AX.25 UI frames (the APRS / packet-radio datagram).
//!
//! `UiFrame::encode` produces the frame body without FCS; HDLC adds the FCS
//! (see [`super::hdlc::encode_frame`]).

use std::fmt;

/// A callsign + SSID, e.g. `N0CALL-7`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub call: String,
    pub ssid: u8,
    /// "Has been repeated" (H) bit for digipeaters / command-response (C) bit.
    pub flag: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ax25Error {
    BadCallsign,
    TooShort,
    NotUi,
    TooManyDigipeaters,
}

impl fmt::Display for Ax25Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for Ax25Error {}

impl Address {
    pub fn new(call: &str, ssid: u8) -> Result<Address, Ax25Error> {
        let call = call.to_ascii_uppercase();
        if call.is_empty() || call.len() > 6 || !call.bytes().all(|b| b.is_ascii_alphanumeric()) || ssid > 15 {
            return Err(Ax25Error::BadCallsign);
        }
        Ok(Address { call, ssid, flag: false })
    }

    /// Parse `CALL` or `CALL-SSID`.
    pub fn parse(s: &str) -> Result<Address, Ax25Error> {
        match s.split_once('-') {
            Some((c, n)) => Address::new(c, n.parse().map_err(|_| Ax25Error::BadCallsign)?),
            None => Address::new(s, 0),
        }
    }

    fn encode(&self, last: bool) -> [u8; 7] {
        let mut out = [b' ' << 1; 7];
        for (i, b) in self.call.bytes().enumerate() {
            out[i] = b << 1;
        }
        out[6] = 0x60 | (self.ssid << 1) | (last as u8) | ((self.flag as u8) << 7);
        out
    }

    fn decode(b: &[u8]) -> Result<(Address, bool), Ax25Error> {
        let mut call = String::new();
        for &c in &b[..6] {
            if c & 1 != 0 {
                return Err(Ax25Error::BadCallsign);
            }
            let ch = c >> 1;
            if ch != b' ' {
                call.push(ch as char);
            }
        }
        if call.is_empty() || !call.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err(Ax25Error::BadCallsign);
        }
        Ok((Address { call, ssid: (b[6] >> 1) & 0x0F, flag: b[6] & 0x80 != 0 }, b[6] & 1 != 0))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ssid == 0 {
            write!(f, "{}", self.call)
        } else {
            write!(f, "{}-{}", self.call, self.ssid)
        }
    }
}

/// An unnumbered-information frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiFrame {
    pub dest: Address,
    pub src: Address,
    pub digis: Vec<Address>,
    /// Protocol ID; 0xF0 = no layer 3 (APRS), 0xCC = IP.
    pub pid: u8,
    pub info: Vec<u8>,
}

/// Control field for UI frames.
pub const CONTROL_UI: u8 = 0x03;

impl UiFrame {
    pub fn new(dest: Address, src: Address, info: &[u8]) -> UiFrame {
        UiFrame { dest, src, digis: Vec::new(), pid: 0xF0, info: info.to_vec() }
    }

    /// Encode (without FCS). The destination carries C=1, the source C=0 (command frame).
    pub fn encode(&self) -> Result<Vec<u8>, Ax25Error> {
        if self.digis.len() > 8 {
            return Err(Ax25Error::TooManyDigipeaters);
        }
        let mut v = Vec::with_capacity(16 + 7 * self.digis.len() + self.info.len());
        let mut d = self.dest.clone();
        d.flag = true;
        v.extend(d.encode(false));
        let mut s = self.src.clone();
        s.flag = false;
        v.extend(s.encode(self.digis.is_empty()));
        for (i, dg) in self.digis.iter().enumerate() {
            v.extend(dg.encode(i + 1 == self.digis.len()));
        }
        v.push(CONTROL_UI);
        v.push(self.pid);
        v.extend_from_slice(&self.info);
        Ok(v)
    }

    /// Parse a frame body (FCS already removed).
    pub fn parse(b: &[u8]) -> Result<UiFrame, Ax25Error> {
        if b.len() < 16 {
            return Err(Ax25Error::TooShort);
        }
        let (dest, l0) = Address::decode(&b[0..7])?;
        if l0 {
            return Err(Ax25Error::BadCallsign);
        }
        let (src, mut last) = Address::decode(&b[7..14])?;
        let mut pos = 14;
        let mut digis = Vec::new();
        while !last {
            if digis.len() >= 8 {
                return Err(Ax25Error::TooManyDigipeaters);
            }
            if b.len() < pos + 7 + 2 {
                return Err(Ax25Error::TooShort);
            }
            let (a, l) = Address::decode(&b[pos..pos + 7])?;
            digis.push(a);
            last = l;
            pos += 7;
        }
        if b.len() < pos + 2 {
            return Err(Ax25Error::TooShort);
        }
        if b[pos] & !0x10 != CONTROL_UI {
            return Err(Ax25Error::NotUi);
        }
        Ok(UiFrame { dest, src, digis, pid: b[pos + 1], info: b[pos + 2..].to_vec() })
    }
}

impl fmt::Display for UiFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}>{}", self.src, self.dest)?;
        for d in &self.digis {
            write!(f, ",{}{}", d, if d.flag { "*" } else { "" })?;
        }
        write!(f, ":{}", String::from_utf8_lossy(&self.info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::hdlc::{encode_frame, HdlcDecoder, Nrzi};

    #[test]
    fn ui_frame_round_trip_through_hdlc() {
        let mut f = UiFrame::new(Address::parse("APRS").unwrap(), Address::parse("n0call-9").unwrap(), b"!4903.50N/07201.75W-Test");
        f.digis.push(Address::parse("WIDE1-1").unwrap());
        let body = f.encode().unwrap();
        // known layout of the first address: 'A'<<1 = 0x82
        assert_eq!(body[0], 0x82);
        assert_eq!(body[6], 0xE0); // C bit + reserved, ssid 0, not last
        assert_eq!(body[13], 0x60 | (9 << 1)); // src: not last (digi follows)
        assert_eq!(body[20], 0x60 | (1 << 1) | 1); // last address
        let bits = encode_frame(&body, 4, 2);
        let line = Nrzi::new().encode_block(&bits);
        let dec = Nrzi::new().decode_block(&line);
        let frames = HdlcDecoder::new(15, 330).push_bits(&dec);
        assert_eq!(frames.len(), 1);
        let g = UiFrame::parse(&frames[0]).unwrap();
        assert_eq!(g.src.to_string(), "N0CALL-9");
        assert_eq!(g.to_string(), "N0CALL-9>APRS,WIDE1-1:!4903.50N/07201.75W-Test");
        assert_eq!(g.info, f.info);
        assert!(Address::parse("TOOLONGCALL").is_err());
        assert_eq!(UiFrame::parse(&body[..10]), Err(Ax25Error::TooShort));
    }
}
