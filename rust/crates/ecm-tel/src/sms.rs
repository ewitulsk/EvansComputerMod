//! 3GPP short messages: TS 23.038 alphabets, TS 23.040 TPDUs and TS 24.011 RP
//! messages, the way IMS carries them in SIP MESSAGE bodies (TS 24.341).
//!
//! **Alphabets.** Text goes out in the GSM 7-bit default alphabet when every
//! character is in it (basic table, or the extension table behind escape 0x1B:
//! `^ { } \ [ ~ ] | €` and form feed, two septets each), otherwise in UCS-2
//! (UTF-16BE; characters outside the BMP as surrogate pairs). Septets pack
//! LSB-first into octets ([`pack_septets`]); when a user data header precedes
//! 7-bit text, fill bits pad the header to a septet boundary.
//!
//! **TPDUs** ([`Tpdu`]), first octet bit 0-1 = TP-MTI:
//!
//! ```text
//! SMS-SUBMIT        (MS->SC, MTI 01): fo(RD,VPF,SRR,UDHI,RP) MR DA PID DCS [VP] UDL UD
//! SMS-DELIVER       (SC->MS, MTI 00): fo(MMS,SRI,UDHI,RP) OA PID DCS SCTS(7) UDL UD
//! SMS-STATUS-REPORT (SC->MS, MTI 10): fo(MMS,SRQ,UDHI) MR RA SCTS(7) DT(7) ST [PI [PID] [DCS] [UDL UD]]
//! ```
//! Addresses are `[len][TOA][value]`: `len` counts useful semi-octets, TOA is
//! `1 TON(3) NPI(4)` (0x91 international, 0x81 unknown, 0xA1 national), the
//! value BCD semi-octets low nibble first with an F filler, or for TON 5
//! (0xD0) GSM 7-bit packed text such as `CARRIER`. TP-DCS emitted is 0x00
//! (GSM 7-bit), 0x08 (UCS-2) or 0x04 (8-bit data, WAP push); the decoder
//! accepts the general and message-waiting coding groups. TP-UDL counts
//! septets for 7-bit (header included) and octets otherwise; TP-UD is at most
//! 140 octets. Timestamps are seven swapped-BCD octets YY MM DD hh mm ss tz, tz
//! in quarter hours with bit 3 of the octet as the minus sign.
//!
//! **User data header** ([`Ie`]): `[UDHL][IEI len data]...`. Concatenation is
//! IEI 0x00 (8-bit ref, total, seq) or 0x08 (16-bit ref); application ports
//! IEI 0x05 (16-bit destination, source; WAP push goes to port 2948).
//! [`split_text`] cuts long text into 153-septet / 67-unit parts (single
//! part 160 / 70) without splitting an escape pair or a surrogate pair, and
//! [`Reassembler`] joins parts arriving in any order.
//!
//! **RP layer** ([`Rp`], TS 24.011 7.3), RP-MTI (3 bits, even = MS->network):
//!
//! ```text
//! RP-DATA  0/1: MTI MR [OA len][OA] [DA len][DA] [UD len][TPDU]
//!          MS->net: OA empty, DA = SMSC; net->MS: OA = SMSC, DA empty
//! RP-ACK   2/3: MTI MR [0x41 len TPDU]
//! RP-ERROR 4/5: MTI MR [cause len 1-2][cause][diagnostic] [0x41 len TPDU]
//! RP-SMMA  6:   MTI MR
//! ```
//! RP addresses are `[octet len][TOA][BCD]`; length 0 means absent.
//!
//! Every decoder is strict: it returns an [`SmsError`] for truncated,
//! trailing, malformed or unsupported input and never panics.

use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Errors and a byte reader

/// Why a decode failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmsError {
    /// The input ended inside a field.
    Truncated,
    /// Bytes follow the end of the message.
    Trailing,
    /// A field holds a value the spec forbids.
    Invalid(&'static str),
    /// A valid message type or coding this module does not handle.
    Unsupported(&'static str),
}

impl fmt::Display for SmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmsError::Truncated => f.write_str("sms: truncated"),
            SmsError::Trailing => f.write_str("sms: trailing bytes"),
            SmsError::Invalid(w) => write!(f, "sms: invalid {w}"),
            SmsError::Unsupported(w) => write!(f, "sms: unsupported {w}"),
        }
    }
}

impl std::error::Error for SmsError {}

pub type SmsResult<T> = Result<T, SmsError>;

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Rd { b, p: 0 }
    }
    fn u8(&mut self) -> SmsResult<u8> {
        let v = *self.b.get(self.p).ok_or(SmsError::Truncated)?;
        self.p += 1;
        Ok(v)
    }
    fn take(&mut self, n: usize) -> SmsResult<&'a [u8]> {
        if self.b.len() - self.p < n {
            return Err(SmsError::Truncated);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn is_empty(&self) -> bool {
        self.p >= self.b.len()
    }
    fn finish(&self) -> SmsResult<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(SmsError::Trailing)
        }
    }
}

// ---------------------------------------------------------------------------
// GSM 03.38 default alphabet

/// The escape septet in front of an extension-table character.
pub const GSM_ESC: u8 = 0x1B;

#[rustfmt::skip]
const GSM_BASIC: [char; 128] = [
    '@', '£', '$', '¥', 'è', 'é', 'ù', 'ì', 'ò', 'Ç', '\n', 'Ø', 'ø', '\r', 'Å', 'å',
    'Δ', '_', 'Φ', 'Γ', 'Λ', 'Ω', 'Π', 'Ψ', 'Σ', 'Θ', 'Ξ', '\u{1B}', 'Æ', 'æ', 'ß', 'É',
    ' ', '!', '"', '#', '¤', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/',
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?',
    '¡', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'Ä', 'Ö', 'Ñ', 'Ü', '§',
    '¿', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'ä', 'ö', 'ñ', 'ü', 'à',
];

/// The basic extension table (code after ESC, character).
const GSM_EXT: [(u8, char); 10] = [
    (0x0A, '\u{0C}'),
    (0x14, '^'),
    (0x28, '{'),
    (0x29, '}'),
    (0x2F, '\\'),
    (0x3C, '['),
    (0x3D, '~'),
    (0x3E, ']'),
    (0x40, '|'),
    (0x65, '€'),
];

/// `(septet, Some(ext))` for an extension char (septet is then ESC).
fn gsm7_lookup(c: char) -> Option<(u8, Option<u8>)> {
    if c == '\u{1B}' {
        return None;
    }
    if let Some(i) = GSM_BASIC.iter().position(|&b| b == c) {
        return Some((i as u8, None));
    }
    GSM_EXT
        .iter()
        .find(|(_, e)| *e == c)
        .map(|(code, _)| (GSM_ESC, Some(*code)))
}

fn gsm7_cost(c: char) -> Option<usize> {
    gsm7_lookup(c).map(|(_, ext)| if ext.is_some() { 2 } else { 1 })
}

/// Whether every character of `text` is in the GSM 7-bit default alphabet
/// (basic or extension table).
pub fn gsm7_encodable(text: &str) -> bool {
    text.chars().all(|c| gsm7_lookup(c).is_some())
}

/// Septets `text` takes in GSM 7-bit (extension characters count 2), or
/// `None` when it is not encodable.
pub fn gsm7_len(text: &str) -> Option<usize> {
    text.chars().map(gsm7_cost).sum()
}

/// `text` as unpacked septets, or `None` when it is not encodable.
pub fn gsm7_encode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        let (s, ext) = gsm7_lookup(c)?;
        out.push(s);
        if let Some(e) = ext {
            out.push(e);
        }
    }
    Some(out)
}

/// Unpacked septets to text. An unknown extension code shows its basic
/// character, ESC ESC and a dangling ESC show a space (TS 23.038 6.2.1.1).
pub fn gsm7_decode(septets: &[u8]) -> String {
    let mut out = String::with_capacity(septets.len());
    let mut i = 0;
    while i < septets.len() {
        let s = septets[i] & 0x7F;
        if s != GSM_ESC {
            out.push(GSM_BASIC[s as usize]);
            i += 1;
            continue;
        }
        match septets.get(i + 1).map(|e| e & 0x7F) {
            None | Some(GSM_ESC) => out.push(' '),
            Some(e) => match GSM_EXT.iter().find(|(code, _)| *code == e) {
                Some((_, c)) => out.push(*c),
                None => out.push(GSM_BASIC[e as usize]),
            },
        }
        i += 2;
    }
    out
}

/// Packs septets LSB-first after `fill_bits` zero bits.
pub fn pack_septets(septets: &[u8], fill_bits: usize) -> Vec<u8> {
    let total = fill_bits + 7 * septets.len();
    let mut out = vec![0u8; total.div_ceil(8)];
    let mut bit = fill_bits;
    for &s in septets {
        let v = ((s & 0x7F) as u16) << (bit % 8);
        out[bit / 8] |= v as u8;
        if bit % 8 > 1 {
            out[bit / 8 + 1] |= (v >> 8) as u8;
        }
        bit += 7;
    }
    out
}

/// Unpacks `count` septets that start after `fill_bits` bits of `bytes`.
pub fn unpack_septets(bytes: &[u8], fill_bits: usize, count: usize) -> SmsResult<Vec<u8>> {
    let bits = count
        .checked_mul(7)
        .and_then(|b| b.checked_add(fill_bits))
        .ok_or(SmsError::Invalid("septet count"))?;
    if bytes.len() < bits.div_ceil(8) {
        return Err(SmsError::Truncated);
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let bit = fill_bits + 7 * i;
        let (byte, off) = (bit / 8, bit % 8);
        let mut v = (bytes[byte] as u16) >> off;
        if off > 1 {
            v |= (bytes[byte + 1] as u16) << (8 - off);
        }
        out.push((v & 0x7F) as u8);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// UCS-2

/// `text` as UTF-16BE (non-BMP characters as surrogate pairs).
pub fn ucs2_encode(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

/// UTF-16BE to text; odd length or unpaired surrogates are errors.
pub fn ucs2_decode(bytes: &[u8]) -> SmsResult<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(SmsError::Invalid("UCS-2 odd length"));
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    String::from_utf16(&units).map_err(|_| SmsError::Invalid("UCS-2 surrogate"))
}

// ---------------------------------------------------------------------------
// Addresses

/// A TP or RP address.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Address {
    /// TON 1 / NPI 1 (TOA 0x91); digits without the `+`.
    International(String),
    /// TON 2 / NPI 1 (TOA 0xA1).
    National(String),
    /// TON 0 / NPI 1 (TOA 0x81): short codes and dial strings.
    Unknown(String),
    /// TON 5 (TOA 0xD0): GSM 7-bit text, at most 11 septets (`CARRIER`).
    Alphanumeric(String),
    /// Any other BCD type of address, kept as received.
    Other { toa: u8, digits: String },
}

fn bcd_nibble(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(c as u8 - b'0'),
        '*' => Some(0xA),
        '#' => Some(0xB),
        'a' => Some(0xC),
        'b' => Some(0xD),
        'c' => Some(0xE),
        _ => None,
    }
}

