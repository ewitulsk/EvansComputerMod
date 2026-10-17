//! OMA MMS encapsulation (MMS 1.2/1.3 binary PDUs) on WSP encoding (WAP-230),
//! multipart bodies, and the WAP Push PDU that carries an MMS notification.
//!
//! **PDU** ([`Pdu`]): a run of headers, then (for `m-send-req` and
//! `m-retrieve-conf` only) a body. Each header is one octet, the MMS field
//! number with the high bit set (`0x81` Bcc ... `0x9B` X-Mms-Read-Status, see
//! the `H_*` constants), followed by its value. The first three headers are
//! fixed: `X-Mms-Message-Type` (`0x8C`), `X-Mms-Transaction-Id` (`0x98`, when
//! the PDU type has one) and `X-Mms-MMS-Version` (`0x8D`, short-integer, `0x92`
//! = 1.2). The rest may come in any order, except `Content-Type` (`0x84`),
//! which is always last: the body starts right after its value.
//!
//! ```text
//! m-notifyresp-ind:  8C 83 | 98 "T1" 00 | 8D 92 | 95 81   (Status = Retrieved)
//! ```
//!
//! **WSP primitives** (WAP-230 section 8.4.2):
//! - *uintvar*: 7 bits per octet, high bit = more follows, at most 5 octets (u32).
//! - *Short-integer*: one octet `0x80 | n` (n < 128). *Long-integer*: a length
//!   octet 1..=30 then that many big-endian octets. *Integer-value*: either.
//! - *Value-length*: one octet 0..=30, or `0x1F` then a uintvar.
//! - *Text-string*: text then `0x00`; a `0x7F` quote goes first when the first
//!   octet is >= 0x80 (this encoder also quotes a leading `"`, `0x7F` or control
//!   octet so every string round-trips). *Quoted-string*: `0x22` text `0x00`.
//! - *Encoded-string-value*: a Text-string, or Value-length, charset
//!   (Integer-value, UTF-8 = 106 = `0xEA`) and a Text-string. Non-ASCII strings
//!   are always sent with the charset form.
//! - *Content-type*: a short-integer well-known media (WAP-230 Table 40, e.g.
//!   `0x83` text/plain, `0xA0` image/png, `0xA3` multipart.mixed, `0xB3`
//!   multipart.related, `0xBE` application/vnd.wap.mms-message), or the media as
//!   text, or with parameters the general form: Value-length, media, then
//!   parameters (`0x81` charset, `0x89` type, `0x8A` start, `0x85` name,
//!   `0x86` filename; the WSP 1.4 numbers and untyped text parameters decode too).
//!
//! **Multipart** ([`Body`], [`Part`]): uintvar part count, then per part
//! uintvar headers-length, uintvar data-length, the part's content-type and
//! part headers (`0xC0` Content-ID as a quoted `<id>`, `0x8E` Content-Location)
//! inside headers-length, then the data. [`Body::message`] builds the usual
//! text + attachments message, optionally with a minimal SMIL part (and the
//! multipart.related `type`/`start` parameters); [`Body::text`] and
//! [`Body::attachments`] read it back.
//!
//! **WAP Push** ([`wap_push_encode`], [`wap_push_decode`]): the connectionless
//! WSP Push PDU, `TID, 0x06, uintvar headers-length, content-type, headers,
//! data`; for MMS the content type is `0xBE` and the header
//! `X-Wap-Application-Id: x-wap-application:mms.ua` (`0xAF 0x84`). The SMS
//! layer sends these octets to port 2948.
//!
//! Addresses are `"+12145550142/TYPE=PLMN"` ([`plmn_address`],
//! [`parse_address`]). Decoders are strict and never panic: every length is
//! bounds-checked, trailing octets inside a length are errors, the fixed
//! header order and each PDU type's mandatory headers are enforced.

use std::fmt;

// ---------------------------------------------------------------------------
// Errors

/// Why a PDU did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmsError {
    /// The input ended inside a value.
    Short,
    /// A malformed or out-of-range value.
    Value(&'static str),
    /// An X-Mms-Message-Type this module doesn't handle.
    MessageType(u8),
    /// A mandatory header for this PDU type is absent.
    Missing(&'static str),
    /// A singular header appeared twice (the header octet).
    Duplicate(u8),
    /// Octets left over inside a length-delimited value.
    Trailing,
}

impl fmt::Display for MmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MmsError::Short => write!(f, "truncated MMS PDU"),
            MmsError::Value(what) => write!(f, "bad MMS value: {what}"),
            MmsError::MessageType(t) => write!(f, "unsupported X-Mms-Message-Type 0x{t:02X}"),
            MmsError::Missing(what) => write!(f, "missing mandatory header {what}"),
            MmsError::Duplicate(h) => write!(f, "duplicate header 0x{h:02X}"),
            MmsError::Trailing => write!(f, "trailing octets in MMS value"),
        }
    }
}

impl std::error::Error for MmsError {}

pub type MmsResult<T> = Result<T, MmsError>;

// ---------------------------------------------------------------------------
// Constants

pub const H_BCC: u8 = 0x81;
pub const H_CC: u8 = 0x82;
pub const H_CONTENT_LOCATION: u8 = 0x83;
pub const H_CONTENT_TYPE: u8 = 0x84;
pub const H_DATE: u8 = 0x85;
pub const H_DELIVERY_REPORT: u8 = 0x86;
pub const H_DELIVERY_TIME: u8 = 0x87;
pub const H_EXPIRY: u8 = 0x88;
pub const H_FROM: u8 = 0x89;
pub const H_MESSAGE_CLASS: u8 = 0x8A;
pub const H_MESSAGE_ID: u8 = 0x8B;
pub const H_MESSAGE_TYPE: u8 = 0x8C;
pub const H_MMS_VERSION: u8 = 0x8D;
pub const H_MESSAGE_SIZE: u8 = 0x8E;
pub const H_PRIORITY: u8 = 0x8F;
pub const H_READ_REPORT: u8 = 0x90;
pub const H_REPORT_ALLOWED: u8 = 0x91;
pub const H_RESPONSE_STATUS: u8 = 0x92;
pub const H_RESPONSE_TEXT: u8 = 0x93;
pub const H_SENDER_VISIBILITY: u8 = 0x94;
pub const H_STATUS: u8 = 0x95;
pub const H_SUBJECT: u8 = 0x96;
pub const H_TO: u8 = 0x97;
pub const H_TRANSACTION_ID: u8 = 0x98;
pub const H_RETRIEVE_STATUS: u8 = 0x99;
pub const H_RETRIEVE_TEXT: u8 = 0x9A;
pub const H_READ_STATUS: u8 = 0x9B;

/// X-Mms-MMS-Version 1.2 (major in bits 4-6, minor in bits 0-3).
pub const VERSION_1_2: u8 = 0x12;
/// X-Mms-MMS-Version 1.3.
pub const VERSION_1_3: u8 = 0x13;

/// IANA MIBenum of UTF-8.
pub const CHARSET_UTF8: u32 = 106;
pub const CHARSET_US_ASCII: u32 = 3;
pub const CHARSET_ISO_8859_1: u32 = 4;

pub const MMS_CONTENT_TYPE: &str = "application/vnd.wap.mms-message";
pub const MULTIPART_RELATED: &str = "application/vnd.wap.multipart.related";
pub const MULTIPART_MIXED: &str = "application/vnd.wap.multipart.mixed";
pub const SMIL: &str = "application/smil";
/// WAP push application port for MMS (the SMS layer's business).
pub const WAP_PUSH_PORT: u16 = 2948;

/// WSP PDU type Push.
const WSP_PUSH: u8 = 0x06;
/// WSP PDU type ConfirmedPush (accepted on decode).
const WSP_CONFIRMED_PUSH: u8 = 0x07;
/// WSP header X-Wap-Application-Id (0x2F).
const WSP_APP_ID: u8 = 0xAF;
/// x-wap-application:mms.ua.
const WSP_APP_MMS_UA: u8 = 0x84;
/// WSP part header Content-ID (0x40).
const WSP_CONTENT_ID: u8 = 0xC0;
/// WSP part header Content-Location (0x0E).
const WSP_CONTENT_LOCATION: u8 = 0x8E;

/// WAP-230 Appendix A Table 40, well-known content types, indexed by number.
/// (Stored lowercase: MIME types compare case-insensitively.)
const WELL_KNOWN_MEDIA: &[&str] = &[
    "*/*",                                        // 0x00
    "text/*",                                     // 0x01
    "text/html",                                  // 0x02
    "text/plain",                                 // 0x03
    "text/x-hdml",                                // 0x04
    "text/x-ttml",                                // 0x05
    "text/x-vcalendar",                           // 0x06
    "text/x-vcard",                               // 0x07
    "text/vnd.wap.wml",                           // 0x08
    "text/vnd.wap.wmlscript",                     // 0x09
    "text/vnd.wap.wta-event",                     // 0x0A
    "multipart/*",                                // 0x0B
    "multipart/mixed",                            // 0x0C
    "multipart/form-data",                        // 0x0D
    "multipart/byterantes",                       // 0x0E (sic, as assigned)
    "multipart/alternative",                      // 0x0F
    "application/*",                              // 0x10
    "application/java-vm",                        // 0x11
    "application/x-www-form-urlencoded",          // 0x12
    "application/x-hdmlc",                        // 0x13
    "application/vnd.wap.wmlc",                   // 0x14
    "application/vnd.wap.wmlscriptc",             // 0x15
    "application/vnd.wap.wta-eventc",             // 0x16
    "application/vnd.wap.uaprof",                 // 0x17
    "application/vnd.wap.wtls-ca-certificate",    // 0x18
    "application/vnd.wap.wtls-user-certificate",  // 0x19
    "application/x-x509-ca-cert",                 // 0x1A
    "application/x-x509-user-cert",               // 0x1B
    "image/*",                                    // 0x1C
    "image/gif",                                  // 0x1D
    "image/jpeg",                                 // 0x1E
    "image/tiff",                                 // 0x1F
    "image/png",                                  // 0x20
    "image/vnd.wap.wbmp",                         // 0x21
    "application/vnd.wap.multipart.*",            // 0x22
    "application/vnd.wap.multipart.mixed",        // 0x23
    "application/vnd.wap.multipart.form-data",    // 0x24
    "application/vnd.wap.multipart.byteranges",   // 0x25
    "application/vnd.wap.multipart.alternative",  // 0x26
    "application/xml",                            // 0x27
    "text/xml",                                   // 0x28
    "application/vnd.wap.wbxml",                  // 0x29
    "application/x-x968-cross-cert",              // 0x2A
    "application/x-x968-ca-cert",                 // 0x2B
    "application/x-x968-user-cert",               // 0x2C
    "text/vnd.wap.si",                            // 0x2D
    "application/vnd.wap.sic",                    // 0x2E
    "text/vnd.wap.sl",                            // 0x2F
    "application/vnd.wap.slc",                    // 0x30
    "text/vnd.wap.co",                            // 0x31
    "application/vnd.wap.coc",                    // 0x32
    "application/vnd.wap.multipart.related",      // 0x33
    "application/vnd.wap.sia",                    // 0x34
    "text/vnd.wap.connectivity-xml",              // 0x35
    "application/vnd.wap.connectivity-wbxml",     // 0x36
    "application/pkcs7-mime",                     // 0x37
    "application/vnd.wap.hashed-certificate",     // 0x38
    "application/vnd.wap.signed-certificate",     // 0x39
    "application/vnd.wap.cert-response",          // 0x3A
    "application/xhtml+xml",                      // 0x3B
    "application/wml+xml",                        // 0x3C
    "text/css",                                   // 0x3D
    "application/vnd.wap.mms-message",            // 0x3E
    "application/vnd.wap.rollover-certificate",   // 0x3F
    "application/vnd.wap.locc+wbxml",             // 0x40
    "application/vnd.wap.loc+xml",                // 0x41
    "application/vnd.syncml.dm+wbxml",            // 0x42
    "application/vnd.syncml.dm+xml",              // 0x43
    "application/vnd.syncml.notification",        // 0x44
    "application/vnd.wap.xhtml+xml",              // 0x45
    "application/vnd.wv.csp.cir",                 // 0x46
    "application/vnd.oma.dd+xml",                 // 0x47
    "application/vnd.oma.drm.message",            // 0x48
    "application/vnd.oma.drm.content",            // 0x49
    "application/vnd.oma.drm.rights+xml",         // 0x4A
    "application/vnd.oma.drm.rights+wbxml",       // 0x4B
];