fn nibble_char(n: u8) -> Option<char> {
    match n {
        0..=9 => Some((b'0' + n) as char),
        0xA => Some('*'),
        0xB => Some('#'),
        0xC => Some('a'),
        0xD => Some('b'),
        0xE => Some('c'),
        _ => None,
    }
}

const MAX_ADDR_DIGITS: usize = 20;
const MAX_ALNUM_SEPTETS: usize = 11;

impl Address {
    /// `+digits` is international, plain digits (`0-9*#`) unknown/short code,
    /// anything else alphanumeric.
    pub fn parse(s: &str) -> Address {
        let dial = |d: &str| {
            !d.is_empty()
                && d.len() <= MAX_ADDR_DIGITS
                && d.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '#')
        };
        if let Some(d) = s.strip_prefix('+') {
            if dial(d) {
                return Address::International(d.to_string());
            }
        } else if dial(s) {
            return Address::Unknown(s.to_string());
        }
        Address::Alphanumeric(s.to_string())
    }

    fn from_toa(toa: u8, digits: String) -> Address {
        match toa {
            0x91 => Address::International(digits),
            0xA1 => Address::National(digits),
            0x81 => Address::Unknown(digits),
            _ => Address::Other { toa, digits },
        }
    }

    /// TOA and value for BCD encoding (alphanumeric falls back to 0x81 with
    /// its digits; an `Other` claiming TON 5 too).
    fn bcd_parts(&self) -> (u8, &str) {
        match self {
            Address::International(d) => (0x91, d),
            Address::National(d) => (0xA1, d),
            Address::Unknown(d) => (0x81, d),
            Address::Alphanumeric(t) => (0x81, t),
            Address::Other { toa, digits } => {
                if (toa >> 4) & 7 == 5 {
                    (0x81, digits)
                } else {
                    (toa | 0x80, digits)
                }
            }
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Address::International(d) => write!(f, "+{d}"),
            Address::National(d) | Address::Unknown(d) | Address::Alphanumeric(d) => f.write_str(d),
            Address::Other { digits, .. } => f.write_str(digits),
        }
    }
}

/// (digit count, packed BCD with F filler). Non-BCD characters are skipped.
fn encode_bcd(digits: &str) -> (usize, Vec<u8>) {
    let nib: Vec<u8> = digits.chars().filter_map(bcd_nibble).take(MAX_ADDR_DIGITS).collect();
    let bytes = nib
        .chunks(2)
        .map(|p| p[0] | (p.get(1).copied().unwrap_or(0xF) << 4))
        .collect();
    (nib.len(), bytes)
}

fn decode_bcd(bytes: &[u8], n: usize) -> SmsResult<String> {
    let mut out = String::with_capacity(n);
    for i in 0..n {
        let b = bytes[i / 2];
        let nib = if i % 2 == 0 { b & 0xF } else { b >> 4 };
        out.push(nibble_char(nib).ok_or(SmsError::Invalid("BCD digit"))?);
    }
    if n % 2 == 1 && bytes[n / 2] >> 4 != 0xF {
        return Err(SmsError::Invalid("BCD filler"));
    }
    Ok(out)
}

fn alnum_septets(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in text.chars() {
        let (s, ext) = gsm7_lookup(c).unwrap_or((0x3F, None));
        let cost = if ext.is_some() { 2 } else { 1 };
        if out.len() + cost > MAX_ALNUM_SEPTETS {
            break;
        }
        out.push(s);
        out.extend(ext);
    }
    out
}

fn encode_tp_addr(a: &Address, out: &mut Vec<u8>) {
    if let Address::Alphanumeric(t) = a {
        let s = alnum_septets(t);
        out.push((s.len() * 7).div_ceil(4) as u8);
        out.push(0xD0);
        out.extend(pack_septets(&s, 0));
        return;
    }
    let (toa, digits) = a.bcd_parts();
    let (n, bytes) = encode_bcd(digits);
    out.push(n as u8);
    out.push(toa);
    out.extend(bytes);
}

fn decode_tp_addr(r: &mut Rd) -> SmsResult<Address> {
    let n = r.u8()? as usize;
    if n > MAX_ADDR_DIGITS {
        return Err(SmsError::Invalid("TP address length"));
    }
    let toa = r.u8()?;
    if toa & 0x80 == 0 {
        return Err(SmsError::Invalid("type of address"));
    }
    let bytes = r.take(n.div_ceil(2))?;
    if (toa >> 4) & 7 == 5 {
        let s = unpack_septets(bytes, 0, n * 4 / 7)?;
        return Ok(Address::Alphanumeric(gsm7_decode(&s)));
    }
    Ok(Address::from_toa(toa, decode_bcd(bytes, n)?))
}

fn encode_rp_addr(a: Option<&Address>, out: &mut Vec<u8>) {
    let Some(a) = a else {
        out.push(0);
        return;
    };
    let (toa, digits) = a.bcd_parts();
    let (_, bytes) = encode_bcd(digits);
    out.push(1 + bytes.len() as u8);
    out.push(toa);
    out.extend(bytes);
}

fn decode_rp_addr(r: &mut Rd) -> SmsResult<Option<Address>> {
    let l = r.u8()? as usize;
    if l == 0 {
        return Ok(None);
    }
    if l > 11 {
        return Err(SmsError::Invalid("RP address length"));
    }
    let toa = r.u8()?;
    if toa & 0x80 == 0 {
        return Err(SmsError::Invalid("type of address"));
    }
    if (toa >> 4) & 7 == 5 {
        return Err(SmsError::Invalid("alphanumeric RP address"));
    }
    let bytes = r.take(l - 1)?;
    let mut n = bytes.len() * 2;
    if bytes.last().is_some_and(|b| b >> 4 == 0xF) {
        n -= 1;
    }
    Ok(Some(Address::from_toa(toa, decode_bcd(bytes, n)?)))
}

// ---------------------------------------------------------------------------
// Timestamps and validity

/// TP-SCTS / TP-DT / absolute TP-VP: two-digit year, local time, and the
/// offset from UTC in quarter hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u8,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub tz_quarters: i8,
}

/// Days from 1970-01-01 to 2000-01-01, the in-game clock epoch.
const EPOCH_2000_DAYS: i64 = 10_957;

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Timestamp {
    /// The UTC calendar time `ms` milliseconds after the in-game epoch,
    /// **2000-01-01 00:00:00 UTC** (so the two-digit year is the year mod 100).
    pub fn from_game_millis(ms: u64) -> Timestamp {
        let secs = ms / 1000;
        let days = (secs / 86_400) as i64;
        let sod = secs % 86_400;
        let (y, m, d) = civil_from_days(days + EPOCH_2000_DAYS);
        Timestamp {
            year: (y.rem_euclid(100)) as u8,
            month: m as u8,
            day: d as u8,
            hour: (sod / 3600) as u8,
            minute: (sod / 60 % 60) as u8,
            second: (sod % 60) as u8,
            tz_quarters: 0,
        }
    }

    /// Milliseconds after the epoch (year read as 20YY, offset removed),
    /// 0 for times before it.
    pub fn to_game_millis(&self) -> u64 {
        let days = days_from_civil(2000 + i64::from(self.year), i64::from(self.month), i64::from(self.day))
            - EPOCH_2000_DAYS;
        let secs = days * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
            - i64::from(self.tz_quarters) * 900;
        (secs.max(0) as u64) * 1000
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let semi = |v: u8| {
            let v = v % 100;
            ((v % 10) << 4) | (v / 10)
        };
        for v in [self.year, self.month, self.day, self.hour, self.minute, self.second] {
            out.push(semi(v));
        }
        let q = self.tz_quarters.unsigned_abs().min(79);
        out.push(((q % 10) << 4) | (q / 10) | if self.tz_quarters < 0 { 0x08 } else { 0 });
    }

    fn decode(r: &mut Rd) -> SmsResult<Timestamp> {
        let b = r.take(7)?;
        let semi = |b: u8| {
            let (lo, hi) = (b & 0xF, b >> 4);
            if lo > 9 || hi > 9 {
                Err(SmsError::Invalid("timestamp digit"))
            } else {
                Ok(lo * 10 + hi)
            }
        };
        let ts = Timestamp {
            year: semi(b[0])?,
            month: semi(b[1])?,
            day: semi(b[2])?,
            hour: semi(b[3])?,
            minute: semi(b[4])?,
            second: semi(b[5])?,
            tz_quarters: {
                if b[6] >> 4 > 9 {
                    return Err(SmsError::Invalid("timestamp zone"));
                }
                let q = ((b[6] & 7) * 10 + (b[6] >> 4)) as i8;
                if b[6] & 0x08 != 0 {
                    -q
                } else {
                    q
                }
            },
        };
        if !(1..=12).contains(&ts.month)
            || !(1..=31).contains(&ts.day)
            || ts.hour > 23
            || ts.minute > 59
            || ts.second > 59
        {
            return Err(SmsError::Invalid("timestamp range"));
        }
        Ok(ts)
    }
}

/// TP-VP of an SMS-SUBMIT (format from TP-VPF).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    None,
    /// Relative, coded per TS 23.040 9.2.3.12.1 (see [`Validity::relative_minutes`]).
    Relative(u8),
    Absolute(Timestamp),
    /// Enhanced format, kept raw.
    Enhanced([u8; 7]),
}

impl Validity {
    /// Minutes a relative TP-VP value stands for (0xA7 = 24 h, 0xAA = 4 days).
    pub fn relative_minutes(vp: u8) -> u32 {
        let v = vp as u32;
        match vp {
            0..=143 => (v + 1) * 5,
            144..=167 => 720 + (v - 143) * 30,
            168..=196 => (v - 166) * 1440,
            _ => (v - 192) * 10_080,
        }
    }