/// The WSP number of a well-known media type, if it has one.
pub fn well_known_media(media: &str) -> Option<u8> {
    WELL_KNOWN_MEDIA.iter().position(|m| m.eq_ignore_ascii_case(media)).map(|i| i as u8)
}

fn media_name(n: u64) -> MmsResult<String> {
    usize::try_from(n)
        .ok()
        .and_then(|i| WELL_KNOWN_MEDIA.get(i))
        .map(|s| s.to_string())
        .ok_or(MmsError::Value("unassigned well-known media"))
}

fn is_multipart_media(media: &str) -> bool {
    let m = media.to_ascii_lowercase();
    m.starts_with("application/vnd.wap.multipart.") || m.starts_with("multipart/")
}

// ---------------------------------------------------------------------------
// Reader

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn new(b: &'a [u8]) -> R<'a> {
        R { b, p: 0 }
    }
    fn done(&self) -> bool {
        self.p >= self.b.len()
    }
    fn rest(&self) -> &'a [u8] {
        &self.b[self.p.min(self.b.len())..]
    }
    fn peek(&self) -> MmsResult<u8> {
        self.b.get(self.p).copied().ok_or(MmsError::Short)
    }
    fn u8(&mut self) -> MmsResult<u8> {
        let o = self.peek()?;
        self.p += 1;
        Ok(o)
    }
    fn take(&mut self, n: usize) -> MmsResult<&'a [u8]> {
        if self.b.len() - self.p < n {
            return Err(MmsError::Short);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn end(&self) -> MmsResult<()> {
        if self.done() {
            Ok(())
        } else {
            Err(MmsError::Trailing)
        }
    }
    fn uintvar(&mut self) -> MmsResult<u32> {
        let mut v: u32 = 0;
        for i in 0..5 {
            let o = self.u8()?;
            if i == 4 && v > (u32::MAX >> 7) {
                return Err(MmsError::Value("uintvar overflow"));
            }
            v = (v << 7) | (o & 0x7F) as u32;
            if o & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(MmsError::Value("uintvar longer than 5 octets"))
    }
    fn short_int(&mut self) -> MmsResult<u8> {
        let o = self.u8()?;
        if o & 0x80 == 0 {
            return Err(MmsError::Value("short-integer"));
        }
        Ok(o & 0x7F)
    }
    fn long_int(&mut self) -> MmsResult<u64> {
        let n = self.u8()?;
        if n == 0 || n > 30 {
            return Err(MmsError::Value("long-integer length"));
        }
        let mut v: u64 = 0;
        for &x in self.take(n as usize)? {
            if v >> 56 != 0 {
                return Err(MmsError::Value("long-integer overflow"));
            }
            v = (v << 8) | x as u64;
        }
        Ok(v)
    }
    fn integer(&mut self) -> MmsResult<u64> {
        if self.peek()? >= 0x80 {
            Ok(self.short_int()? as u64)
        } else {
            self.long_int()
        }
    }
    fn value_length(&mut self) -> MmsResult<usize> {
        match self.u8()? {
            n @ 0..=30 => Ok(n as usize),
            31 => Ok(self.uintvar()? as usize),
            _ => Err(MmsError::Value("value-length")),
        }
    }
    /// A Value-length-delimited sub-reader.
    fn sub(&mut self) -> MmsResult<R<'a>> {
        let n = self.value_length()?;
        Ok(R::new(self.take(n)?))
    }
    /// Octets up to (not including) the next 0x00, which is consumed.
    fn cstr(&mut self) -> MmsResult<&'a [u8]> {
        let rest = self.rest();
        let i = rest.iter().position(|&c| c == 0).ok_or(MmsError::Short)?;
        self.p += i + 1;
        Ok(&rest[..i])
    }
    /// Text-string / Quoted-string octets, quote stripped.
    fn text_bytes(&mut self) -> MmsResult<&'a [u8]> {
        let o = self.peek()?;
        if o != 0 && !(32..0x80).contains(&o) {
            return Err(MmsError::Value("text-string"));
        }
        let s = self.cstr()?;
        Ok(match s.first() {
            Some(0x7F) | Some(0x22) => &s[1..],
            _ => s,
        })
    }
    fn text(&mut self) -> MmsResult<String> {
        Ok(String::from_utf8_lossy(self.text_bytes()?).into_owned())
    }
    /// Text-value: No-value (0x00) | Token-text | Quoted-string.
    fn text_value(&mut self) -> MmsResult<String> {
        self.text()
    }
    fn token(&mut self) -> MmsResult<String> {
        let s = self.cstr()?;
        if s.is_empty() || s.iter().any(|&c| !(33..0x7F).contains(&c)) {
            return Err(MmsError::Value("token-text"));
        }
        Ok(String::from_utf8_lossy(s).into_owned())
    }
    fn encoded_string(&mut self) -> MmsResult<String> {
        if self.peek()? <= 31 {
            let mut s = self.sub()?;
            let cs = charset_value(&mut s)?;
            let raw = s.text_bytes()?;
            s.end()?;
            Ok(decode_charset(cs, raw))
        } else {
            self.text()
        }
    }
    fn yes_no(&mut self) -> MmsResult<bool> {
        match self.u8()? {
            0x80 => Ok(true),
            0x81 => Ok(false),
            _ => Err(MmsError::Value("yes/no")),
        }
    }
}

/// Skip one generic WSP field value (length-prefixed, text, or short-integer).
fn skip_value(r: &mut R) -> MmsResult<()> {
    match r.peek()? {
        0..=31 => {
            let n = r.value_length()?;
            r.take(n)?;
        }
        32..=127 => {
            r.cstr()?;
        }
        _ => {
            r.u8()?;
        }
    }
    Ok(())
}

fn charset_by_name(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => CHARSET_UTF8,
        "us-ascii" | "ascii" => CHARSET_US_ASCII,
        "iso-8859-1" | "latin1" => CHARSET_ISO_8859_1,
        "iso-10646-ucs-2" => 1000,
        "utf-16" => 1015,
        _ => return None,
    })
}

/// Well-known-charset: Any-charset (0x80) | Integer-value (or, leniently, a name).
fn charset_value(r: &mut R) -> MmsResult<u32> {
    let o = r.peek()?;
    if (32..0x80).contains(&o) {
        let name = r.text()?;
        return charset_by_name(&name).ok_or(MmsError::Value("charset"));
    }
    u32::try_from(r.integer()?).map_err(|_| MmsError::Value("charset"))
}

fn decode_charset(cs: u32, raw: &[u8]) -> String {
    match cs {
        CHARSET_ISO_8859_1 => raw.iter().map(|&c| c as char).collect(),
        _ => String::from_utf8_lossy(raw).into_owned(),
    }
}

// ---------------------------------------------------------------------------
// Writer helpers

/// Append a uintvar.
pub fn put_uintvar(w: &mut Vec<u8>, v: u32) {
    let mut tmp = [0u8; 5];
    let mut n = 0;
    let mut x = v;
    loop {
        tmp[n] = (x & 0x7F) as u8;
        n += 1;
        x >>= 7;
        if x == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        w.push(tmp[i] | if i > 0 { 0x80 } else { 0 });
    }
}

/// Decode a uintvar from the front of `b`: (value, octets used).
pub fn get_uintvar(b: &[u8]) -> MmsResult<(u32, usize)> {
    let mut r = R::new(b);
    let v = r.uintvar()?;
    Ok((v, r.p))
}

fn put_long(w: &mut Vec<u8>, v: u64) {
    let bytes = v.to_be_bytes();
    let skip = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    w.push((8 - skip) as u8);
    w.extend_from_slice(&bytes[skip..]);
}

fn put_integer(w: &mut Vec<u8>, v: u64) {
    if v < 0x80 {
        w.push(0x80 | v as u8);
    } else {
        put_long(w, v);
    }
}

fn put_value_length(w: &mut Vec<u8>, n: usize) {
    if n <= 30 {
        w.push(n as u8);
    } else {
        w.push(31);
        put_uintvar(w, n as u32);
    }
}

fn with_length(w: &mut Vec<u8>, inner: &[u8]) {
    put_value_length(w, inner.len());
    w.extend_from_slice(inner);
}

/// Text-string. NULs can't be carried and are dropped.
fn put_text(w: &mut Vec<u8>, s: &str) {
    let b: Vec<u8> = s.bytes().filter(|&c| c != 0).collect();
    if let Some(&f) = b.first() {
        if !(0x20..0x7F).contains(&f) || f == 0x22 {
            w.push(0x7F);
        }
    }
    w.extend_from_slice(&b);
    w.push(0);
}

fn put_quoted(w: &mut Vec<u8>, s: &str) {
    w.push(0x22);
    w.extend(s.bytes().filter(|&c| c != 0));
    w.push(0);
}

fn put_encoded_string(w: &mut Vec<u8>, s: &str) {
    if s.is_ascii() {
        put_text(w, s);
    } else {
        let mut inner = vec![0x80 | CHARSET_UTF8 as u8];
        put_text(&mut inner, s);
        with_length(w, &inner);
    }
}

// ---------------------------------------------------------------------------
// Content-Type

/// A content type with the parameters MMS uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentType {
    /// The media type, e.g. `"image/png"`. Well-known types decode lowercase.
    pub media: String,
    /// Charset MIBenum (106 = UTF-8).
    pub charset: Option<u32>,
    /// multipart.related `type` (the root part's media, e.g. application/smil).
    pub related_type: Option<String>,
    /// multipart.related `start` (the root part's Content-ID, with `<>`).
    pub start: Option<String>,
    pub name: Option<String>,
    pub filename: Option<String>,
}

impl ContentType {
    pub fn new(media: impl Into<String>) -> ContentType {
        ContentType { media: media.into(), charset: None, related_type: None, start: None, name: None, filename: None }
    }
    pub fn is(&self, media: &str) -> bool {
        self.media.eq_ignore_ascii_case(media)
    }
    pub fn is_multipart(&self) -> bool {
        is_multipart_media(&self.media)
    }
    fn has_params(&self) -> bool {
        self.charset.is_some()
            || self.related_type.is_some()
            || self.start.is_some()
            || self.name.is_some()
            || self.filename.is_some()
    }
    /// The WSP Content-type-value octets.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Vec::new();
        self.write(&mut w);
        w
    }
    fn write(&self, w: &mut Vec<u8>) {
        let put_media = |w: &mut Vec<u8>, m: &str| match well_known_media(m) {
            Some(n) => w.push(0x80 | n),
            None => put_text(w, m),
        };
        if !self.has_params() {
            put_media(w, &self.media);
            return;
        }
        let mut inner = Vec::new();
        put_media(&mut inner, &self.media);
        if let Some(cs) = self.charset {
            inner.push(0x81);
            put_integer(&mut inner, cs as u64);
        }
        if let Some(t) = &self.related_type {
            inner.push(0x89);
            put_media(&mut inner, t);
        }
        if let Some(s) = &self.start {
            inner.push(0x8A);
            put_text(&mut inner, s);
        }
        if let Some(n) = &self.name {
            inner.push(0x85);
            put_text(&mut inner, n);
        }
        if let Some(f) = &self.filename {
            inner.push(0x86);
            put_text(&mut inner, f);
        }
        with_length(w, &inner);
    }
    /// Decode a whole Content-type-value (no trailing octets).
    pub fn decode(b: &[u8]) -> MmsResult<ContentType> {
        let mut r = R::new(b);
        let ct = content_type(&mut r)?;
        r.end()?;
        Ok(ct)
    }
}

/// Constrained-encoding: short-integer / long-integer media number, or text.
fn constrained_media(r: &mut R) -> MmsResult<String> {
    let o = r.peek()?;
    if o >= 0x80 || (1..=30).contains(&o) {
        media_name(r.integer()?)
    } else {
        let m = r.text()?;
        if m.is_empty() {
            return Err(MmsError::Value("empty media"));
        }
        Ok(m)
    }
}

fn content_type(r: &mut R) -> MmsResult<ContentType> {
    let o = r.peek()?;
    if o >= 32 {
        return Ok(ContentType::new(constrained_media(r)?));
    }
    let mut s = r.sub()?;
    let mut ct = ContentType::new(constrained_media(&mut s)?);
    while !s.done() {
        let o = s.peek()?;
        if o >= 0x80 || o <= 30 {
            match s.integer()? {
                0x00 => {
                    s.uintvar()?; // Q-value
                }
                0x01 => ct.charset = Some(charset_value(&mut s)?),
                0x03 | 0x09 => ct.related_type = Some(constrained_media(&mut s)?),
                0x05 | 0x17 => ct.name = Some(s.text_value()?),
                0x06 | 0x18 => ct.filename = Some(s.text_value()?),
                0x0A | 0x19 => ct.start = Some(s.text_value()?),
                _ => skip_value(&mut s)?,
            }
        } else {
            let name = s.token()?.to_ascii_lowercase();
            let v = s.peek()?;
            let int = v != 0 && (v >= 0x80 || v <= 30);
            match name.as_str() {
                "charset" => ct.charset = Some(charset_value(&mut s)?),
                "type" => ct.related_type = Some(constrained_media(&mut s)?),
                "start" if !int => ct.start = Some(s.text_value()?),
                "name" if !int => ct.name = Some(s.text_value()?),
                "filename" if !int => ct.filename = Some(s.text_value()?),
                _ => {
                    if int {
                        s.integer()?;
                    } else {
                        s.text_value()?;
                    }
                }
            }
        }
    }
    Ok(ct)
}

// ---------------------------------------------------------------------------
// Multipart

/// One part of a multipart body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// Media type, e.g. `"text/plain"`.
    pub content_type: String,
    /// Charset MIBenum for text parts (106 = UTF-8).
    pub charset: Option<u32>,
    /// The `name` content-type parameter (the attachment's file name).
    pub name: Option<String>,
    /// Content-ID without the angle brackets.
    pub content_id: Option<String>,
    pub content_location: Option<String>,
    pub data: Vec<u8>,
}

impl Part {
    pub fn new(content_type: impl Into<String>, data: Vec<u8>) -> Part {
        Part { content_type: content_type.into(), charset: None, name: None, content_id: None, content_location: None, data }
    }
    /// text/plain;charset=utf-8.
    pub fn text(s: &str) -> Part {
        Part { charset: Some(CHARSET_UTF8), ..Part::new("text/plain", s.as_bytes().to_vec()) }
    }
    /// A named attachment: sets name, Content-Location and Content-ID to `name`.
    pub fn attachment(content_type: &str, name: &str, data: Vec<u8>) -> Part {
        Part {
            name: Some(name.to_string()),
            content_id: Some(name.to_string()),
            content_location: Some(name.to_string()),
            ..Part::new(content_type, data)
        }
    }
    pub fn png(name: &str, data: Vec<u8>) -> Part {
        Part::attachment("image/png", name, data)
    }
    pub fn wav(name: &str, data: Vec<u8>) -> Part {
        Part::attachment("audio/wav", name, data)
    }
    pub fn vcard(name: &str, data: Vec<u8>) -> Part {
        Part::attachment("text/x-vcard", name, data)
    }
    pub fn is(&self, media: &str) -> bool {
        self.content_type.eq_ignore_ascii_case(media)
    }
    /// The part's data as text when it is text/plain.
    pub fn as_text(&self) -> Option<String> {
        self.is("text/plain").then(|| decode_charset(self.charset.unwrap_or(CHARSET_UTF8), &self.data))
    }
    fn content_type_value(&self) -> ContentType {
        ContentType { charset: self.charset, name: self.name.clone(), ..ContentType::new(self.content_type.clone()) }
    }
}

/// A PDU body: the top-level Content-Type and its parts. For a multipart type
/// the parts are the multipart entries; for any other type there is one part
/// holding the raw body (only `parts[0].data` is encoded).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub content_type: ContentType,
    pub parts: Vec<Part>,
}

impl Body {
    /// application/vnd.wap.multipart.mixed of `parts`.
    pub fn mixed(parts: Vec<Part>) -> Body {
        Body { content_type: ContentType::new(MULTIPART_MIXED), parts }
    }

    /// A message of optional text plus attachments. Parts without a
    /// Content-Location/Content-ID get one. With `smil`, a minimal SMIL
    /// presentation goes first and the body is multipart.related with
    /// `type=application/smil; start=<smil>`; otherwise multipart.mixed.
    pub fn message(text: Option<&str>, attachments: Vec<Part>, smil: bool) -> Body {
        let mut parts = Vec::new();
        if let Some(t) = text {
            let mut p = Part::text(t);
            p.content_location = Some("text_0.txt".into());
            p.content_id = Some("text_0".into());
            parts.push(p);
        }
        for (i, mut p) in attachments.into_iter().enumerate() {
            if p.content_location.is_none() {
                p.content_location = Some(p.name.clone().unwrap_or_else(|| format!("part_{}", i + 1)));
            }
            if p.content_id.is_none() {
                p.content_id = p.content_location.clone();
            }
            parts.push(p);
        }
        if !smil {
            return Body::mixed(parts);
        }
        let mut sp = Part::new(SMIL, smil_for(&parts).into_bytes());
        sp.charset = Some(CHARSET_UTF8);
        sp.content_id = Some("smil".into());
        sp.content_location = Some("smil.xml".into());
        parts.insert(0, sp);
        Body {
            content_type: ContentType {
                related_type: Some(SMIL.into()),
                start: Some("<smil>".into()),
                ..ContentType::new(MULTIPART_RELATED)
            },
            parts,
        }
    }

    /// All text/plain parts' text, joined with newlines.
    pub fn text(&self) -> Option<String> {
        let texts: Vec<String> = self.parts.iter().filter_map(Part::as_text).collect();
        (!texts.is_empty()).then(|| texts.join("\n"))
    }

    /// Every part that is neither text/plain nor SMIL.
    pub fn attachments(&self) -> Vec<&Part> {
        self.parts.iter().filter(|p| !p.is("text/plain") && !p.is(SMIL)).collect()
    }

    pub fn smil(&self) -> Option<&Part> {
        self.parts.iter().find(|p| p.is(SMIL))
    }

    /// Content-Type value then the body octets.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Vec::new();
        self.write(&mut w);
        w
    }

    fn write(&self, w: &mut Vec<u8>) {
        self.content_type.write(w);
        if self.content_type.is_multipart() {
            w.extend_from_slice(&encode_multipart(&self.parts));
        } else if let Some(p) = self.parts.first() {
            w.extend_from_slice(&p.data);
        }
    }

    /// Decode Content-Type value + body octets (the inverse of [`Body::encode`]).
    pub fn decode(b: &[u8]) -> MmsResult<Body> {
        let mut r = R::new(b);
        let ct = content_type(&mut r)?;
        Body::from_parts(ct, r.rest())
    }

    fn from_parts(ct: ContentType, b: &[u8]) -> MmsResult<Body> {
        let parts = if ct.is_multipart() {
            decode_multipart(b)?
        } else {
            vec![Part {
                content_type: ct.media.clone(),
                charset: ct.charset,
                name: ct.name.clone().or_else(|| ct.filename.clone()),
                content_id: None,
                content_location: None,
                data: b.to_vec(),
            }]
        };
        Ok(Body { content_type: ct, parts })
    }
}

/// WSP multipart octets (count, then entries).
pub fn encode_multipart(parts: &[Part]) -> Vec<u8> {
    let mut w = Vec::new();
    put_uintvar(&mut w, parts.len() as u32);
    for p in parts {
        let mut h = Vec::new();
        p.content_type_value().write(&mut h);
        if let Some(id) = &p.content_id {
            h.push(WSP_CONTENT_ID);
            put_quoted(&mut h, &format!("<{id}>"));
        }
        if let Some(loc) = &p.content_location {
            h.push(WSP_CONTENT_LOCATION);
            put_text(&mut h, loc);
        }
        put_uintvar(&mut w, h.len() as u32);
        put_uintvar(&mut w, p.data.len() as u32);
        w.extend_from_slice(&h);
        w.extend_from_slice(&p.data);
    }
    w
}

fn strip_angle(s: String) -> String {
    match s.strip_prefix('<').and_then(|x| x.strip_suffix('>')) {
        Some(inner) => inner.to_string(),
        None => s,
    }
}