    /// The smallest relative TP-VP lasting at least `minutes` (0xFF at most).
    pub fn relative_from_minutes(minutes: u32) -> u8 {
        (0..=255u8)
            .find(|&vp| Self::relative_minutes(vp) >= minutes)
            .unwrap_or(0xFF)
    }

    fn vpf(&self) -> u8 {
        match self {
            Validity::None => 0b00,
            Validity::Enhanced(_) => 0b01,
            Validity::Relative(_) => 0b10,
            Validity::Absolute(_) => 0b11,
        }
    }
}

// ---------------------------------------------------------------------------
// User data

/// A user data header information element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ie {
    /// IEI 0x00: concatenation, 8-bit reference.
    Concat8 { reference: u8, total: u8, seq: u8 },
    /// IEI 0x08: concatenation, 16-bit reference.
    Concat16 { reference: u16, total: u8, seq: u8 },
    /// IEI 0x05: application port addressing, 16-bit ports.
    Ports16 { dest: u16, src: u16 },
    /// Any other element, kept raw (data at most 255 octets).
    Other { id: u8, data: Vec<u8> },
}

/// The WAP push destination port (TS 23.040 9.2.3.24.4, WAP WDP).
pub const WAP_PUSH_PORT: u16 = 2948;
/// The conventional WAP push source port (WSP connectionless).
pub const WAP_PUSH_SOURCE_PORT: u16 = 9200;

impl Ie {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Ie::Concat8 { reference, total, seq } => out.extend([0x00, 3, *reference, *total, *seq]),
            Ie::Concat16 { reference, total, seq } => {
                let r = reference.to_be_bytes();
                out.extend([0x08, 4, r[0], r[1], *total, *seq]);
            }
            Ie::Ports16 { dest, src } => {
                let (d, s) = (dest.to_be_bytes(), src.to_be_bytes());
                out.extend([0x05, 4, d[0], d[1], s[0], s[1]]);
            }
            Ie::Other { id, data } => {
                let n = data.len().min(255);
                out.push(*id);
                out.push(n as u8);
                out.extend(&data[..n]);
            }
        }
    }
}

fn concat_valid(total: u8, seq: u8) -> bool {
    total != 0 && seq != 0 && seq <= total
}

fn parse_ies(h: &[u8]) -> SmsResult<Vec<Ie>> {
    let mut r = Rd::new(h);
    let mut out = Vec::new();
    while !r.is_empty() {
        let id = r.u8()?;
        let len = r.u8()? as usize;
        let d = r.take(len)?;
        out.push(match id {
            0x00 => {
                if len != 3 || !concat_valid(d[1], d[2]) {
                    return Err(SmsError::Invalid("concatenation IE"));
                }
                Ie::Concat8 { reference: d[0], total: d[1], seq: d[2] }
            }
            0x08 => {
                if len != 4 || !concat_valid(d[2], d[3]) {
                    return Err(SmsError::Invalid("concatenation IE"));
                }
                Ie::Concat16 { reference: u16::from_be_bytes([d[0], d[1]]), total: d[2], seq: d[3] }
            }
            0x05 => {
                if len != 4 {
                    return Err(SmsError::Invalid("port IE"));
                }
                Ie::Ports16 { dest: u16::from_be_bytes([d[0], d[1]]), src: u16::from_be_bytes([d[2], d[3]]) }
            }
            _ => Ie::Other { id, data: d.to_vec() },
        });
    }
    Ok(out)
}

/// What a message body is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text(String),
    Binary(Vec<u8>),
}

/// TP-DCS character set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// GSM 7-bit default alphabet (DCS 0x00).
    Gsm7,
    /// UCS-2 / UTF-16BE (DCS 0x08).
    Ucs2,
    /// 8-bit data (DCS 0x04).
    Data8,
}

impl Encoding {
    pub fn dcs(self) -> u8 {
        match self {
            Encoding::Gsm7 => 0x00,
            Encoding::Ucs2 => 0x08,
            Encoding::Data8 => 0x04,
        }
    }

    /// The character set a TP-DCS selects (TS 23.038 4).
    pub fn from_dcs(dcs: u8) -> SmsResult<Encoding> {
        let by_bits = |b: u8| match b & 0x0C {
            0x00 => Ok(Encoding::Gsm7),
            0x04 => Ok(Encoding::Data8),
            0x08 => Ok(Encoding::Ucs2),
            _ => Err(SmsError::Invalid("DCS alphabet")),
        };
        match dcs >> 4 {
            0x0..=0x7 => {
                if dcs & 0x20 != 0 {
                    Err(SmsError::Unsupported("compressed DCS"))
                } else {
                    by_bits(dcs)
                }
            }
            0x8..=0xB => Err(SmsError::Invalid("reserved DCS group")),
            0xC | 0xD => Ok(Encoding::Gsm7),
            0xE => Ok(Encoding::Ucs2),
            _ => Ok(if dcs & 0x04 != 0 { Encoding::Data8 } else { Encoding::Gsm7 }),
        }
    }
}

/// TP-UD: header elements and body. TP-UDHI is set when `udh` is not empty.
///
/// Encoding picks the character set from the body: text in GSM 7-bit when it
/// all fits the alphabet, otherwise UCS-2; binary as 8-bit data. One part
/// carries at most 140 octets; longer bodies are cut to fit on encode (use
/// [`split_text`] for long text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserData {
    pub udh: Vec<Ie>,
    pub body: Body,
}

impl UserData {
    pub fn text(s: &str) -> UserData {
        UserData { udh: Vec::new(), body: Body::Text(s.to_string()) }
    }

    pub fn binary(b: &[u8]) -> UserData {
        UserData { udh: Vec::new(), body: Body::Binary(b.to_vec()) }
    }

    pub fn encoding(&self) -> Encoding {
        match &self.body {
            Body::Text(t) if gsm7_encodable(t) => Encoding::Gsm7,
            Body::Text(_) => Encoding::Ucs2,
            Body::Binary(_) => Encoding::Data8,
        }
    }

    /// `(reference, total, seq)` of the first concatenation element.
    pub fn concat(&self) -> Option<(u16, u8, u8)> {
        self.udh.iter().find_map(|ie| match ie {
            Ie::Concat8 { reference, total, seq } => Some((*reference as u16, *total, *seq)),
            Ie::Concat16 { reference, total, seq } => Some((*reference, *total, *seq)),
            _ => None,
        })
    }

    /// `(dest, src)` of the port addressing element.
    pub fn ports(&self) -> Option<(u16, u16)> {
        self.udh.iter().find_map(|ie| match ie {
            Ie::Ports16 { dest, src } => Some((*dest, *src)),
            _ => None,
        })
    }

    /// `(dcs, udl, ud, udhi)`.
    fn encode_ud(&self) -> (u8, u8, Vec<u8>, bool) {
        let mut hdr = Vec::new();
        for ie in &self.udh {
            ie.encode(&mut hdr);
        }
        if hdr.len() > 139 {
            hdr.clear();
        }
        let hlen = if hdr.is_empty() { 0 } else { 1 + hdr.len() };
        let mut ud = Vec::with_capacity(140);
        if hlen > 0 {
            ud.push(hdr.len() as u8);
            ud.extend(&hdr);
        }
        let enc = self.encoding();
        let udl = match &self.body {
            Body::Text(t) if enc == Encoding::Gsm7 => {
                let mut s = gsm7_encode(t).unwrap_or_default();
                let hs = (hlen * 8).div_ceil(7);
                s.truncate(160 - hs);
                // A cut can only orphan an escape at the end (ext codes are never 0x1B).
                if s.last() == Some(&GSM_ESC) {
                    s.pop();
                }
                ud.extend(pack_septets(&s, hs * 7 - hlen * 8));
                hs + s.len()
            }
            Body::Text(t) => {
                let mut units: Vec<u16> = t.encode_utf16().collect();
                units.truncate((140 - hlen) / 2);
                if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
                    units.pop();
                }
                ud.extend(units.iter().flat_map(|u| u.to_be_bytes()));
                ud.len()
            }
            Body::Binary(b) => {
                ud.extend(&b[..b.len().min(140 - hlen)]);
                ud.len()
            }
        };
        (enc.dcs(), udl as u8, ud, hlen > 0)
    }

    fn decode_ud(dcs: u8, udhi: bool, r: &mut Rd) -> SmsResult<UserData> {
        let enc = Encoding::from_dcs(dcs)?;
        let udl = r.u8()? as usize;
        let ud = match enc {
            Encoding::Gsm7 if udl > 160 => return Err(SmsError::Invalid("TP-UDL")),
            Encoding::Gsm7 => r.take((udl * 7).div_ceil(8))?,
            _ if udl > 140 => return Err(SmsError::Invalid("TP-UDL")),
            _ => r.take(udl)?,
        };
        let (udh, hbytes) = if udhi {
            let hl = *ud.first().ok_or(SmsError::Invalid("UDHI without header"))? as usize;
            if 1 + hl > ud.len() {
                return Err(SmsError::Invalid("UDH longer than UD"));
            }
            (parse_ies(&ud[1..1 + hl])?, 1 + hl)
        } else {
            (Vec::new(), 0)
        };
        let body = match enc {
            Encoding::Gsm7 => {
                let hs = (hbytes * 8).div_ceil(7);
                if hs > udl {
                    return Err(SmsError::Invalid("UDH longer than UD"));
                }
                let s = unpack_septets(&ud[hbytes..], hs * 7 - hbytes * 8, udl - hs)?;
                Body::Text(gsm7_decode(&s))
            }
            Encoding::Ucs2 => Body::Text(ucs2_decode(&ud[hbytes..])?),
            Encoding::Data8 => Body::Binary(ud[hbytes..].to_vec()),
        };
        Ok(UserData { udh, body })
    }
}

// ---------------------------------------------------------------------------
// Splitting and the compose counter

/// Byte ranges and units used, per part.
fn chunk(text: &str) -> (Encoding, usize, Vec<(usize, usize, usize)>) {
    let enc = if gsm7_encodable(text) { Encoding::Gsm7 } else { Encoding::Ucs2 };
    let cost = |c: char| match enc {
        Encoding::Gsm7 => gsm7_cost(c).unwrap_or(1),
        _ => c.len_utf16(),
    };
    let (single, multi) = if enc == Encoding::Gsm7 { (160, 153) } else { (70, 67) };
    let total: usize = text.chars().map(cost).sum();
    if total <= single {
        return (enc, single, vec![(0, text.len(), total)]);
    }
    let mut out = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (i, c) in text.char_indices() {
        let k = cost(c);
        if used + k > multi {
            out.push((start, i, used));
            if out.len() == 255 {
                return (enc, multi, out);
            }
            start = i;
            used = 0;
        }
        used += k;
    }
    out.push((start, text.len(), used));
    (enc, multi, out)
}