/// Decode WSP multipart octets (exactly; trailing octets are an error).
pub fn decode_multipart(b: &[u8]) -> MmsResult<Vec<Part>> {
    let mut r = R::new(b);
    let n = r.uintvar()?;
    let mut parts = Vec::new();
    for _ in 0..n {
        let hl = r.uintvar()? as usize;
        let dl = r.uintvar()? as usize;
        let mut h = R::new(r.take(hl)?);
        let data = r.take(dl)?;
        let ct = content_type(&mut h)?;
        let mut part = Part {
            content_type: ct.media,
            charset: ct.charset,
            name: ct.name.or(ct.filename),
            content_id: None,
            content_location: None,
            data: data.to_vec(),
        };
        while !h.done() {
            let o = h.peek()?;
            if o >= 0x80 {
                h.u8()?;
                match o {
                    WSP_CONTENT_ID => part.content_id = Some(strip_angle(h.text()?)),
                    WSP_CONTENT_LOCATION => part.content_location = Some(h.text()?),
                    _ => skip_value(&mut h)?,
                }
            } else if o >= 32 {
                let name = h.token()?.to_ascii_lowercase();
                let v = h.text()?;
                match name.as_str() {
                    "content-id" => part.content_id = Some(strip_angle(v)),
                    "content-location" => part.content_location = Some(v),
                    _ => {}
                }
            } else {
                return Err(MmsError::Value("part header"));
            }
        }
        parts.push(part);
    }
    r.end()?;
    Ok(parts)
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

/// A minimal SMIL presentation showing every part on one slide.
pub fn smil_for(parts: &[Part]) -> String {
    let mut s = String::from(
        "<smil><head><layout><root-layout/>\
         <region id=\"Image\" top=\"0\" left=\"0\" height=\"80%\" width=\"100%\" fit=\"meet\"/>\
         <region id=\"Text\" top=\"80%\" left=\"0\" height=\"20%\" width=\"100%\" fit=\"scroll\"/>\
         </layout></head><body><par dur=\"5000ms\">",
    );
    for p in parts {
        let Some(src) = p.content_location.as_deref() else { continue };
        let src = xml_escape(src);
        let ct = p.content_type.to_ascii_lowercase();
        let tag = if ct == "text/plain" {
            format!("<text src=\"{src}\" region=\"Text\"/>")
        } else if ct.starts_with("image/") {
            format!("<img src=\"{src}\" region=\"Image\"/>")
        } else if ct.starts_with("audio/") {
            format!("<audio src=\"{src}\"/>")
        } else if ct.starts_with("video/") {
            format!("<video src=\"{src}\" region=\"Image\"/>")
        } else {
            format!("<ref src=\"{src}\"/>")
        };
        s.push_str(&tag);
    }
    s.push_str("</par></body></smil>");
    s
}

// ---------------------------------------------------------------------------
// Header value types

/// X-Mms-Message-Type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MessageType {
    SendReq = 0x80,
    SendConf = 0x81,
    NotificationInd = 0x82,
    NotifyRespInd = 0x83,
    RetrieveConf = 0x84,
    AcknowledgeInd = 0x85,
    DeliveryInd = 0x86,
    ReadRecInd = 0x87,
    ReadOrigInd = 0x88,
}

impl MessageType {
    pub fn from_u8(o: u8) -> MmsResult<MessageType> {
        use MessageType::*;
        Ok(match o {
            0x80 => SendReq,
            0x81 => SendConf,
            0x82 => NotificationInd,
            0x83 => NotifyRespInd,
            0x84 => RetrieveConf,
            0x85 => AcknowledgeInd,
            0x86 => DeliveryInd,
            0x87 => ReadRecInd,
            0x88 => ReadOrigInd,
            _ => return Err(MmsError::MessageType(o)),
        })
    }
    /// Whether this PDU type carries Content-Type and a body.
    pub fn has_body(self) -> bool {
        matches!(self, MessageType::SendReq | MessageType::RetrieveConf)
    }
    pub fn name(self) -> &'static str {
        use MessageType::*;
        match self {
            SendReq => "m-send-req",
            SendConf => "m-send-conf",
            NotificationInd => "m-notification-ind",
            NotifyRespInd => "m-notifyresp-ind",
            RetrieveConf => "m-retrieve-conf",
            AcknowledgeInd => "m-acknowledge-ind",
            DeliveryInd => "m-delivery-ind",
            ReadRecInd => "m-read-rec-ind",
            ReadOrigInd => "m-read-orig-ind",
        }
    }
}

/// From: an address, or the Insert-address token (the MMSC fills it in).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sender {
    Address(String),
    InsertAddress,
}

/// X-Mms-Message-Class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageClass {
    Personal,
    Advertisement,
    Informational,
    Auto,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Low,
    Normal,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SenderVisibility {
    Hide,
    Show,
}

/// X-Mms-Expiry / X-Mms-Delivery-Time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSpec {
    /// Seconds since the Unix epoch.
    Absolute(u64),
    /// Seconds from now.
    Relative(u64),
}

/// X-Mms-Response-Status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseStatus {
    Ok,
    ErrorUnspecified,
    ErrorServiceDenied,
    ErrorMessageFormatCorrupt,
    ErrorSendingAddressUnresolved,
    ErrorMessageNotFound,
    ErrorNetworkProblem,
    ErrorContentNotAccepted,
    /// Any other value (e.g. MMS 1.2 transient 0xC0.. / permanent 0xE0..).
    Other(u8),
}

impl ResponseStatus {
    pub fn to_u8(self) -> u8 {
        use ResponseStatus::*;
        match self {
            Ok => 0x80,
            ErrorUnspecified => 0x81,
            ErrorServiceDenied => 0x82,
            ErrorMessageFormatCorrupt => 0x83,
            ErrorSendingAddressUnresolved => 0x84,
            ErrorMessageNotFound => 0x85,
            ErrorNetworkProblem => 0x86,
            ErrorContentNotAccepted => 0x87,
            Other(o) => o | 0x80,
        }
    }
    pub fn from_u8(o: u8) -> MmsResult<ResponseStatus> {
        use ResponseStatus::*;
        Result::Ok(match o {
            0x80 => Ok,
            0x81 => ErrorUnspecified,
            0x82 => ErrorServiceDenied,
            0x83 => ErrorMessageFormatCorrupt,
            0x84 => ErrorSendingAddressUnresolved,
            0x85 => ErrorMessageNotFound,
            0x86 => ErrorNetworkProblem,
            0x87 => ErrorContentNotAccepted,
            0x88..=0xFF => Other(o),
            _ => return Err(MmsError::Value("response-status")),
        })
    }
}

/// X-Mms-Status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Expired,
    Retrieved,
    Rejected,
    Deferred,
    Unrecognised,
    Indeterminate,
    Forwarded,
    Unreachable,
}

impl Status {
    pub fn to_u8(self) -> u8 {
        0x80 + self as u8
    }
    pub fn from_u8(o: u8) -> MmsResult<Status> {
        use Status::*;
        Ok(match o {
            0x80 => Expired,
            0x81 => Retrieved,
            0x82 => Rejected,
            0x83 => Deferred,
            0x84 => Unrecognised,
            0x85 => Indeterminate,
            0x86 => Forwarded,
            0x87 => Unreachable,
            _ => return Err(MmsError::Value("status")),
        })
    }
}

/// X-Mms-Read-Status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadStatus {
    Read,
    DeletedWithoutBeingRead,
}

/// X-Mms-Retrieve-Status Ok (others: 0xC0.. transient, 0xE0.. permanent).
pub const RETRIEVE_STATUS_OK: u8 = 0x80;

// ---------------------------------------------------------------------------
// PDU

/// One MMS PDU: its type and every header this module knows, all optional
/// except the type and version. [`Pdu::decode`] enforces each type's
/// mandatory headers; the constructors set them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pdu {
    pub message_type: MessageType,
    pub transaction_id: Option<String>,
    /// X-Mms-MMS-Version, e.g. [`VERSION_1_2`].
    pub version: u8,
    pub date: Option<u64>,
    pub from: Option<Sender>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: Option<String>,
    pub message_class: Option<MessageClass>,
    pub message_id: Option<String>,
    pub message_size: Option<u64>,
    pub expiry: Option<TimeSpec>,
    pub delivery_time: Option<TimeSpec>,
    pub content_location: Option<String>,
    pub priority: Option<Priority>,
    pub delivery_report: Option<bool>,
    pub read_report: Option<bool>,
    pub report_allowed: Option<bool>,
    pub sender_visibility: Option<SenderVisibility>,
    pub response_status: Option<ResponseStatus>,
    pub response_text: Option<String>,
    pub status: Option<Status>,
    pub retrieve_status: Option<u8>,
    pub retrieve_text: Option<String>,
    pub read_status: Option<ReadStatus>,
    /// Only encoded for types with a body (send-req, retrieve-conf).
    pub body: Option<Body>,
}

impl Pdu {
    /// A PDU of `message_type`, version 1.2, no other headers.
    pub fn new(message_type: MessageType) -> Pdu {
        Pdu {
            message_type,
            transaction_id: None,
            version: VERSION_1_2,
            date: None,
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: None,
            message_class: None,
            message_id: None,
            message_size: None,
            expiry: None,
            delivery_time: None,
            content_location: None,
            priority: None,
            delivery_report: None,
            read_report: None,
            report_allowed: None,
            sender_visibility: None,
            response_status: None,
            response_text: None,
            status: None,
            retrieve_status: None,
            retrieve_text: None,
            read_status: None,
            body: None,
        }
    }

    pub fn send_req(tid: &str, from: Sender, to: Vec<String>, subject: Option<&str>, body: Body) -> Pdu {
        Pdu {
            transaction_id: Some(tid.into()),
            from: Some(from),
            to,
            subject: subject.map(Into::into),
            body: Some(body),
            ..Pdu::new(MessageType::SendReq)
        }
    }

    pub fn send_conf(tid: &str, status: ResponseStatus, message_id: Option<&str>) -> Pdu {
        Pdu {
            transaction_id: Some(tid.into()),
            response_status: Some(status),
            message_id: message_id.map(Into::into),
            ..Pdu::new(MessageType::SendConf)
        }
    }

    /// Class Personal.
    pub fn notification_ind(
        tid: &str,
        from: Option<&str>,
        subject: Option<&str>,
        size: u64,
        expiry: TimeSpec,
        content_location: &str,
    ) -> Pdu {
        Pdu {
            transaction_id: Some(tid.into()),
            from: from.map(|f| Sender::Address(f.into())),
            subject: subject.map(Into::into),
            message_class: Some(MessageClass::Personal),
            message_size: Some(size),
            expiry: Some(expiry),
            content_location: Some(content_location.into()),
            ..Pdu::new(MessageType::NotificationInd)
        }
    }

    pub fn notifyresp_ind(tid: &str, status: Status) -> Pdu {
        Pdu { transaction_id: Some(tid.into()), status: Some(status), ..Pdu::new(MessageType::NotifyRespInd) }
    }

    pub fn retrieve_conf(
        tid: Option<&str>,
        message_id: Option<&str>,
        date: u64,
        from: Option<&str>,
        to: Vec<String>,
        subject: Option<&str>,
        body: Body,
    ) -> Pdu {
        Pdu {
            transaction_id: tid.map(Into::into),
            message_id: message_id.map(Into::into),
            date: Some(date),
            from: from.map(|f| Sender::Address(f.into())),
            to,
            subject: subject.map(Into::into),
            body: Some(body),
            ..Pdu::new(MessageType::RetrieveConf)
        }
    }

    pub fn acknowledge_ind(tid: &str) -> Pdu {
        Pdu { transaction_id: Some(tid.into()), ..Pdu::new(MessageType::AcknowledgeInd) }
    }

    pub fn delivery_ind(message_id: &str, to: &str, date: u64, status: Status) -> Pdu {
        Pdu {
            message_id: Some(message_id.into()),
            to: vec![to.into()],
            date: Some(date),
            status: Some(status),
            ..Pdu::new(MessageType::DeliveryInd)
        }
    }

    pub fn read_rec_ind(message_id: &str, to: &str, from: &str, date: Option<u64>, read: ReadStatus) -> Pdu {
        Pdu {
            message_id: Some(message_id.into()),
            to: vec![to.into()],
            from: Some(Sender::Address(from.into())),
            date,
            read_status: Some(read),
            ..Pdu::new(MessageType::ReadRecInd)
        }
    }

    pub fn read_orig_ind(message_id: &str, to: &str, from: &str, date: u64, read: ReadStatus) -> Pdu {
        Pdu { date: Some(date), message_type: MessageType::ReadOrigInd, ..Pdu::read_rec_ind(message_id, to, from, None, read) }
    }

    /// The PDU octets. Message-Type, Transaction-Id, MMS-Version first, each
    /// type's key mandatory header late, Content-Type and the body last.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = vec![H_MESSAGE_TYPE, self.message_type as u8];
        if let Some(t) = &self.transaction_id {
            w.push(H_TRANSACTION_ID);
            put_text(&mut w, t);
        }
        w.push(H_MMS_VERSION);
        w.push(0x80 | (self.version & 0x7F));
        if let Some(d) = self.date {
            w.push(H_DATE);
            put_long(&mut w, d);
        }
        if let Some(f) = &self.from {
            w.push(H_FROM);
            let mut inner = Vec::new();
            match f {
                Sender::Address(a) => {
                    inner.push(0x80);
                    put_encoded_string(&mut inner, a);
                }
                Sender::InsertAddress => inner.push(0x81),
            }
            with_length(&mut w, &inner);
        }
        for (h, list) in [(H_TO, &self.to), (H_CC, &self.cc), (H_BCC, &self.bcc)] {
            for a in list {
                w.push(h);
                put_encoded_string(&mut w, a);
            }
        }
        if let Some(s) = &self.subject {
            w.push(H_SUBJECT);
            put_encoded_string(&mut w, s);
        }
        if let Some(c) = &self.message_class {
            w.push(H_MESSAGE_CLASS);
            match c {
                MessageClass::Personal => w.push(0x80),
                MessageClass::Advertisement => w.push(0x81),
                MessageClass::Informational => w.push(0x82),
                MessageClass::Auto => w.push(0x83),
                MessageClass::Other(t) => put_text(&mut w, t),
            }
        }
        if let Some(p) = self.priority {
            w.push(H_PRIORITY);
            w.push(0x80 + p as u8);
        }
        if let Some(v) = self.sender_visibility {
            w.push(H_SENDER_VISIBILITY);
            w.push(0x80 + v as u8);
        }
        for (h, v) in [
            (H_DELIVERY_REPORT, self.delivery_report),
            (H_READ_REPORT, self.read_report),
            (H_REPORT_ALLOWED, self.report_allowed),
        ] {
            if let Some(v) = v {
                w.push(h);
                w.push(if v { 0x80 } else { 0x81 });
            }
        }
        let put_time = |w: &mut Vec<u8>, h: u8, t: TimeSpec| {
            w.push(h);
            let mut inner = Vec::new();
            match t {
                TimeSpec::Absolute(s) => {
                    inner.push(0x80);
                    put_long(&mut inner, s);
                }
                TimeSpec::Relative(s) => {
                    inner.push(0x81);
                    put_long(&mut inner, s);
                }
            }
            with_length(w, &inner);
        };
        if let Some(t) = self.delivery_time {
            put_time(&mut w, H_DELIVERY_TIME, t);
        }
        if let Some(t) = &self.response_text {
            w.push(H_RESPONSE_TEXT);
            put_encoded_string(&mut w, t);
        }
        if let Some(s) = self.retrieve_status {
            w.push(H_RETRIEVE_STATUS);
            w.push(s | 0x80);
        }
        if let Some(t) = &self.retrieve_text {
            w.push(H_RETRIEVE_TEXT);
            put_encoded_string(&mut w, t);
        }
        if let Some(id) = &self.message_id {
            w.push(H_MESSAGE_ID);
            put_text(&mut w, id);
        }
        if let Some(s) = self.message_size {
            w.push(H_MESSAGE_SIZE);
            put_long(&mut w, s);
        }
        if let Some(t) = self.expiry {
            put_time(&mut w, H_EXPIRY, t);
        }
        if let Some(l) = &self.content_location {
            w.push(H_CONTENT_LOCATION);
            put_text(&mut w, l);
        }
        if let Some(s) = self.response_status {
            w.push(H_RESPONSE_STATUS);
            w.push(s.to_u8());
        }
        if let Some(s) = self.status {
            w.push(H_STATUS);
            w.push(s.to_u8());
        }
        if let Some(s) = self.read_status {
            w.push(H_READ_STATUS);
            w.push(0x80 + s as u8);
        }
        if self.message_type.has_body() {
            if let Some(b) = &self.body {
                w.push(H_CONTENT_TYPE);
                b.write(&mut w);
            }
        }
        w
    }

    pub fn decode(b: &[u8]) -> MmsResult<Pdu> {
        let mut r = R::new(b);
        if r.u8()? != H_MESSAGE_TYPE {
            return Err(MmsError::Value("first header must be X-Mms-Message-Type"));
        }
        let mt = MessageType::from_u8(r.u8()?)?;
        let mut p = Pdu::new(mt);
        if r.peek()? == H_TRANSACTION_ID {
            r.u8()?;
            p.transaction_id = Some(r.text()?);
        }
        if r.u8()? != H_MMS_VERSION {
            return Err(MmsError::Value("X-Mms-MMS-Version must follow Message-Type/Transaction-Id"));
        }
        p.version = r.short_int()?;
        // Bit per header number seen; the three fixed headers can't repeat.
        let mut seen: u128 = (1 << (H_MESSAGE_TYPE & 0x7F)) | (1 << (H_MMS_VERSION & 0x7F)) | (1 << (H_TRANSACTION_ID & 0x7F));
        while !r.done() {
            let o = r.peek()?;
            if o < 0x80 {
                if o < 32 {
                    return Err(MmsError::Value("header"));
                }
                // Application-header: Token-text name, Text-string value.
                r.token()?;
                r.text()?;
                continue;
            }
            r.u8()?;
            if !matches!(o, H_TO | H_CC | H_BCC) {
                let bit = 1u128 << (o & 0x7F);
                if seen & bit != 0 {
                    return Err(MmsError::Duplicate(o));
                }
                seen |= bit;
            }
            match o {
                H_BCC => p.bcc.push(r.encoded_string()?),
                H_CC => p.cc.push(r.encoded_string()?),
                H_TO => p.to.push(r.encoded_string()?),
                H_CONTENT_LOCATION => p.content_location = Some(r.text()?),
                H_CONTENT_TYPE => {
                    if !mt.has_body() {
                        return Err(MmsError::Value("Content-Type in a PDU without a body"));
                    }
                    let ct = content_type(&mut r)?;
                    p.body = Some(Body::from_parts(ct, r.rest())?);
                    r.p = r.b.len();
                }
                H_DATE => p.date = Some(r.long_int()?),
                H_DELIVERY_REPORT => p.delivery_report = Some(r.yes_no()?),
                H_DELIVERY_TIME => p.delivery_time = Some(time_spec(&mut r)?),
                H_EXPIRY => p.expiry = Some(time_spec(&mut r)?),
                H_FROM => {
                    let mut s = r.sub()?;
                    p.from = Some(match s.u8()? {
                        0x80 => Sender::Address(s.encoded_string()?),
                        0x81 => Sender::InsertAddress,
                        _ => return Err(MmsError::Value("from token")),
                    });
                    s.end()?;
                }
                H_MESSAGE_CLASS => {
                    p.message_class = Some(if r.peek()? >= 0x80 {
                        match r.u8()? {
                            0x80 => MessageClass::Personal,
                            0x81 => MessageClass::Advertisement,
                            0x82 => MessageClass::Informational,
                            0x83 => MessageClass::Auto,
                            _ => return Err(MmsError::Value("message-class")),
                        }
                    } else {
                        MessageClass::Other(r.text()?)
                    })
                }
                H_MESSAGE_ID => p.message_id = Some(r.text()?),
                H_MESSAGE_SIZE => p.message_size = Some(r.long_int()?),
                H_PRIORITY => {
                    p.priority = Some(match r.u8()? {
                        0x80 => Priority::Low,
                        0x81 => Priority::Normal,
                        0x82 => Priority::High,
                        _ => return Err(MmsError::Value("priority")),
                    })
                }
                H_READ_REPORT => p.read_report = Some(r.yes_no()?),
                H_REPORT_ALLOWED => p.report_allowed = Some(r.yes_no()?),
                H_RESPONSE_STATUS => p.response_status = Some(ResponseStatus::from_u8(r.u8()?)?),
                H_RESPONSE_TEXT => p.response_text = Some(r.encoded_string()?),
                H_SENDER_VISIBILITY => {
                    p.sender_visibility = Some(match r.u8()? {
                        0x80 => SenderVisibility::Hide,
                        0x81 => SenderVisibility::Show,
                        _ => return Err(MmsError::Value("sender-visibility")),
                    })
                }
                H_STATUS => p.status = Some(Status::from_u8(r.u8()?)?),
                H_SUBJECT => p.subject = Some(r.encoded_string()?),
                H_RETRIEVE_STATUS => {
                    let s = r.u8()?;
                    if s < 0x80 {
                        return Err(MmsError::Value("retrieve-status"));
                    }
                    p.retrieve_status = Some(s);
                }
                H_RETRIEVE_TEXT => p.retrieve_text = Some(r.encoded_string()?),
                H_READ_STATUS => {
                    p.read_status = Some(match r.u8()? {
                        0x80 => ReadStatus::Read,
                        0x81 => ReadStatus::DeletedWithoutBeingRead,
                        _ => return Err(MmsError::Value("read-status")),
                    })
                }
                _ => skip_value(&mut r)?,
            }
        }
        p.check()?;
        Ok(p)
    }

    /// The mandatory headers of this PDU's type are present.
    pub fn check(&self) -> MmsResult<()> {
        use MessageType::*;
        let need = |ok: bool, what: &'static str| if ok { Ok(()) } else { Err(MmsError::Missing(what)) };
        if matches!(self.message_type, SendReq | SendConf | NotificationInd | NotifyRespInd | AcknowledgeInd) {
            need(self.transaction_id.is_some(), "X-Mms-Transaction-Id")?;
        }
        match self.message_type {
            SendReq => {
                need(self.from.is_some(), "From")?;
                need(self.body.is_some(), "Content-Type")?;
            }
            SendConf => need(self.response_status.is_some(), "X-Mms-Response-Status")?,
            NotificationInd => {
                need(self.message_class.is_some(), "X-Mms-Message-Class")?;
                need(self.message_size.is_some(), "X-Mms-Message-Size")?;
                need(self.expiry.is_some(), "X-Mms-Expiry")?;
                need(self.content_location.is_some(), "X-Mms-Content-Location")?;
            }
            NotifyRespInd => need(self.status.is_some(), "X-Mms-Status")?,
            RetrieveConf => {
                need(self.date.is_some(), "Date")?;
                need(self.body.is_some(), "Content-Type")?;
            }
            AcknowledgeInd => {}
            DeliveryInd => {
                need(self.message_id.is_some(), "Message-ID")?;
                need(!self.to.is_empty(), "To")?;
                need(self.date.is_some(), "Date")?;
                need(self.status.is_some(), "X-Mms-Status")?;
            }
            ReadRecInd | ReadOrigInd => {
                need(self.message_id.is_some(), "Message-ID")?;
                need(!self.to.is_empty(), "To")?;
                need(self.from.is_some(), "From")?;
                if self.message_type == ReadOrigInd {
                    need(self.date.is_some(), "Date")?;
                }
                need(self.read_status.is_some(), "X-Mms-Read-Status")?;
            }
        }
        Ok(())
    }

    /// The From address, when present (not Insert-address).
    pub fn from_address(&self) -> Option<&str> {
        match &self.from {
            Some(Sender::Address(a)) => Some(a),
            _ => None,
        }
    }
}