/// Splits `text` into message parts: one part without a header when it fits
/// (160 septets / 70 UTF-16 units), otherwise parts of at most 153 septets /
/// 67 units, each with a concatenation IE 0x00 carrying `reference`. Escape
/// pairs and surrogate pairs stay whole. At most 255 parts; text past them
/// is dropped.
pub fn split_text(text: &str, reference: u8) -> Vec<UserData> {
    let (_, _, parts) = chunk(text);
    if parts.len() == 1 {
        return vec![UserData::text(text)];
    }
    let total = parts.len() as u8;
    parts
        .iter()
        .enumerate()
        .map(|(i, &(a, b, _))| UserData {
            udh: vec![Ie::Concat8 { reference, total, seq: i as u8 + 1 }],
            body: Body::Text(text[a..b].to_string()),
        })
        .collect()
}

/// For a compose counter: the encoding, the number of parts, and how many
/// septets (GSM 7-bit) or UTF-16 units (UCS-2) are left in the last part.
pub fn parts_needed(text: &str) -> (Encoding, usize, usize) {
    let (enc, limit, parts) = chunk(text);
    let last = parts.last().map_or(0, |p| p.2);
    (enc, parts.len(), limit.saturating_sub(last))
}

// ---------------------------------------------------------------------------
// Reassembly

/// Most incomplete sets a [`Reassembler`] keeps; the oldest goes first.
pub const MAX_PENDING_SETS: usize = 256;

#[derive(Debug)]
struct Pending {
    first_ms: u64,
    parts: Vec<Option<UserData>>,
    have: usize,
}

/// Rejoins concatenated parts keyed by (sender, reference, total), in any
/// order, dropping duplicates; sets incomplete `timeout_ms` after their
/// first part are dropped.
#[derive(Debug)]
pub struct Reassembler {
    timeout_ms: u64,
    sets: HashMap<(String, u16, u8), Pending>,
}

impl Reassembler {
    pub fn new(timeout_ms: u64) -> Reassembler {
        Reassembler { timeout_ms, sets: HashMap::new() }
    }

    /// Feeds one part. Returns the whole message when this part completes
    /// it (or at once for an unconcatenated message): the first part's
    /// other header elements and the joined body. Duplicates and invalid
    /// parts return `None`.
    pub fn push(&mut self, sender: &str, ud: UserData, now_ms: u64) -> Option<UserData> {
        self.expire(now_ms);
        let Some((reference, total, seq)) = ud.concat() else {
            return Some(ud);
        };
        if !concat_valid(total, seq) {
            return None;
        }
        if total == 1 {
            return Some(join(vec![ud]));
        }
        let key = (sender.to_string(), reference, total);
        if !self.sets.contains_key(&key) && self.sets.len() >= MAX_PENDING_SETS {
            if let Some(old) = self.sets.iter().min_by_key(|(_, p)| p.first_ms).map(|(k, _)| k.clone()) {
                self.sets.remove(&old);
            }
        }
        let p = self.sets.entry(key.clone()).or_insert_with(|| Pending {
            first_ms: now_ms,
            parts: vec![None; total as usize],
            have: 0,
        });
        let slot = &mut p.parts[seq as usize - 1];
        if slot.is_some() {
            return None;
        }
        *slot = Some(ud);
        p.have += 1;
        if p.have < total as usize {
            return None;
        }
        let p = self.sets.remove(&key)?;
        Some(join(p.parts.into_iter().flatten().collect()))
    }

    /// Drops sets whose first part arrived `timeout_ms` or more ago; returns
    /// how many.
    pub fn expire(&mut self, now_ms: u64) -> usize {
        let before = self.sets.len();
        let t = self.timeout_ms;
        self.sets.retain(|_, p| now_ms.saturating_sub(p.first_ms) < t);
        before - self.sets.len()
    }

    /// Incomplete sets held.
    pub fn pending(&self) -> usize {
        self.sets.len()
    }
}

fn join(parts: Vec<UserData>) -> UserData {
    let udh = parts
        .first()
        .map(|p| {
            p.udh
                .iter()
                .filter(|ie| !matches!(ie, Ie::Concat8 { .. } | Ie::Concat16 { .. }))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let body = if parts.iter().all(|p| matches!(p.body, Body::Text(_))) {
        Body::Text(
            parts
                .iter()
                .map(|p| match &p.body {
                    Body::Text(t) => t.as_str(),
                    Body::Binary(_) => "",
                })
                .collect(),
        )
    } else {
        let mut b = Vec::new();
        for p in &parts {
            match &p.body {
                Body::Text(t) => b.extend(t.as_bytes()),
                Body::Binary(d) => b.extend(d),
            }
        }
        Body::Binary(b)
    };
    UserData { udh, body }
}

// ---------------------------------------------------------------------------
// TPDUs

/// TP-PID for plain person-to-person messages.
pub const PID_DEFAULT: u8 = 0x00;

/// Common TP-ST values (TS 23.040 9.2.3.15).
pub mod status {
    pub const DELIVERED: u8 = 0x00;
    pub const FORWARDED_UNCONFIRMED: u8 = 0x01;
    pub const REPLACED: u8 = 0x02;
    pub const TEMP_CONGESTION: u8 = 0x20;
    pub const TEMP_SME_BUSY: u8 = 0x21;
    pub const TEMP_NO_RESPONSE: u8 = 0x22;
    pub const PERM_REMOTE_PROCEDURE_ERROR: u8 = 0x40;
    pub const PERM_INCOMPATIBLE_DESTINATION: u8 = 0x41;
    pub const PERM_VALIDITY_EXPIRED: u8 = 0x46;
    pub const GAVE_UP_CONGESTION: u8 = 0x60;
    pub const GAVE_UP_SME_BUSY: u8 = 0x61;
}

/// SMS-SUBMIT (MS -> SC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submit {
    /// TP-RD.
    pub reject_duplicates: bool,
    /// TP-SRR.
    pub status_report_request: bool,
    /// TP-RP.
    pub reply_path: bool,
    /// TP-MR.
    pub mr: u8,
    /// TP-DA.
    pub destination: Address,
    pub pid: u8,
    pub validity: Validity,
    pub ud: UserData,
}

impl Submit {
    /// A plain submit valid for 24 hours (TP-VP 0xA7).
    pub fn new(mr: u8, destination: Address, ud: UserData) -> Submit {
        Submit {
            reject_duplicates: false,
            status_report_request: false,
            reply_path: false,
            mr,
            destination,
            pid: PID_DEFAULT,
            validity: Validity::Relative(0xA7),
            ud,
        }
    }
}

/// SMS-DELIVER (SC -> MS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deliver {
    /// More messages are waiting at the SC (TP-MMS bit is 0 then).
    pub more_messages: bool,
    /// TP-SRI.
    pub status_report_indication: bool,
    /// TP-RP.
    pub reply_path: bool,
    /// TP-OA.
    pub originator: Address,
    pub pid: u8,
    /// TP-SCTS.
    pub timestamp: Timestamp,
    pub ud: UserData,
}

/// SMS-STATUS-REPORT (SC -> MS). Optional TP-PI fields are accepted on decode
/// and not kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusReport {
    /// More messages are waiting at the SC (TP-MMS bit is 0 then).
    pub more_messages: bool,
    /// TP-SRQ: the report answers an SMS-COMMAND (false: an SMS-SUBMIT).
    pub srq: bool,
    /// TP-MR of the submit it reports on.
    pub mr: u8,
    /// TP-RA.
    pub recipient: Address,
    /// TP-SCTS.
    pub sc_timestamp: Timestamp,
    /// TP-DT.
    pub discharge_time: Timestamp,
    /// TP-ST (see [`status`]).
    pub status: u8,
}

/// A transfer-layer PDU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tpdu {
    Submit(Submit),
    Deliver(Deliver),
    StatusReport(StatusReport),
}

fn bit(b: bool, n: u8) -> u8 {
    if b {
        1 << n
    } else {
        0
    }
}