fn time_spec(r: &mut R) -> MmsResult<TimeSpec> {
    let mut s = r.sub()?;
    let t = match s.u8()? {
        0x80 => TimeSpec::Absolute(s.long_int()?),
        0x81 => TimeSpec::Relative(s.integer()?),
        _ => return Err(MmsError::Value("time token")),
    };
    s.end()?;
    Ok(t)
}

// ---------------------------------------------------------------------------
// Addresses

/// `"+12145550142"` -> `"+12145550142/TYPE=PLMN"`.
pub fn plmn_address(number: &str) -> String {
    format!("{number}/TYPE=PLMN")
}

/// Split `"+12145550142/TYPE=PLMN"` into (`"+12145550142"`, `Some("PLMN")`);
/// an address without `/TYPE=` (e.g. an e-mail address) has no type.
pub fn parse_address(addr: &str) -> (&str, Option<&str>) {
    let lower = addr.to_ascii_lowercase();
    match lower.rfind("/type=") {
        Some(i) => (&addr[..i], Some(&addr[i + 6..])),
        None => (addr, None),
    }
}

// ---------------------------------------------------------------------------
// WAP Push

/// The WSP Push PDU carrying an MMS PDU (normally an m-notification-ind):
/// `tid 06 03 BE AF 84 <pdu>`.
pub fn wap_push_encode(notification_pdu: &[u8], tid: u8) -> Vec<u8> {
    let headers = [0x80 | 0x3E, WSP_APP_ID, WSP_APP_MMS_UA];
    let mut w = vec![tid, WSP_PUSH];
    put_uintvar(&mut w, headers.len() as u32);
    w.extend_from_slice(&headers);
    w.extend_from_slice(notification_pdu);
    w
}

/// Decode a WSP (Confirmed)Push PDU: (content-type media, body).
pub fn wap_push_decode(b: &[u8]) -> MmsResult<(String, Vec<u8>)> {
    let mut r = R::new(b);
    let _tid = r.u8()?;
    let ty = r.u8()?;
    if ty != WSP_PUSH && ty != WSP_CONFIRMED_PUSH {
        return Err(MmsError::Value("WSP PDU type is not Push"));
    }
    let hl = r.uintvar()? as usize;
    let mut h = R::new(r.take(hl)?);
    let ct = content_type(&mut h)?;
    while !h.done() {
        let o = h.peek()?;
        if o >= 0x80 {
            h.u8()?;
            skip_value(&mut h)?;
        } else if o >= 32 {
            h.token()?;
            h.text()?;
        } else {
            return Err(MmsError::Value("push header"));
        }
    }
    Ok((ct.media, r.rest().to_vec()))
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn png_blob() -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend((0..300u32).map(|i| (i * 7 % 256) as u8));
        v
    }
    fn wav_blob() -> Vec<u8> {
        let mut v = b"RIFF\x24\x08\x00\x00WAVEfmt ".to_vec();
        v.extend((0..200u32).map(|i| (i * 13 % 256) as u8));
        v.push(0);
        v
    }
    fn body() -> Body {
        Body::message(
            Some("Hello from the mine \u{1F48E}"),
            vec![Part::png("ore.png", png_blob()), Part::wav("ping.wav", wav_blob())],
            true,
        )
    }
    fn to() -> Vec<String> {
        vec![plmn_address("+12145550142")]
    }

    fn corpus() -> Vec<Pdu> {
        let mut send = Pdu::send_req("tx-1", Sender::InsertAddress, to(), Some("Ore report"), body());
        send.cc = vec![plmn_address("+12145550143")];
        send.bcc = vec!["steve@mine.example".into()];
        send.date = Some(1_700_000_000);
        send.message_class = Some(MessageClass::Personal);
        send.expiry = Some(TimeSpec::Relative(604_800));
        send.delivery_time = Some(TimeSpec::Absolute(1_700_000_100));
        send.priority = Some(Priority::High);
        send.sender_visibility = Some(SenderVisibility::Show);
        send.delivery_report = Some(true);
        send.read_report = Some(false);
        let mut conf = Pdu::send_conf("tx-1", ResponseStatus::Ok, Some("msg-0001"));
        conf.response_text = Some("Accepted".into());
        let mut notif = Pdu::notification_ind(
            "tx-2",
            Some(&plmn_address("+12145550199")),
            Some("Caf\u{e9}"),
            2048,
            TimeSpec::Relative(259_200),
            "http://mmsc.example/m/0001",
        );
        notif.delivery_report = Some(false);
        notif.priority = Some(Priority::Normal);
        let mut nresp = Pdu::notifyresp_ind("tx-2", Status::Retrieved);
        nresp.report_allowed = Some(true);
        let mut retr = Pdu::retrieve_conf(
            Some("tx-2"),
            Some("msg-0001"),
            1_700_000_000,
            Some(&plmn_address("+12145550199")),
            to(),
            Some("Ore report"),
            body(),
        );
        retr.message_class = Some(MessageClass::Other("x-ecm".into()));
        retr.retrieve_status = Some(RETRIEVE_STATUS_OK);
        retr.retrieve_text = Some("ok".into());
        let mut ack = Pdu::acknowledge_ind("tx-2");
        ack.report_allowed = Some(false);
        let mut v13 = Pdu::acknowledge_ind("tx-3");
        v13.version = VERSION_1_3;
        vec![
            send,
            conf,
            notif,
            nresp,
            retr,
            ack,
            v13,
            Pdu::delivery_ind("msg-0001", &plmn_address("+12145550142"), 1_700_000_500, Status::Retrieved),
            Pdu::read_rec_ind("msg-0001", &to()[0], &plmn_address("+12145550142"), Some(1), ReadStatus::Read),
            Pdu::read_rec_ind("msg-0001", &to()[0], &plmn_address("+1"), None, ReadStatus::DeletedWithoutBeingRead),
            Pdu::read_orig_ind("msg-0001", &to()[0], &plmn_address("+12145550142"), 1_700_000_900, ReadStatus::Read),
            Pdu::send_conf("tx-9", ResponseStatus::Other(0xE3), None),
        ]
    }

    #[test]
    fn uintvar_edge_cases() {
        let cases: &[(u32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7F]),
            (128, &[0x81, 0x00]),
            (16_383, &[0xFF, 0x7F]),
            (16_384, &[0x81, 0x80, 0x00]),
            (0x0FFF_FFFF, &[0xFF, 0xFF, 0xFF, 0x7F]),
            (0x1000_0000, &[0x81, 0x80, 0x80, 0x80, 0x00]),
            (u32::MAX, &[0x8F, 0xFF, 0xFF, 0xFF, 0x7F]),
        ];
        for &(v, bytes) in cases {
            let mut w = Vec::new();
            put_uintvar(&mut w, v);
            assert_eq!(w, bytes, "{v}");
            assert_eq!(get_uintvar(bytes), Ok((v, bytes.len())));
            for n in 0..bytes.len() {
                assert_eq!(get_uintvar(&bytes[..n]), Err(MmsError::Short));
            }
        }
        // Over 32 bits, more than 5 octets.
        assert!(get_uintvar(&[0x90, 0x80, 0x80, 0x80, 0x00]).is_err());
        assert!(get_uintvar(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]).is_err());
        // Non-minimal but within 5 octets is fine.
        assert_eq!(get_uintvar(&[0x80, 0x05]), Ok((5, 2)));
        assert_eq!(get_uintvar(&[0x05, 0xFF]), Ok((5, 1)));
    }

    #[test]
    fn integers_and_lengths() {
        let mut w = Vec::new();
        put_long(&mut w, 0);
        put_long(&mut w, 0x6553_F100);
        put_long(&mut w, u64::MAX);
        assert_eq!(w, [1, 0, 4, 0x65, 0x53, 0xF1, 0x00, 8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let mut r = R::new(&w);
        assert_eq!(r.long_int(), Ok(0));
        assert_eq!(r.long_int(), Ok(0x6553_F100));
        assert_eq!(r.long_int(), Ok(u64::MAX));
        assert!(R::new(&[0]).long_int().is_err());
        assert!(R::new(&[31]).long_int().is_err());
        assert!(R::new(&[9, 1, 0, 0, 0, 0, 0, 0, 0, 0]).long_int().is_err());
        assert_eq!(R::new(&[9, 0, 0, 0, 0, 0, 0, 0, 0, 5]).long_int(), Ok(5));

        let mut w = Vec::new();
        put_integer(&mut w, 106);
        put_integer(&mut w, 1000);
        assert_eq!(w, [0xEA, 0x02, 0x03, 0xE8]);

        for (n, bytes) in [(0usize, vec![0u8]), (30, vec![30]), (31, vec![31, 31]), (200, vec![31, 0x81, 0x48])] {
            let mut w = Vec::new();
            put_value_length(&mut w, n);
            assert_eq!(w, bytes);
            assert_eq!(R::new(&bytes).value_length(), Ok(n));
        }
        assert!(R::new(&[0x20]).value_length().is_err());
    }

    #[test]
    fn text_quote_rule() {
        let mut w = Vec::new();
        put_text(&mut w, "abc");
        put_text(&mut w, "\u{e9}t\u{e9}");
        put_text(&mut w, "\"q\"");
        put_text(&mut w, "");
        put_text(&mut w, "a\0b");
        assert_eq!(&w[..4], b"abc\0");
        assert_eq!(&w[4..11], &[0x7F, 0xC3, 0xA9, b't', 0xC3, 0xA9, 0]);
        assert_eq!(&w[11..16], &[0x7F, b'"', b'q', b'"', 0]);
        let mut r = R::new(&w);
        assert_eq!(r.text().unwrap(), "abc");
        assert_eq!(r.text().unwrap(), "\u{e9}t\u{e9}");
        assert_eq!(r.text().unwrap(), "\"q\"");
        assert_eq!(r.text().unwrap(), "");
        assert_eq!(r.text().unwrap(), "ab");
        assert!(r.done());
        // A quoted-string drops its opening quote.
        assert_eq!(R::new(b"\"<id>\0").text().unwrap(), "<id>");
        // Unterminated, or starting with a non-text octet.
        assert_eq!(R::new(b"abc").text(), Err(MmsError::Short));
        assert!(R::new(&[0x85, 0]).text().is_err());
        assert!(R::new(&[0x05, 0]).text().is_err());
    }

    #[test]
    fn encoded_string_values() {
        let mut w = Vec::new();
        put_encoded_string(&mut w, "Hi");
        put_encoded_string(&mut w, "Caf\u{e9}");
        assert_eq!(w, [b'H', b'i', 0, 7, 0xEA, b'C', b'a', b'f', 0xC3, 0xA9, 0]);
        let mut r = R::new(&w);
        assert_eq!(r.encoded_string().unwrap(), "Hi");
        assert_eq!(r.encoded_string().unwrap(), "Caf\u{e9}");
        // Latin-1 charset decodes too; leftover octets inside the length fail.
        assert_eq!(R::new(&[4, 0x84, b'C', 0xE9, 0]).encoded_string().unwrap(), "C\u{e9}");
        assert_eq!(R::new(&[4, 0xEA, b'C', 0, 0]).encoded_string(), Err(MmsError::Trailing));
        assert_eq!(R::new(&[5, 0xEA, b'C', 0]).encoded_string(), Err(MmsError::Short));
    }

    #[test]
    fn minimal_notifyresp_and_ack_bytes() {
        let p = Pdu::notifyresp_ind("T1", Status::Retrieved);
        assert_eq!(p.encode(), [0x8C, 0x83, 0x98, b'T', b'1', 0x00, 0x8D, 0x92, 0x95, 0x81]);
        assert_eq!(Pdu::decode(&p.encode()).unwrap(), p);
        let a = Pdu::acknowledge_ind("T1");
        assert_eq!(a.encode(), [0x8C, 0x85, 0x98, b'T', b'1', 0x00, 0x8D, 0x92]);
        let mut a2 = a.clone();
        a2.report_allowed = Some(true);
        assert_eq!(a2.encode(), [0x8C, 0x85, 0x98, b'T', b'1', 0x00, 0x8D, 0x92, 0x91, 0x80]);
    }

    #[test]
    fn header_value_bytes() {
        let mut p = Pdu::send_conf("1", ResponseStatus::ErrorContentNotAccepted, None);
        p.date = Some(0x6553_F100);
        let e = p.encode();
        assert_eq!(e, [0x8C, 0x81, 0x98, b'1', 0, 0x8D, 0x92, 0x85, 4, 0x65, 0x53, 0xF1, 0x00, 0x92, 0x87]);

        let n = Pdu::notification_ind("2", None, None, 300, TimeSpec::Relative(259_200), "u");
        let e = n.encode();
        // Class Personal, Size 300 (long 2 octets), Expiry relative, Location.
        assert_eq!(
            &e[7..],
            &[0x8A, 0x80, 0x8E, 2, 0x01, 0x2C, 0x88, 5, 0x81, 3, 0x03, 0xF4, 0x80, 0x83, b'u', 0]
        );

        let s = Pdu::send_req("3", Sender::InsertAddress, vec!["+1/TYPE=PLMN".into()], None, Body::mixed(vec![]));
        let e = s.encode();
        let mut want = vec![0x8C, 0x80, 0x98, b'3', 0, 0x8D, 0x92, 0x89, 0x01, 0x81, 0x97];
        want.extend_from_slice(b"+1/TYPE=PLMN\0");
        want.extend_from_slice(&[0x84, 0xA3, 0x00]);
        assert_eq!(e, want);

        let mut r = Pdu::read_rec_ind("m", "a", "b", None, ReadStatus::Read);
        r.from = Some(Sender::Address("b".into()));
        let e = r.encode();
        assert_eq!(&e[4..], &[0x89, 3, 0x80, b'b', 0, 0x97, b'a', 0, 0x8B, b'm', 0, 0x9B, 0x80]);
    }

    #[test]
    fn content_type_bytes() {
        assert_eq!(ContentType::new("text/plain").encode(), [0x83]);
        assert_eq!(ContentType::new("image/png").encode(), [0xA0]);
        assert_eq!(ContentType::new("image/gif").encode(), [0x9D]);
        assert_eq!(ContentType::new("Text/X-vCard").encode(), [0x87]);
        assert_eq!(ContentType::new(MULTIPART_MIXED).encode(), [0xA3]);
        assert_eq!(ContentType::new(MMS_CONTENT_TYPE).encode(), [0xBE]);
        assert_eq!(ContentType::new("audio/wav").encode(), b"audio/wav\0");
        let t = ContentType { charset: Some(106), ..ContentType::new("text/plain") };
        assert_eq!(t.encode(), [0x03, 0x83, 0x81, 0xEA]);
        let rel = body().content_type;
        let mut want = vec![27, 0xB3, 0x89];
        want.extend_from_slice(b"application/smil\0");
        want.push(0x8A);
        want.extend_from_slice(b"<smil>\0");
        assert_eq!(rel.encode(), want);
        for ct in [t, rel, ContentType { name: Some("a b.png".into()), filename: Some("x".into()), ..ContentType::new("image/png") }] {
            assert_eq!(ContentType::decode(&ct.encode()).unwrap(), ct);
        }
        // A long parameter list takes the uintvar length form.
        let long = ContentType { name: Some("n".repeat(40)), ..ContentType::new("image/png") };
        let e = long.encode();
        assert_eq!(&e[..3], &[31, 43, 0xA0]);
        assert_eq!(ContentType::decode(&e).unwrap(), long);
    }

    #[test]
    fn content_type_alternative_forms() {
        // Long-integer media number, WSP 1.4 typed parameters, Q, an unknown typed one.
        let b = [12, 1, 0x33, 0x99, b'<', b'a', b'>', 0, 0x80, 0x01, 0x97, b'n', 0];
        let b = {
            let mut v = b.to_vec();
            v[0] = (v.len() - 1) as u8;
            v
        };
        let ct = ContentType::decode(&b).unwrap();
        assert_eq!(ct.media, MULTIPART_RELATED);
        assert_eq!(ct.start.as_deref(), Some("<a>"));
        assert_eq!(ct.name.as_deref(), Some("n"));
        // Untyped parameters by name, a text charset, a quoted value, an extension parameter.
        let mut inner = b"text/plain\0".to_vec();
        inner.extend_from_slice(b"charset\0utf-8\0name\0\"hi\0x-foo\0bar\0level\0\x81");
        let mut v = Vec::new();
        with_length(&mut v, &inner);
        assert_eq!(v[0], 31);
        let ct = ContentType::decode(&v).unwrap();
        assert_eq!(ct.media, "text/plain");
        assert_eq!(ct.charset, Some(106));
        assert_eq!(ct.name.as_deref(), Some("hi"));
        // Unassigned numbers, empty media, trailing octets.
        assert!(ContentType::decode(&[0xFF]).is_err());
        assert!(ContentType::decode(&[0x00]).is_err());
        assert!(ContentType::decode(&[1, 0x00]).is_err());
        assert_eq!(ContentType::decode(&[0x83, 0x00]), Err(MmsError::Trailing));
        assert_eq!(ContentType::decode(&[3, 0x83, 0x81]), Err(MmsError::Short));
    }

    #[test]
    fn every_pdu_type_round_trips() {
        let mut types = std::collections::HashSet::new();
        for p in corpus() {
            let e = p.encode();
            let d = Pdu::decode(&e).unwrap_or_else(|err| panic!("{}: {err}", p.message_type.name()));
            assert_eq!(d, p, "{}", p.message_type.name());
            assert_eq!(d.encode(), e);
            types.insert(p.message_type);
        }
        assert_eq!(types.len(), 9);
    }

    #[test]
    fn message_body_helpers() {
        let b = body();
        assert_eq!(b.parts.len(), 4);
        assert!(b.content_type.is(MULTIPART_RELATED));
        let smil = String::from_utf8(b.smil().unwrap().data.clone()).unwrap();
        assert!(smil.contains("<text src=\"text_0.txt\" region=\"Text\"/>"));
        assert!(smil.contains("<img src=\"ore.png\" region=\"Image\"/>"));
        assert!(smil.contains("<audio src=\"ping.wav\"/>"));
        let d = Body::decode(&b.encode()).unwrap();
        assert_eq!(d, b);
        assert_eq!(d.text().as_deref(), Some("Hello from the mine \u{1F48E}"));
        let att = d.attachments();
        assert_eq!(att.len(), 2);
        assert_eq!(att[0].content_type, "image/png");
        assert_eq!(att[0].data, png_blob());
        assert_eq!(att[0].name.as_deref(), Some("ore.png"));
        assert_eq!(att[1].content_type, "audio/wav");
        assert_eq!(att[1].data, wav_blob());

        // Without SMIL: multipart.mixed; a vCard and an unnamed attachment.
        let m = Body::message(None, vec![Part::vcard("steve.vcf", b"BEGIN:VCARD\r\n".to_vec()), Part::new("image/jpeg", vec![1, 2])], false);
        assert!(m.content_type.is(MULTIPART_MIXED));
        assert!(m.text().is_none() && m.smil().is_none());
        assert_eq!(m.parts[1].content_location.as_deref(), Some("part_2"));
        let d = Body::decode(&m.encode()).unwrap();
        assert_eq!(d, m);
        assert_eq!(d.attachments()[0].content_type, "text/x-vcard");
    }

    #[test]
    fn part_bytes() {
        let mut p = Part::text("hi");
        p.content_id = Some("t".into());
        p.content_location = Some("t.txt".into());
        let e = encode_multipart(&[p.clone()]);
        let mut want = vec![1, 17, 2, 3, 0x83, 0x81, 0xEA, 0xC0, 0x22, b'<', b't', b'>', 0, 0x8E];
        want.extend_from_slice(b"t.txt\0hi");
        assert_eq!(e, want);
        assert_eq!(decode_multipart(&e).unwrap(), vec![p.clone()]);
        // Application-header forms of Content-ID / Content-Location; unknown headers skipped.
        let mut h = vec![0x83];
        h.extend_from_slice(b"Content-ID\0<x>\0Content-Location\0x.txt\0X-Other\0v\0");
        h.extend_from_slice(&[0xAE, 2, 0x80, 0x00]); // Content-Disposition (skipped)
        let mut v = vec![1];
        put_uintvar(&mut v, h.len() as u32);
        v.push(1);
        v.extend_from_slice(&h);
        v.push(b'z');
        let d = decode_multipart(&v).unwrap();
        assert_eq!(d[0].content_id.as_deref(), Some("x"));
        assert_eq!(d[0].content_location.as_deref(), Some("x.txt"));
        assert_eq!(d[0].data, b"z");
        // Trailing octets after the last part, a header below 0x20.
        let mut t = e.clone();
        t.push(0);
        assert_eq!(decode_multipart(&t), Err(MmsError::Trailing));
        assert!(decode_multipart(&[1, 2, 0, 0x83, 0x05]).is_err());
        // A huge part count with no parts fails cleanly.
        assert_eq!(decode_multipart(&[0x8F, 0xFF, 0xFF, 0xFF, 0x7F]), Err(MmsError::Short));
    }

    #[test]
    fn single_part_body() {
        let b = Body { content_type: ContentType { charset: Some(106), ..ContentType::new("text/plain") }, parts: vec![] };
        let p = Pdu::send_req("s", Sender::Address("+1/TYPE=PLMN".into()), to(), None, b);
        let d = Pdu::decode(&p.encode()).unwrap();
        let body = d.body.as_ref().unwrap();
        assert_eq!(body.parts.len(), 1);
        assert!(body.parts[0].data.is_empty());
        let mut p2 = d.clone();
        p2.body.as_mut().unwrap().parts[0].data = b"plain body".to_vec();
        let d2 = Pdu::decode(&p2.encode()).unwrap();
        assert_eq!(d2, p2);
        assert_eq!(d2.body.unwrap().text().as_deref(), Some("plain body"));
    }

    #[test]
    fn header_order_and_duplicates() {
        let ok = Pdu::notifyresp_ind("T", Status::Deferred).encode();
        // Version before Transaction-Id.
        assert!(Pdu::decode(&[0x8C, 0x83, 0x8D, 0x92, 0x98, b'T', 0, 0x95, 0x81]).is_err());
        // Something other than Message-Type first.
        assert!(Pdu::decode(&[0x98, b'T', 0, 0x8C, 0x83, 0x8D, 0x92, 0x95, 0x81]).is_err());
        // Repeated Status / Message-Type.
        let mut d = ok.clone();
        d.extend_from_slice(&[0x95, 0x81]);
        assert_eq!(Pdu::decode(&d), Err(MmsError::Duplicate(0x95)));
        let mut d = ok.clone();
        d.extend_from_slice(&[0x8C, 0x83]);
        assert_eq!(Pdu::decode(&d), Err(MmsError::Duplicate(0x8C)));
        // To may repeat.
        let mut di = Pdu::delivery_ind("m", "a", 1, Status::Retrieved);
        di.to.push("b".into());
        assert_eq!(Pdu::decode(&di.encode()).unwrap(), di);
        // Content-Type in a bodiless PDU.
        let mut d = ok.clone();
        d.extend_from_slice(&[0x84, 0x83]);
        assert!(Pdu::decode(&d).is_err());
        // Unknown well-known and application headers are skipped.
        let mut d = ok.clone();
        d.extend_from_slice(&[0xA0, 0x02, 0x01, 0x02, 0x9F, 0x80, 0xB0]);
        d.extend_from_slice(b"text\0");
        d.extend_from_slice(b"X-Ecm-Thing\0value\0");
        assert_eq!(Pdu::decode(&d).unwrap(), Pdu::decode(&ok).unwrap());
        // Bad values.
        assert!(Pdu::decode(&[0x8C, 0x83, 0x98, b'T', 0, 0x8D, 0x92, 0x95, 0x88]).is_err());
        assert!(Pdu::decode(&[0x8C, 0x99, 0x8D, 0x92]).is_err());
        assert_eq!(Pdu::decode(&[0x8C, 0x93, 0x8D, 0x92]), Err(MmsError::MessageType(0x93)));
        assert!(Pdu::decode(&[0x8C, 0x83, 0x98, b'T', 0, 0x8D, 0x92, 0x95, 0x81, 0x10]).is_err());
        assert!(Pdu::decode(&[0x8C, 0x83, 0x98, b'T', 0, 0x8D, 0x12, 0x95, 0x81]).is_err());
    }

    #[test]
    fn mandatory_headers_enforced() {
        let cases: Vec<(Pdu, &str)> = vec![
            (Pdu::new(MessageType::NotifyRespInd), "X-Mms-Transaction-Id"),
            (Pdu { transaction_id: Some("t".into()), ..Pdu::new(MessageType::NotifyRespInd) }, "X-Mms-Status"),
            (Pdu { transaction_id: Some("t".into()), ..Pdu::new(MessageType::SendConf) }, "X-Mms-Response-Status"),
            (Pdu { transaction_id: Some("t".into()), ..Pdu::new(MessageType::SendReq) }, "From"),
            (
                Pdu { transaction_id: Some("t".into()), from: Some(Sender::InsertAddress), ..Pdu::new(MessageType::SendReq) },
                "Content-Type",
            ),
            (Pdu { content_location: None, ..Pdu::notification_ind("t", None, None, 1, TimeSpec::Relative(1), "u") }, "X-Mms-Content-Location"),
            (Pdu { date: None, ..Pdu::retrieve_conf(None, None, 1, None, vec![], None, body()) }, "Date"),
            (Pdu { to: vec![], ..Pdu::delivery_ind("m", "a", 1, Status::Expired) }, "To"),
            (Pdu { date: None, ..Pdu::read_orig_ind("m", "a", "b", 1, ReadStatus::Read) }, "Date"),
            (Pdu { from: None, ..Pdu::read_rec_ind("m", "a", "b", None, ReadStatus::Read) }, "From"),
        ];
        for (p, what) in cases {
            assert_eq!(Pdu::decode(&p.encode()), Err(MmsError::Missing(what)), "{}", p.message_type.name());
        }
        // retrieve-conf without a Transaction-Id is fine.
        let r = Pdu::retrieve_conf(None, None, 1, None, vec![], None, body());
        assert_eq!(Pdu::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn every_truncation_is_an_error() {
        let mut list = corpus();
        // m-acknowledge-ind's only optional header could be cut off cleanly;
        // check the plain one here (the corpus covers the other in round trips).
        list.retain(|p| !(p.message_type == MessageType::AcknowledgeInd && p.report_allowed.is_some()));
        for p in list {
            let e = p.encode();
            for n in 0..e.len() {
                assert!(Pdu::decode(&e[..n]).is_err(), "{} cut at {n}/{}", p.message_type.name(), e.len());
            }
        }
        let e = body().encode();
        for n in 0..e.len() {
            assert!(Body::decode(&e[..n]).is_err(), "body cut at {n}");
        }
    }

    #[test]
    fn wap_push() {
        let notif = Pdu::notification_ind("tx", None, None, 10, TimeSpec::Relative(60), "http://m/1").encode();
        let w = wap_push_encode(&notif, 0x2A);
        assert_eq!(&w[..6], &[0x2A, 0x06, 0x03, 0xBE, 0xAF, 0x84]);
        let (ct, b) = wap_push_decode(&w).unwrap();
        assert_eq!(ct, MMS_CONTENT_TYPE);
        assert_eq!(b, notif);
        assert_eq!(Pdu::decode(&b).unwrap().content_location.as_deref(), Some("http://m/1"));
        // Text content type, ConfirmedPush, an application header.
        let mut h = b"application/vnd.wap.mms-message\0".to_vec();
        h.extend_from_slice(b"X-Wap-Application-Id\0x-wap-application:mms.ua\0");
        let mut v = vec![1, 0x07];
        put_uintvar(&mut v, h.len() as u32);
        v.extend_from_slice(&h);
        v.extend_from_slice(&[9, 9]);
        assert_eq!(wap_push_decode(&v).unwrap(), (MMS_CONTENT_TYPE.to_string(), vec![9, 9]));
        // Not a push, header length past the end, truncated headers.
        assert!(wap_push_decode(&[1, 0x04, 0x01, 0xBE]).is_err());
        assert_eq!(wap_push_decode(&[1, 0x06, 0x05, 0xBE]), Err(MmsError::Short));
        for n in 0..6 {
            assert!(wap_push_decode(&w[..n]).is_err(), "cut {n}");
        }
    }

    #[test]
    fn addresses() {
        assert_eq!(plmn_address("+12145550142"), "+12145550142/TYPE=PLMN");
        assert_eq!(parse_address("+12145550142/TYPE=PLMN"), ("+12145550142", Some("PLMN")));
        assert_eq!(parse_address("+1/type=plmn"), ("+1", Some("plmn")));
        assert_eq!(parse_address("steve@mine.example"), ("steve@mine.example", None));
        let p = Pdu::retrieve_conf(None, None, 1, Some("+1/TYPE=PLMN"), vec![], None, body());
        assert_eq!(p.from_address(), Some("+1/TYPE=PLMN"));
    }

    #[test]
    fn random_and_flipped_inputs_never_panic() {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let samples: Vec<Vec<u8>> = corpus().iter().map(Pdu::encode).collect();
        let pushes: Vec<Vec<u8>> = samples.iter().map(|p| wap_push_encode(p, 7)).collect();
        for i in 0..20_000 {
            let input: Vec<u8> = match i % 3 {
                0 => {
                    let n = (rnd() % 64) as usize;
                    let mut v: Vec<u8> = (0..n).map(|_| rnd() as u8).collect();
                    if i % 2 == 0 && v.len() > 4 {
                        // Mostly-valid prefix to reach the header parser.
                        v[0] = 0x8C;
                        v[1] = 0x80 + (rnd() % 9) as u8;
                    }
                    v
                }
                1 => {
                    let mut v = samples[(rnd() as usize) % samples.len()].clone();
                    for _ in 0..1 + rnd() % 3 {
                        let bit = (rnd() as usize) % (v.len() * 8);
                        v[bit / 8] ^= 1 << (bit % 8);
                    }
                    v
                }
                _ => {
                    let mut v = pushes[(rnd() as usize) % pushes.len()].clone();
                    let bit = (rnd() as usize) % (v.len() * 8);
                    v[bit / 8] ^= 1 << (bit % 8);
                    let cut = (rnd() as usize) % (v.len() + 1);
                    v.truncate(cut.max(v.len() / 2));
                    v
                }
            };
            let _ = Pdu::decode(&input);
            let _ = wap_push_decode(&input);
            let _ = Body::decode(&input);
            let _ = decode_multipart(&input);
            let _ = ContentType::decode(&input);
            if let Ok(p) = Pdu::decode(&input) {
                // Anything that decodes re-encodes and decodes to itself.
                let again = Pdu::decode(&p.encode());
                assert!(again.is_ok(), "re-decode of {input:02X?}");
            }
        }
    }
}