impl Tpdu {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(176);
        match self {
            Tpdu::Submit(s) => {
                let (dcs, udl, ud, udhi) = s.ud.encode_ud();
                out.push(
                    0x01 | bit(s.reject_duplicates, 2)
                        | s.validity.vpf() << 3
                        | bit(s.status_report_request, 5)
                        | bit(udhi, 6)
                        | bit(s.reply_path, 7),
                );
                out.push(s.mr);
                encode_tp_addr(&s.destination, &mut out);
                out.push(s.pid);
                out.push(dcs);
                match &s.validity {
                    Validity::None => {}
                    Validity::Relative(v) => out.push(*v),
                    Validity::Absolute(t) => t.encode(&mut out),
                    Validity::Enhanced(e) => out.extend(e),
                }
                out.push(udl);
                out.extend(ud);
            }
            Tpdu::Deliver(d) => {
                let (dcs, udl, ud, udhi) = d.ud.encode_ud();
                out.push(
                    bit(!d.more_messages, 2)
                        | bit(d.status_report_indication, 5)
                        | bit(udhi, 6)
                        | bit(d.reply_path, 7),
                );
                encode_tp_addr(&d.originator, &mut out);
                out.push(d.pid);
                out.push(dcs);
                d.timestamp.encode(&mut out);
                out.push(udl);
                out.extend(ud);
            }
            Tpdu::StatusReport(r) => {
                out.push(0x02 | bit(!r.more_messages, 2) | bit(r.srq, 5));
                out.push(r.mr);
                encode_tp_addr(&r.recipient, &mut out);
                r.sc_timestamp.encode(&mut out);
                r.discharge_time.encode(&mut out);
                out.push(r.status);
            }
        }
        out
    }

    /// Decodes a TPDU travelling MS -> SC (SMS-SUBMIT).
    pub fn decode_mo(bytes: &[u8]) -> SmsResult<Tpdu> {
        let mut r = Rd::new(bytes);
        let fo = r.u8()?;
        match fo & 3 {
            0b01 => {}
            0b00 => return Err(SmsError::Unsupported("SMS-DELIVER-REPORT")),
            0b10 => return Err(SmsError::Unsupported("SMS-COMMAND")),
            _ => return Err(SmsError::Invalid("TP-MTI")),
        }
        let mr = r.u8()?;
        let destination = decode_tp_addr(&mut r)?;
        let pid = r.u8()?;
        let dcs = r.u8()?;
        let validity = match (fo >> 3) & 3 {
            0b00 => Validity::None,
            0b10 => Validity::Relative(r.u8()?),
            0b11 => Validity::Absolute(Timestamp::decode(&mut r)?),
            _ => {
                let e = r.take(7)?;
                let mut a = [0u8; 7];
                a.copy_from_slice(e);
                Validity::Enhanced(a)
            }
        };
        let ud = UserData::decode_ud(dcs, fo & 0x40 != 0, &mut r)?;
        r.finish()?;
        Ok(Tpdu::Submit(Submit {
            reject_duplicates: fo & 0x04 != 0,
            status_report_request: fo & 0x20 != 0,
            reply_path: fo & 0x80 != 0,
            mr,
            destination,
            pid,
            validity,
            ud,
        }))
    }

    /// Decodes a TPDU travelling SC -> MS (SMS-DELIVER or SMS-STATUS-REPORT).
    pub fn decode_mt(bytes: &[u8]) -> SmsResult<Tpdu> {
        let mut r = Rd::new(bytes);
        let fo = r.u8()?;
        let more_messages = fo & 0x04 == 0;
        match fo & 3 {
            0b00 => {
                let originator = decode_tp_addr(&mut r)?;
                let pid = r.u8()?;
                let dcs = r.u8()?;
                let timestamp = Timestamp::decode(&mut r)?;
                let ud = UserData::decode_ud(dcs, fo & 0x40 != 0, &mut r)?;
                r.finish()?;
                Ok(Tpdu::Deliver(Deliver {
                    more_messages,
                    status_report_indication: fo & 0x20 != 0,
                    reply_path: fo & 0x80 != 0,
                    originator,
                    pid,
                    timestamp,
                    ud,
                }))
            }
            0b10 => {
                let mr = r.u8()?;
                let recipient = decode_tp_addr(&mut r)?;
                let sc_timestamp = Timestamp::decode(&mut r)?;
                let discharge_time = Timestamp::decode(&mut r)?;
                let status = r.u8()?;
                if !r.is_empty() {
                    let mut pi = r.u8()?;
                    let first = pi;
                    while pi & 0x80 != 0 {
                        pi = r.u8()?;
                    }
                    if first & 0x01 != 0 {
                        r.u8()?;
                    }
                    let dcs = if first & 0x02 != 0 { r.u8()? } else { 0 };
                    if first & 0x04 != 0 {
                        UserData::decode_ud(dcs, fo & 0x40 != 0, &mut r)?;
                    }
                    r.finish()?;
                }
                Ok(Tpdu::StatusReport(StatusReport {
                    more_messages,
                    srq: fo & 0x20 != 0,
                    mr,
                    recipient,
                    sc_timestamp,
                    discharge_time,
                    status,
                }))
            }
            0b01 => Err(SmsError::Unsupported("SMS-SUBMIT-REPORT")),
            _ => Err(SmsError::Invalid("TP-MTI")),
        }
    }
}

// ---------------------------------------------------------------------------
// RP layer

/// Which way an RP message travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    MsToNetwork,
    NetworkToMs,
}

/// RP-Cause values (TS 24.011 8.2.5.4).
pub mod cause {
    pub const UNASSIGNED_NUMBER: u8 = 1;
    pub const OPERATOR_DETERMINED_BARRING: u8 = 8;
    pub const CALL_BARRED: u8 = 10;
    pub const SHORT_MESSAGE_TRANSFER_REJECTED: u8 = 21;
    pub const MEMORY_CAPACITY_EXCEEDED: u8 = 22;
    pub const DESTINATION_OUT_OF_ORDER: u8 = 27;
    pub const UNIDENTIFIED_SUBSCRIBER: u8 = 28;
    pub const UNKNOWN_SUBSCRIBER: u8 = 30;
    pub const NETWORK_OUT_OF_ORDER: u8 = 38;
    pub const TEMPORARY_FAILURE: u8 = 41;
    pub const CONGESTION: u8 = 42;
    pub const RESOURCES_UNAVAILABLE: u8 = 47;
    pub const INVALID_MESSAGE: u8 = 95;
    pub const INVALID_MANDATORY_INFORMATION: u8 = 96;
    pub const MESSAGE_TYPE_NONEXISTENT: u8 = 97;
    pub const PROTOCOL_ERROR: u8 = 111;
}

/// RP-User-Data IEI in RP-ACK / RP-ERROR.
const RP_UD_IEI: u8 = 0x41;

/// A relay-layer message (the body of an IMS SIP MESSAGE,
/// `application/vnd.3gpp.sms`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rp {
    /// RP-DATA. `smsc` is RP-DA going MS->network and RP-OA coming back;
    /// the other address is empty. `tpdu` is at most 255 octets.
    Data { mr: u8, smsc: Address, tpdu: Vec<u8> },
    /// RP-ACK, optionally carrying a DELIVER-REPORT / SUBMIT-REPORT.
    Ack { mr: u8, tpdu: Option<Vec<u8>> },
    /// RP-ERROR with a cause (see [`cause`]).
    Error { mr: u8, cause: u8, diagnostic: Option<u8>, tpdu: Option<Vec<u8>> },
    /// RP-SMMA (memory available again); MS->network only.
    Smma { mr: u8 },
}

fn push_lv(out: &mut Vec<u8>, b: &[u8]) {
    let n = b.len().min(255);
    out.push(n as u8);
    out.extend(&b[..n]);
}

fn rp_ud_opt(r: &mut Rd) -> SmsResult<Option<Vec<u8>>> {
    if r.is_empty() {
        return Ok(None);
    }
    if r.u8()? != RP_UD_IEI {
        return Err(SmsError::Invalid("RP IEI"));
    }
    let l = r.u8()? as usize;
    Ok(Some(r.take(l)?.to_vec()))
}

impl Rp {
    pub fn mr(&self) -> u8 {
        match self {
            Rp::Data { mr, .. } | Rp::Ack { mr, .. } | Rp::Error { mr, .. } | Rp::Smma { mr } => *mr,
        }
    }

    /// Encodes for travel in `dir` (RP-SMMA is always MTI 6).
    pub fn encode(&self, dir: Direction) -> Vec<u8> {
        let up = u8::from(dir == Direction::NetworkToMs);
        let mut out = Vec::with_capacity(200);
        match self {
            Rp::Data { mr, smsc, tpdu } => {
                out.extend([up, *mr]);
                if dir == Direction::MsToNetwork {
                    encode_rp_addr(None, &mut out);
                    encode_rp_addr(Some(smsc), &mut out);
                } else {
                    encode_rp_addr(Some(smsc), &mut out);
                    encode_rp_addr(None, &mut out);
                }
                push_lv(&mut out, tpdu);
            }
            Rp::Ack { mr, tpdu } => {
                out.extend([2 | up, *mr]);
                if let Some(t) = tpdu {
                    out.push(RP_UD_IEI);
                    push_lv(&mut out, t);
                }
            }
            Rp::Error { mr, cause, diagnostic, tpdu } => {
                out.extend([4 | up, *mr]);
                match diagnostic {
                    Some(d) => out.extend([2, cause & 0x7F, *d]),
                    None => out.extend([1, cause & 0x7F]),
                }
                if let Some(t) = tpdu {
                    out.push(RP_UD_IEI);
                    push_lv(&mut out, t);
                }
            }
            Rp::Smma { mr } => out.extend([6, *mr]),
        }
        out
    }

    /// Decodes a message travelling in `dir`; an RP-MTI of the other
    /// direction is an error.
    pub fn decode(bytes: &[u8], dir: Direction) -> SmsResult<Rp> {
        let mut r = Rd::new(bytes);
        let mti = r.u8()?;
        if mti & 0xF8 != 0 {
            return Err(SmsError::Invalid("RP spare bits"));
        }
        if mti == 7 {
            return Err(SmsError::Invalid("RP-MTI"));
        }
        let mti_dir = if mti & 1 == 0 { Direction::MsToNetwork } else { Direction::NetworkToMs };
        if mti_dir != dir {
            return Err(SmsError::Invalid("RP-MTI direction"));
        }
        let mr = r.u8()?;
        let msg = match mti >> 1 {
            0 => {
                let oa = decode_rp_addr(&mut r)?;
                let da = decode_rp_addr(&mut r)?;
                let smsc = match (dir, oa, da) {
                    (Direction::MsToNetwork, None, Some(a)) => a,
                    (Direction::NetworkToMs, Some(a), None) => a,
                    _ => return Err(SmsError::Invalid("RP-DATA addresses")),
                };
                let l = r.u8()? as usize;
                if l == 0 {
                    return Err(SmsError::Invalid("empty RP-User-Data"));
                }
                Rp::Data { mr, smsc, tpdu: r.take(l)?.to_vec() }
            }
            1 => Rp::Ack { mr, tpdu: rp_ud_opt(&mut r)? },
            2 => {
                let cl = r.u8()?;
                if cl == 0 || cl > 2 {
                    return Err(SmsError::Invalid("RP-Cause length"));
                }
                let cause = r.u8()?;
                if cause & 0x80 != 0 {
                    return Err(SmsError::Invalid("RP-Cause extension"));
                }
                let diagnostic = if cl == 2 { Some(r.u8()?) } else { None };
                Rp::Error { mr, cause, diagnostic, tpdu: rp_ud_opt(&mut r)? }
            }
            _ => Rp::Smma { mr },
        };
        r.finish()?;
        Ok(msg)
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02X}")).collect()
    }

    fn ts() -> Timestamp {
        Timestamp { year: 26, month: 10, day: 17, hour: 9, minute: 5, second: 30, tz_quarters: -20 }
    }

    fn submit_of(tpdu: Tpdu) -> Submit {
        match tpdu {
            Tpdu::Submit(s) => s,
            other => panic!("not a submit: {other:?}"),
        }
    }

    fn deliver_of(tpdu: Tpdu) -> Deliver {
        match tpdu {
            Tpdu::Deliver(d) => d,
            other => panic!("not a deliver: {other:?}"),
        }
    }

    #[test]
    fn hellohello_packs_to_published_vector() {
        let s = gsm7_encode("hellohello").unwrap();
        let packed = pack_septets(&s, 0);
        assert_eq!(to_hex(&packed), "E8329BFD4697D9EC37");
        assert_eq!(gsm7_decode(&unpack_septets(&packed, 0, 10).unwrap()), "hellohello");
        assert_eq!(unpack_septets(&packed[..8], 0, 10), Err(SmsError::Truncated));
    }

    #[test]
    fn classic_submit_pdu_decodes_and_reencodes() {
        // AT+CMGS form: leading 00 = default SMSC, then the TPDU.
        let pdu = hex("0011000B916407281553F80000AA0AE8329BFD4697D9EC37");
        assert_eq!(pdu[0], 0x00);
        let s = submit_of(Tpdu::decode_mo(&pdu[1..]).unwrap());
        assert_eq!(s.destination, Address::International("46708251358".into()));
        assert_eq!(s.destination.to_string(), "+46708251358");
        assert_eq!(s.validity, Validity::Relative(0xAA));
        assert_eq!(Validity::relative_minutes(0xAA), 4 * 1440);
        assert_eq!(s.mr, 0);
        assert!(!s.reject_duplicates && !s.status_report_request && !s.reply_path);
        assert_eq!(s.ud, UserData::text("hellohello"));
        assert_eq!(Tpdu::Submit(s).encode(), pdu[1..]);
        // It is not an MT PDU.
        assert!(Tpdu::decode_mt(&pdu[1..]).is_err());
    }

    #[test]
    fn published_deliver_pdu_decodes() {
        // SMSC 07 91 3126040000F0 (+31624000000), then the SMS-DELIVER.
        let pdu = hex("07911326040000F0040B911346610089F60000208062917314080CC8F71D14969741F977FD07");
        let d = deliver_of(Tpdu::decode_mt(&pdu[8..]).unwrap());
        assert_eq!(d.originator.to_string(), "+31641600986");
        assert!(!d.more_messages);
        assert_eq!(
            (d.timestamp.year, d.timestamp.month, d.timestamp.day),
            (2, 8, 26)
        );
        assert_eq!((d.timestamp.hour, d.timestamp.minute, d.timestamp.second), (19, 37, 41));
        assert_eq!(d.ud.body, Body::Text("How are you?".into()));
        // The SMSC prefix itself is an RP-style BCD address.
        let mut r = Rd::new(&pdu[..8]);
        assert_eq!(decode_rp_addr(&mut r).unwrap(), Some(Address::International("31624000000".into())));
    }

    #[test]
    fn deliver_round_trip_with_zone_and_flags() {
        let d = Deliver {
            more_messages: true,
            status_report_indication: true,
            reply_path: false,
            originator: Address::International("15551234567".into()),
            pid: 0,
            timestamp: ts(),
            ud: UserData::text("Meet at the station 5pm"),
        };
        let b = Tpdu::Deliver(d.clone()).encode();
        assert_eq!(b[0] & 0x04, 0, "MMS bit 0 = more waiting");
        assert_eq!(b[0] & 0x20, 0x20);
        // Timestamp 26-10-17 09:05:30 -5h: 62 01 71 90 50 03 0A
        let tsb = &b[b.len() - 1 - 21 - 7..b.len() - 1 - 21];
        assert_eq!(to_hex(tsb), "6201719050030A");
        assert_eq!(Tpdu::decode_mt(&b).unwrap(), Tpdu::Deliver(d));
    }

    #[test]
    fn ucs2_submit_vector() {
        let s = Submit::new(5, Address::International("46708251358".into()), UserData::text("你好"));
        let b = Tpdu::Submit(s.clone()).encode();
        assert_eq!(to_hex(&b), "1105 0B916407281553F8 0008A7 04 4F60597D".replace(' ', ""));
        assert_eq!(submit_of(Tpdu::decode_mo(&b).unwrap()), s);
        // Non-BMP: a surrogate pair.
        assert_eq!(to_hex(&ucs2_encode("😀")), "D83DDE00");
        assert_eq!(ucs2_decode(&hex("D83DDE00")).unwrap(), "😀");
        assert!(ucs2_decode(&hex("D83D")).is_err());
        assert!(ucs2_decode(&hex("00")).is_err());
    }

    #[test]
    fn extension_characters() {
        let text = "€[]{}^~|\\";
        assert!(gsm7_encodable(text));
        assert_eq!(gsm7_len(text), Some(18));
        let s = gsm7_encode("€[]").unwrap();
        assert_eq!(s, vec![0x1B, 0x65, 0x1B, 0x3C, 0x1B, 0x3E]);
        assert_eq!(gsm7_decode(&s), "€[]");
        assert!(!gsm7_encodable("Ж"));
        assert!(!gsm7_encodable("\u{1B}"));
        assert_eq!(gsm7_len("Ж"), None);
        // Dangling escape, double escape, unknown extension code.
        assert_eq!(gsm7_decode(&[0x41, 0x1B]), "A ");
        assert_eq!(gsm7_decode(&[0x1B, 0x1B]), " ");
        assert_eq!(gsm7_decode(&[0x1B, 0x41]), "A");
        let ud = UserData::text("price: 5€ [ok]");
        let s = Submit::new(1, Address::Unknown("12345".into()), ud.clone());
        let b = Tpdu::Submit(s).encode();
        assert_eq!(b[b.len() - 1 - (17 * 7usize).div_ceil(8)], 17, "UDL counts escapes");
        assert_eq!(submit_of(Tpdu::decode_mo(&b).unwrap()).ud, ud);
    }

    #[test]
    fn two_part_concatenation_round_trip() {
        let text: String = (0..200).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let parts = split_text(&text, 0x42);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].udh, vec![Ie::Concat8 { reference: 0x42, total: 2, seq: 1 }]);
        let mut pdus = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            let b = Tpdu::Submit(Submit::new(i as u8, Address::International("15551234567".into()), p.clone())).encode();
            pdus.push(b);
        }
        // fo has UDHI; UD starts 05 00 03 42 02 01 and UDL = 7 + 153 septets.
        let b = &pdus[0];
        assert_eq!(b[0], 0x51);
        let udl_at = b.len() - 140 - 1;
        assert_eq!(b[udl_at], 160);
        let ud = &b[udl_at + 1..];
        assert_eq!(to_hex(&ud[..6]), "050003420201");
        // One fill bit: the first septet ('a' = 0x61) sits shifted left by one.
        assert_eq!(ud[6], 0x61u8 << 1);
        let second = &pdus[1];
        assert_eq!(second[second.len() - 48 - 1], 7 + 47);
        let mut re = Reassembler::new(60_000);
        let p2 = submit_of(Tpdu::decode_mo(&pdus[1]).unwrap()).ud;
        let p1 = submit_of(Tpdu::decode_mo(&pdus[0]).unwrap()).ud;
        assert_eq!(p1.body, Body::Text(text[..153].to_string()));
        assert_eq!(re.push("+15551234567", p2, 0), None);
        let whole = re.push("+15551234567", p1, 10).unwrap();
        assert_eq!(whole.body, Body::Text(text));
        assert!(whole.udh.is_empty());
        assert_eq!(re.pending(), 0);
    }

    #[test]
    fn split_never_breaks_escape_or_surrogate() {
        let text = format!("{}€{}", "a".repeat(152), "b".repeat(20));
        let parts = split_text(&text, 1);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].body, Body::Text("a".repeat(152)));
        assert_eq!(parts[1].body, Body::Text(format!("€{}", "b".repeat(20))));

        let text = format!("{}😀{}", "Ж".repeat(66), "Ж".repeat(10));
        let parts = split_text(&text, 2);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].body, Body::Text("Ж".repeat(66)));
        for p in &parts {
            assert_eq!(p.encoding(), Encoding::Ucs2);
            let b = Tpdu::Submit(Submit::new(0, Address::Unknown("1".into()), p.clone())).encode();
            assert_eq!(submit_of(Tpdu::decode_mo(&b).unwrap()).ud, *p);
        }
        let joined: String = parts
            .iter()
            .map(|p| match &p.body {
                Body::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn compose_counter() {
        assert_eq!(parts_needed(""), (Encoding::Gsm7, 1, 160));
        assert_eq!(parts_needed(&"a".repeat(160)), (Encoding::Gsm7, 1, 0));
        assert_eq!(parts_needed(&"a".repeat(161)), (Encoding::Gsm7, 2, 145));
        assert_eq!(parts_needed(&"a".repeat(306)), (Encoding::Gsm7, 2, 0));
        assert_eq!(parts_needed(&"a".repeat(307)), (Encoding::Gsm7, 3, 152));
        assert_eq!(parts_needed("€"), (Encoding::Gsm7, 1, 158));
        assert_eq!(parts_needed("Ж"), (Encoding::Ucs2, 1, 69));
        assert_eq!(parts_needed("😀"), (Encoding::Ucs2, 1, 68));
        assert_eq!(parts_needed(&"Ж".repeat(71)), (Encoding::Ucs2, 2, 63));
        assert_eq!(split_text("hi", 9), vec![UserData::text("hi")]);
        // Capped at 255 parts.
        assert_eq!(split_text(&"a".repeat(153 * 300), 0).len(), 255);
    }

    #[test]
    fn alphanumeric_address() {
        // Widely published: "Design@Home" = 14 D0 C4F23C7D760390EF7619.
        let mut out = Vec::new();
        encode_tp_addr(&Address::Alphanumeric("Design@Home".into()), &mut out);
        assert_eq!(to_hex(&out), "14D0C4F23C7D760390EF7619");
        assert_eq!(decode_tp_addr(&mut Rd::new(&out)).unwrap(), Address::Alphanumeric("Design@Home".into()));

        let d = Deliver {
            more_messages: false,
            status_report_indication: false,
            reply_path: false,
            originator: Address::parse("CARRIER"),
            pid: 0,
            timestamp: ts(),
            ud: UserData::text("Your bill is ready"),
        };
        let b = Tpdu::Deliver(d.clone()).encode();
        assert_eq!(&b[1..3], &[13, 0xD0]);
        assert_eq!(Tpdu::decode_mt(&b).unwrap(), Tpdu::Deliver(d));
        // Long names are cut to 11 septets.
        let mut out = Vec::new();
        encode_tp_addr(&Address::Alphanumeric("ABCDEFGHIJKLMNOP".into()), &mut out);
        assert_eq!(decode_tp_addr(&mut Rd::new(&out)).unwrap(), Address::Alphanumeric("ABCDEFGHIJK".into()));
    }

    #[test]
    fn address_parse_and_bcd() {
        assert_eq!(Address::parse("+15551234567"), Address::International("15551234567".into()));
        assert_eq!(Address::parse("611"), Address::Unknown("611".into()));
        assert_eq!(Address::parse("*86#"), Address::Unknown("*86#".into()));
        assert_eq!(Address::parse("abc"), Address::Alphanumeric("abc".into()));
        assert_eq!(Address::parse("+"), Address::Alphanumeric("+".into()));
        let mut out = Vec::new();
        encode_tp_addr(&Address::National("5551234".into()), &mut out);
        assert_eq!(to_hex(&out), "07A1551532F4");
        assert_eq!(decode_tp_addr(&mut Rd::new(&out)).unwrap(), Address::National("5551234".into()));
        // Filler inside, missing filler, bad TOA, too long.
        assert!(decode_tp_addr(&mut Rd::new(&hex("04815F21"))).is_err());
        assert!(decode_tp_addr(&mut Rd::new(&hex("03815521"))).is_err());
        assert!(decode_tp_addr(&mut Rd::new(&hex("02115521"))).is_err());
        assert!(decode_tp_addr(&mut Rd::new(&hex("1591"))).is_err());
        let other = Address::Other { toa: 0x80, digits: "12".into() };
        let mut out = Vec::new();
        encode_tp_addr(&other, &mut out);
        assert_eq!(decode_tp_addr(&mut Rd::new(&out)).unwrap(), other);
    }

    #[test]
    fn status_report_round_trip() {
        let r = StatusReport {
            more_messages: false,
            srq: false,
            mr: 0x2A,
            recipient: Address::International("15551234567".into()),
            sc_timestamp: ts(),
            discharge_time: Timestamp { second: 45, ..ts() },
            status: status::DELIVERED,
        };
        let b = Tpdu::StatusReport(r.clone()).encode();
        assert_eq!(b[0], 0x06);
        assert_eq!(Tpdu::decode_mt(&b).unwrap(), Tpdu::StatusReport(r.clone()));
        // With TP-PI: PID + DCS + UD present.
        let mut ext = b.clone();
        ext.extend([0x07, 0x00, 0x00, 0x02, 0xC8, 0x34]);
        assert_eq!(Tpdu::decode_mt(&ext).unwrap(), Tpdu::StatusReport(r));
        ext.push(0);
        assert_eq!(Tpdu::decode_mt(&ext), Err(SmsError::Trailing));
    }

    #[test]
    fn wap_push_ports_and_binary() {
        let wsp = hex("0106226170706C69636174696F6E2F766E642E7761702E6D6D732D6D65737361676500");
        let ud = UserData {
            udh: vec![Ie::Ports16 { dest: WAP_PUSH_PORT, src: WAP_PUSH_SOURCE_PORT }],
            body: Body::Binary(wsp.clone()),
        };
        let d = Deliver {
            more_messages: false,
            status_report_indication: false,
            reply_path: false,
            originator: Address::International("15550000000".into()),
            pid: 0,
            timestamp: ts(),
            ud: ud.clone(),
        };
        let b = Tpdu::Deliver(d).encode();
        let ud_at = b.len() - wsp.len() - 7;
        assert_eq!(b[ud_at - 2 - 7], 0x04, "DCS 8-bit");
        assert_eq!(b[ud_at - 1] as usize, 7 + wsp.len());
        assert_eq!(to_hex(&b[ud_at..ud_at + 7]), "0605040B8423F0");
        let back = deliver_of(Tpdu::decode_mt(&b).unwrap()).ud;
        assert_eq!(back, ud);
        assert_eq!(back.ports(), Some((2948, 9200)));
    }

    #[test]
    fn concat16_and_port_ie_together() {
        let ud = UserData {
            udh: vec![
                Ie::Ports16 { dest: 2948, src: 9200 },
                Ie::Concat16 { reference: 0xBEEF, total: 2, seq: 2 },
                Ie::Other { id: 0x24, data: vec![1] },
            ],
            body: Body::Binary(vec![9; 100]),
        };
        let b = Tpdu::Submit(Submit::new(3, Address::Unknown("2948".into()), ud.clone())).encode();
        let s = submit_of(Tpdu::decode_mo(&b).unwrap());
        assert_eq!(s.ud, ud);
        assert_eq!(s.ud.concat(), Some((0xBEEF, 2, 2)));
        // Bad concat IE values are rejected.
        for bad in ["0003010001", "0003010203", "000201FF", "0804BEEF0200", "0804BEEF0102", "0503000000", "0005"] {
            assert!(parse_ies(&hex(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn gsm7_with_odd_header_fill_bits() {
        // Ports (6 octets) + concat (5 octets) + UDHL = 12 octets = 96 bits -> 14 septets, 2 fill bits.
        let ud = UserData {
            udh: vec![Ie::Ports16 { dest: 1, src: 2 }, Ie::Concat8 { reference: 1, total: 3, seq: 3 }],
            body: Body::Text("Fill bits {ok}".into()),
        };
        let b = Tpdu::Submit(Submit::new(0, Address::Unknown("1".into()), ud.clone())).encode();
        assert_eq!(submit_of(Tpdu::decode_mo(&b).unwrap()).ud, ud);
        let septets = gsm7_encode("Fill bits {ok}").unwrap();
        let tail = pack_septets(&septets, 2);
        assert!(b.ends_with(&tail));
    }

    #[test]
    fn oversize_bodies_are_cut_to_fit() {
        let s = Submit::new(0, Address::Unknown("1".into()), UserData::text(&format!("{}€", "a".repeat(159))));
        let d = submit_of(Tpdu::decode_mo(&Tpdu::Submit(s).encode()).unwrap());
        assert_eq!(d.ud.body, Body::Text("a".repeat(159)));
        let s = Submit::new(0, Address::Unknown("1".into()), UserData::text(&format!("{}😀", "Ж".repeat(69))));
        let d = submit_of(Tpdu::decode_mo(&Tpdu::Submit(s).encode()).unwrap());
        assert_eq!(d.ud.body, Body::Text("Ж".repeat(69)));
        let s = Submit::new(0, Address::Unknown("1".into()), UserData::binary(&[1; 300]));
        let d = submit_of(Tpdu::decode_mo(&Tpdu::Submit(s).encode()).unwrap());
        assert_eq!(d.ud.body, Body::Binary(vec![1; 140]));
    }

    #[test]
    fn dcs_groups() {
        assert_eq!(Encoding::from_dcs(0x00), Ok(Encoding::Gsm7));
        assert_eq!(Encoding::from_dcs(0x08), Ok(Encoding::Ucs2));
        assert_eq!(Encoding::from_dcs(0x04), Ok(Encoding::Data8));
        assert_eq!(Encoding::from_dcs(0x11), Ok(Encoding::Gsm7));
        assert_eq!(Encoding::from_dcs(0xF1), Ok(Encoding::Gsm7));
        assert_eq!(Encoding::from_dcs(0xF5), Ok(Encoding::Data8));
        assert_eq!(Encoding::from_dcs(0xE0), Ok(Encoding::Ucs2));
        assert!(Encoding::from_dcs(0x20).is_err());
        assert!(Encoding::from_dcs(0x0C).is_err());
        assert!(Encoding::from_dcs(0x80).is_err());
        // DCS 0xF1 (class 1) still decodes as GSM 7-bit text.
        let pdu = hex("11000B916407281553F800F1AA0AE8329BFD4697D9EC37");
        assert_eq!(submit_of(Tpdu::decode_mo(&pdu).unwrap()).ud, UserData::text("hellohello"));
    }

    #[test]
    fn validity_formats() {
        assert_eq!(Validity::relative_minutes(0), 5);
        assert_eq!(Validity::relative_minutes(143), 720);
        assert_eq!(Validity::relative_minutes(0xA7), 1440);
        assert_eq!(Validity::relative_minutes(255), 63 * 10_080);
        assert_eq!(Validity::relative_from_minutes(1440), 0xA7);
        assert_eq!(Validity::relative_from_minutes(1), 0);
        assert_eq!(Validity::relative_from_minutes(u32::MAX), 0xFF);
        for v in [Validity::None, Validity::Absolute(ts()), Validity::Enhanced([1, 2, 3, 4, 5, 6, 7])] {
            let s = Submit { validity: v, ..Submit::new(9, Address::Unknown("1".into()), UserData::text("x")) };
            assert_eq!(submit_of(Tpdu::decode_mo(&Tpdu::Submit(s.clone()).encode()).unwrap()), s);
        }
    }

    #[test]
    fn timestamps() {
        let t = Timestamp::from_game_millis(0);
        assert_eq!(t, Timestamp { year: 0, month: 1, day: 1, hour: 0, minute: 0, second: 0, tz_quarters: 0 });
        let ms = (8825u64 * 86_400 + 12 * 3600 + 34 * 60 + 56) * 1000 + 789;
        let t = Timestamp::from_game_millis(ms);
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute, t.second),
            (24, 2, 29, 12, 34, 56)
        );
        assert_eq!(t.to_game_millis(), ms - 789);
        let shifted = Timestamp { hour: 7, minute: 34, tz_quarters: -20, ..t };
        assert_eq!(shifted.to_game_millis(), ms - 789);
        // Bad digits / ranges.
        for bad in ["A201719050030A", "62317190500300", "620171905003A0", "62000000000000"] {
            assert!(Timestamp::decode(&mut Rd::new(&hex(bad))).is_err(), "{bad}");
        }
        let mut out = Vec::new();
        ts().encode(&mut out);
        assert_eq!(Timestamp::decode(&mut Rd::new(&out)).unwrap(), ts());
    }

    #[test]
    fn reassembler_order_duplicates_expiry() {
        let text = "x".repeat(400);
        let parts = split_text(&text, 7);
        assert_eq!(parts.len(), 3);
        let mut re = Reassembler::new(1000);
        assert_eq!(re.push("A", parts[2].clone(), 0), None);
        assert_eq!(re.push("A", parts[2].clone(), 1), None, "duplicate");
        // Same ref from another sender is a different set.
        assert_eq!(re.push("B", parts[0].clone(), 2), None);
        assert_eq!(re.pending(), 2);
        assert_eq!(re.push("A", parts[0].clone(), 3), None);
        assert_eq!(re.push("A", parts[1].clone(), 4).unwrap().body, Body::Text(text.clone()));
        assert_eq!(re.pending(), 1);
        // B's set expires.
        assert_eq!(re.expire(1001), 0);
        assert_eq!(re.expire(1002), 1);
        assert_eq!(re.pending(), 0);
        // A late part starts a fresh set and never completes alone.
        assert_eq!(re.push("A", parts[1].clone(), 5000), None);
        assert_eq!(re.push("A", parts[0].clone(), 6001), None);
        assert_eq!(re.pending(), 1, "the 5000 set expired, a new one began");
        // Unconcatenated passes straight through.
        assert_eq!(re.push("C", UserData::text("hi"), 6002), Some(UserData::text("hi")));
        // Invalid concat info is dropped.
        let bad = UserData { udh: vec![Ie::Concat8 { reference: 1, total: 2, seq: 3 }], body: Body::Text("z".into()) };
        assert_eq!(re.push("C", bad, 6003), None);
        // Bounded.
        for i in 0..(MAX_PENDING_SETS + 10) {
            re.push(&format!("s{i}"), parts[0].clone(), 6004);
        }
        assert_eq!(re.pending(), MAX_PENDING_SETS);
    }

    #[test]
    fn rp_vectors_and_round_trips() {
        let smsc = Address::International("15550000001".into());
        let tpdu = hex("11000B916407281553F80000AA0AE8329BFD4697D9EC37");
        let mo = Rp::Data { mr: 0x10, smsc: smsc.clone(), tpdu: tpdu.clone() };
        let b = mo.encode(Direction::MsToNetwork);
        assert_eq!(to_hex(&b[..11]), "00100007915155000000F1");
        assert_eq!(b[11] as usize, tpdu.len());
        assert_eq!(Rp::decode(&b, Direction::MsToNetwork).unwrap(), mo);
        assert!(Rp::decode(&b, Direction::NetworkToMs).is_err());

        let mt = Rp::Data { mr: 0x11, smsc: smsc.clone(), tpdu: tpdu.clone() };
        let b = mt.encode(Direction::NetworkToMs);
        assert_eq!(to_hex(&b[..3]), "011107");
        assert_eq!(b[10], 0x00, "empty RP-DA");
        assert_eq!(Rp::decode(&b, Direction::NetworkToMs).unwrap(), mt);

        let ack = Rp::Ack { mr: 3, tpdu: None };
        assert_eq!(ack.encode(Direction::MsToNetwork), vec![0x02, 0x03]);
        assert_eq!(Rp::decode(&[0x03, 0x03], Direction::NetworkToMs).unwrap(), ack);
        let ack_ud = Rp::Ack { mr: 3, tpdu: Some(vec![0x00, 0x00]) };
        assert_eq!(ack_ud.encode(Direction::MsToNetwork), hex("0203410200 00"));
        assert_eq!(Rp::decode(&hex("020341020000"), Direction::MsToNetwork).unwrap(), ack_ud);

        let err = Rp::Error { mr: 7, cause: cause::MEMORY_CAPACITY_EXCEEDED, diagnostic: None, tpdu: None };
        assert_eq!(err.encode(Direction::MsToNetwork), hex("04070116"));
        assert_eq!(Rp::decode(&hex("04070116"), Direction::MsToNetwork).unwrap(), err);
        for c in [
            cause::UNASSIGNED_NUMBER,
            cause::SHORT_MESSAGE_TRANSFER_REJECTED,
            cause::DESTINATION_OUT_OF_ORDER,
            cause::CONGESTION,
        ] {
            let e = Rp::Error { mr: 1, cause: c, diagnostic: Some(9), tpdu: Some(vec![1, 2]) };
            assert_eq!(Rp::decode(&e.encode(Direction::NetworkToMs), Direction::NetworkToMs).unwrap(), e);
        }
        assert_eq!(Rp::Smma { mr: 4 }.encode(Direction::MsToNetwork), vec![6, 4]);
        assert_eq!(Rp::decode(&[6, 4], Direction::MsToNetwork).unwrap(), Rp::Smma { mr: 4 });
        assert_eq!(Rp::Smma { mr: 4 }.mr(), 4);

        // Strictness.
        assert!(Rp::decode(&[0x07, 0], Direction::NetworkToMs).is_err());
        assert!(Rp::decode(&[0x08, 0], Direction::MsToNetwork).is_err());
        assert!(Rp::decode(&hex("04070016"), Direction::MsToNetwork).is_err());
        assert!(Rp::decode(&hex("04070196"), Direction::MsToNetwork).is_err());
        assert!(Rp::decode(&hex("020342020000"), Direction::MsToNetwork).is_err());
        assert_eq!(Rp::decode(&[6, 4, 0], Direction::MsToNetwork), Err(SmsError::Trailing));
        // MO RP-DATA with an originator is rejected.
        let mut bad = mt.encode(Direction::NetworkToMs);
        bad[0] = 0;
        assert!(Rp::decode(&bad, Direction::MsToNetwork).is_err());
    }

    #[test]
    fn mti_routing() {
        assert_eq!(Tpdu::decode_mo(&[0x00]), Err(SmsError::Unsupported("SMS-DELIVER-REPORT")));
        assert_eq!(Tpdu::decode_mo(&[0x02]), Err(SmsError::Unsupported("SMS-COMMAND")));
        assert_eq!(Tpdu::decode_mt(&[0x01]), Err(SmsError::Unsupported("SMS-SUBMIT-REPORT")));
        assert!(Tpdu::decode_mt(&[0x03]).is_err());
        assert!(Tpdu::decode_mo(&[]).is_err());
        let mut pdu = hex("11000B916407281553F80000AA0AE8329BFD4697D9EC37");
        pdu.push(0);
        assert_eq!(Tpdu::decode_mo(&pdu), Err(SmsError::Trailing));
    }

    type Samples = (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<(Vec<u8>, Direction)>);

    fn valid_samples() -> Samples {
        let mo = {
            let mut v = vec![hex("11000B916407281553F80000AA0AE8329BFD4697D9EC37")];
            let text = "z".repeat(170);
            for p in split_text(&text, 3) {
                v.push(Tpdu::Submit(Submit::new(1, Address::International("15551234567".into()), p)).encode());
            }
            v.push(Tpdu::Submit(Submit::new(2, Address::Unknown("611".into()), UserData::text("Ж😀"))).encode());
            v
        };
        let mt = vec![
            hex("040B911346610089F60000208062917314080CC8F71D14969741F977FD07"),
            Tpdu::Deliver(Deliver {
                more_messages: true,
                status_report_indication: false,
                reply_path: true,
                originator: Address::Alphanumeric("CARRIER".into()),
                pid: 0,
                timestamp: ts(),
                ud: UserData {
                    udh: vec![Ie::Ports16 { dest: 2948, src: 9200 }],
                    body: Body::Binary(vec![1, 6, 0x22]),
                },
            })
            .encode(),
            Tpdu::StatusReport(StatusReport {
                more_messages: false,
                srq: false,
                mr: 1,
                recipient: Address::International("15551234567".into()),
                sc_timestamp: ts(),
                discharge_time: ts(),
                status: 0,
            })
            .encode(),
        ];
        let smsc = Address::International("15550000001".into());
        let rp = vec![
            (Rp::Data { mr: 1, smsc: smsc.clone(), tpdu: mo[0].clone() }.encode(Direction::MsToNetwork), Direction::MsToNetwork),
            (Rp::Data { mr: 2, smsc, tpdu: mt[0].clone() }.encode(Direction::NetworkToMs), Direction::NetworkToMs),
            (
                Rp::Error { mr: 3, cause: 27, diagnostic: Some(1), tpdu: None }.encode(Direction::NetworkToMs),
                Direction::NetworkToMs,
            ),
            (Rp::Smma { mr: 4 }.encode(Direction::MsToNetwork), Direction::MsToNetwork),
        ];
        (mo, mt, rp)
    }

    #[test]
    fn every_truncation_is_an_error() {
        let (mo, mt, rp) = valid_samples();
        for p in &mo {
            assert!(Tpdu::decode_mo(p).is_ok());
            for n in 0..p.len() {
                assert!(Tpdu::decode_mo(&p[..n]).is_err(), "mo prefix {n} of {}", to_hex(p));
            }
        }
        for p in &mt {
            assert!(Tpdu::decode_mt(p).is_ok());
            for n in 0..p.len() {
                assert!(Tpdu::decode_mt(&p[..n]).is_err(), "mt prefix {n} of {}", to_hex(p));
            }
        }
        for (p, dir) in &rp {
            assert!(Rp::decode(p, *dir).is_ok());
            for n in 0..p.len() {
                assert!(Rp::decode(&p[..n], *dir).is_err(), "rp prefix {n} of {}", to_hex(p));
            }
        }
    }

    #[test]
    fn fuzz_never_panics() {
        let (mo, mt, rp) = valid_samples();
        let mut seeds: Vec<Vec<u8>> = mo.clone();
        seeds.extend(mt.clone());
        seeds.extend(rp.iter().map(|(p, _)| p.clone()));
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut re = Reassembler::new(100);
        for i in 0..20_000u64 {
            let input: Vec<u8> = if i % 2 == 0 {
                let len = (next() % 80) as usize;
                (0..len).map(|_| next() as u8).collect()
            } else {
                let mut v = seeds[(next() % seeds.len() as u64) as usize].clone();
                for _ in 0..1 + next() % 3 {
                    let at = (next() % v.len() as u64) as usize;
                    v[at] ^= 1 << (next() % 8);
                }
                if next() % 4 == 0 {
                    let cut = (next() % (v.len() as u64 + 1)) as usize;
                    v.truncate(cut);
                }
                v
            };
            for t in [Tpdu::decode_mo(&input), Tpdu::decode_mt(&input)].into_iter().flatten() {
                let b = t.encode();
                let again = match &t {
                    Tpdu::Submit(_) => Tpdu::decode_mo(&b),
                    _ => Tpdu::decode_mt(&b),
                };
                assert!(again.is_ok(), "re-encode of {} failed: {again:?}", to_hex(&input));
                let ud = match t {
                    Tpdu::Submit(s) => Some(s.ud),
                    Tpdu::Deliver(d) => Some(d.ud),
                    Tpdu::StatusReport(_) => None,
                };
                if let Some(ud) = ud {
                    re.push("f", ud, i);
                }
            }
            for dir in [Direction::MsToNetwork, Direction::NetworkToMs] {
                if let Ok(m) = Rp::decode(&input, dir) {
                    let _ = m.encode(dir);
                }
            }
            let _ = ucs2_decode(&input);
            let _ = unpack_septets(&input, (i % 7) as usize, input.len());
            let _ = gsm7_decode(&input);
            let _ = parse_ies(&input);
        }
    }
}
