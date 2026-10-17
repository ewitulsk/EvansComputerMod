//! RFC 1035 messages: header flags, questions, resource records (A, NS, CNAME, SOA, PTR,
//! MX, TXT; anything else is kept as raw RDATA), decode with hop and size limits, encode
//! with name compression.

use crate::name::{fqdn, ip_str, normalize, Ip};
use std::collections::HashMap;
use std::fmt::Write as _;

pub const A: u16 = 1;
pub const NS: u16 = 2;
pub const CNAME: u16 = 5;
pub const SOA: u16 = 6;
pub const PTR: u16 = 12;
pub const MX: u16 = 15;
pub const TXT: u16 = 16;
pub const AAAA: u16 = 28;
pub const AXFR: u16 = 252;
pub const ANY: u16 = 255;
pub const IN: u16 = 1;

pub const OP_QUERY: u8 = 0;
pub const OP_NOTIFY: u8 = 4;

pub const NOERROR: u8 = 0;
pub const FORMERR: u8 = 1;
pub const SERVFAIL: u8 = 2;
pub const NXDOMAIN: u8 = 3;
pub const NOTIMP: u8 = 4;
pub const REFUSED: u8 = 5;
pub const NOTAUTH: u8 = 9;

/// Compression pointers followed while reading one name.
pub const MAX_POINTER_HOPS: usize = 16;
/// Classic UDP limit (no EDNS): longer answers are truncated with TC set.
pub const UDP_MAX: usize = 512;
/// Records per message we accept (a 64 KiB AXFR message holds far fewer of ours).
pub const MAX_RECORDS: usize = 4096;

pub fn type_name(t: u16) -> String {
    match t {
        A => "A".into(),
        NS => "NS".into(),
        CNAME => "CNAME".into(),
        SOA => "SOA".into(),
        PTR => "PTR".into(),
        MX => "MX".into(),
        TXT => "TXT".into(),
        AAAA => "AAAA".into(),
        AXFR => "AXFR".into(),
        ANY => "ANY".into(),
        n => format!("TYPE{n}"),
    }
}

pub fn type_from_name(s: &str) -> Option<u16> {
    let u = s.to_ascii_uppercase();
    Some(match u.as_str() {
        "A" => A,
        "NS" => NS,
        "CNAME" => CNAME,
        "SOA" => SOA,
        "PTR" => PTR,
        "MX" => MX,
        "TXT" => TXT,
        "AAAA" => AAAA,
        "AXFR" => AXFR,
        "ANY" | "*" => ANY,
        _ => return u.strip_prefix("TYPE")?.parse().ok(),
    })
}

pub fn rcode_name(r: u8) -> String {
    match r {
        NOERROR => "NOERROR".into(),
        FORMERR => "FORMERR".into(),
        SERVFAIL => "SERVFAIL".into(),
        NXDOMAIN => "NXDOMAIN".into(),
        NOTIMP => "NOTIMP".into(),
        REFUSED => "REFUSED".into(),
        NOTAUTH => "NOTAUTH".into(),
        n => format!("RCODE{n}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Soa {
    pub mname: String,
    pub rname: String,
    pub serial: u32,
    pub refresh: u32,
    pub retry: u32,
    pub expire: u32,
    pub minimum: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RData {
    A(Ip),
    Ns(String),
    Cname(String),
    Ptr(String),
    Mx(u16, String),
    Txt(Vec<String>),
    Soa(Soa),
    Other(Vec<u8>),
}

impl RData {
    /// Master-file / dig presentation of the data.
    pub fn to_text(&self) -> String {
        match self {
            RData::A(ip) => ip_str(*ip),
            RData::Ns(n) | RData::Cname(n) | RData::Ptr(n) => fqdn(n),
            RData::Mx(p, n) => format!("{p} {}", fqdn(n)),
            RData::Txt(parts) => parts
                .iter()
                .map(|p| format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(" "),
            RData::Soa(s) => format!(
                "{} {} {} {} {} {} {}",
                fqdn(&s.mname),
                fqdn(&s.rname),
                s.serial,
                s.refresh,
                s.retry,
                s.expire,
                s.minimum
            ),
            RData::Other(b) => {
                let mut s = format!("\\# {}", b.len());
                if !b.is_empty() {
                    s.push(' ');
                    for x in b {
                        let _ = write!(s, "{x:02x}");
                    }
                }
                s
            }
        }
    }

    /// A target name the record points at (NS, CNAME, PTR, MX).
    pub fn target(&self) -> Option<&str> {
        match self {
            RData::Ns(n) | RData::Cname(n) | RData::Ptr(n) | RData::Mx(_, n) => Some(n),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    pub class: u16,
    pub ttl: u32,
    pub data: RData,
}

impl Record {
    pub fn new(name: &str, ttl: u32, data: RData) -> Record {
        let rtype = match &data {
            RData::A(_) => A,
            RData::Ns(_) => NS,
            RData::Cname(_) => CNAME,
            RData::Ptr(_) => PTR,
            RData::Mx(..) => MX,
            RData::Txt(_) => TXT,
            RData::Soa(_) => SOA,
            RData::Other(_) => 0,
        };
        Record { name: name.to_string(), rtype, class: IN, ttl, data }
    }

    /// One line as `dig` prints it: `www.lab.test.  300  IN  A  10.0.0.3`.
    pub fn to_text(&self) -> String {
        let class = if self.class == IN { "IN".to_string() } else { format!("CLASS{}", self.class) };
        format!("{}\t{}\t{}\t{}\t{}", fqdn(&self.name), self.ttl, class, type_name(self.rtype), self.data.to_text())
    }

    /// The same in aligned columns (the console has no tab stops).
    pub fn to_columns(&self) -> String {
        format!("{:<23} {:>6} IN {:<5} {}", fqdn(&self.name), self.ttl, type_name(self.rtype), self.data.to_text())
    }

    pub fn soa(&self) -> Option<&Soa> {
        match &self.data {
            RData::Soa(s) => Some(s),
            _ => None,
        }
    }

    pub fn a(&self) -> Option<Ip> {
        match self.data {
            RData::A(ip) => Some(ip),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub id: u16,
    pub qr: bool,
    pub opcode: u8,
    pub aa: bool,
    pub tc: bool,
    pub rd: bool,
    pub ra: bool,
    pub rcode: u8,
}

impl Header {
    fn flags(&self) -> u16 {
        (self.qr as u16) << 15
            | ((self.opcode as u16) & 0xf) << 11
            | (self.aa as u16) << 10
            | (self.tc as u16) << 9
            | (self.rd as u16) << 8
            | (self.ra as u16) << 7
            | (self.rcode as u16 & 0xf)
    }

    fn from_flags(id: u16, f: u16) -> Header {
        Header {
            id,
            qr: f & 0x8000 != 0,
            opcode: ((f >> 11) & 0xf) as u8,
            aa: f & 0x0400 != 0,
            tc: f & 0x0200 != 0,
            rd: f & 0x0100 != 0,
            ra: f & 0x0080 != 0,
            rcode: (f & 0xf) as u8,
        }
    }

    /// `qr aa rd ra` as dig prints it.
    pub fn flag_text(&self) -> String {
        let mut v = Vec::new();
        for (on, n) in [(self.qr, "qr"), (self.aa, "aa"), (self.tc, "tc"), (self.rd, "rd"), (self.ra, "ra")] {
            if on {
                v.push(n);
            }
        }
        v.join(" ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub authority: Vec<Record>,
    pub additional: Vec<Record>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    BadName,
    PointerLoop,
    BadRdata,
    TooMany,
    TooLong,
}

impl Message {
    /// A query for one name (RD as given).
    pub fn query(id: u16, name: &str, qtype: u16, rd: bool) -> Message {
        Message {
            header: Header { id, rd, ..Default::default() },
            questions: vec![Question { name: name.to_string(), qtype, qclass: IN }],
            ..Default::default()
        }
    }

    /// The response skeleton for `q`: same id, question, opcode and RD.
    pub fn response_to(q: &Message, rcode: u8) -> Message {
        Message {
            header: Header {
                id: q.header.id,
                qr: true,
                opcode: q.header.opcode,
                rd: q.header.rd,
                rcode,
                ..Default::default()
            },
            questions: q.questions.clone(),
            ..Default::default()
        }
    }

    pub fn question(&self) -> Option<&Question> {
        self.questions.first()
    }

    pub fn decode(data: &[u8]) -> Result<Message, WireError> {
        let h = data.get(..12).ok_or(WireError::Truncated)?;
        let be = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let header = Header::from_flags(be(0), be(2));
        let counts = [be(4) as usize, be(6) as usize, be(8) as usize, be(10) as usize];
        // Every question takes at least 5 bytes and every record 11.
        if counts[0] * 5 + (counts[1] + counts[2] + counts[3]) * 11 > data.len() - 12 {
            return Err(WireError::Truncated);
        }
        if counts[1] + counts[2] + counts[3] > MAX_RECORDS {
            return Err(WireError::TooMany);
        }
        let mut off = 12;
        let mut m = Message { header, ..Default::default() };
        for _ in 0..counts[0] {
            let (name, o) = read_name(data, off)?;
            let q = data.get(o..o + 4).ok_or(WireError::Truncated)?;
            m.questions.push(Question {
                name,
                qtype: u16::from_be_bytes([q[0], q[1]]),
                qclass: u16::from_be_bytes([q[2], q[3]]),
            });
            off = o + 4;
        }
        for (section, n) in [1, 2, 3].into_iter().zip(&counts[1..]) {
            for _ in 0..*n {
                let (r, o) = read_record(data, off)?;
                off = o;
                match section {
                    1 => m.answers.push(r),
                    2 => m.authority.push(r),
                    _ => m.additional.push(r),
                }
            }
        }
        Ok(m)
    }

    pub fn encode(&self) -> Vec<u8> {
        self.encode_limited(usize::MAX)
    }

    /// Encode for UDP: when it does not fit `max` bytes, drop the additional section,
    /// then whole records from the end of the other sections, and set TC.
    pub fn encode_limited(&self, max: usize) -> Vec<u8> {
        let full = encode_parts(self, self.answers.len(), self.authority.len(), self.additional.len(), false);
        if full.len() <= max {
            return full;
        }
        // Referrals stay useful without glue (the resolver looks the names up), so try
        // without the additional section before truncating.
        let no_add = encode_parts(self, self.answers.len(), self.authority.len(), 0, false);
        if no_add.len() <= max {
            return no_add;
        }
        let (mut an, mut ns) = (self.answers.len(), self.authority.len());
        loop {
            if ns > 0 {
                ns -= 1;
            } else if an > 0 {
                an -= 1;
            }
            let b = encode_parts(self, an, ns, 0, true);
            if b.len() <= max || (an == 0 && ns == 0) {
                return b;
            }
        }
    }
}

fn encode_parts(m: &Message, an: usize, ns: usize, ar: usize, tc: bool) -> Vec<u8> {
    let mut h = m.header;
    h.tc |= tc;
    let mut w = Writer { buf: Vec::with_capacity(512), names: HashMap::new() };
    w.u16(h.id);
    w.u16(h.flags());
    w.u16(m.questions.len() as u16);
    w.u16(an as u16);
    w.u16(ns as u16);
    w.u16(ar as u16);
    for q in &m.questions {
        w.name(&q.name, true);
        w.u16(q.qtype);
        w.u16(q.qclass);
    }
    for r in m.answers[..an].iter().chain(&m.authority[..ns]).chain(&m.additional[..ar]) {
        w.record(r);
    }
    w.buf
}

struct Writer {
    buf: Vec<u8>,
    names: HashMap<String, u16>,
}

impl Writer {
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn name(&mut self, name: &str, compress: bool) {
        let mut rest = name;
        while !rest.is_empty() {
            if compress {
                if let Some(&off) = self.names.get(rest) {
                    self.u16(0xC000 | off);
                    return;
                }
                if self.buf.len() < 0x3FFF {
                    self.names.insert(rest.to_string(), self.buf.len() as u16);
                }
            }
            let (label, tail) = rest.split_once('.').unwrap_or((rest, ""));
            let label = &label.as_bytes()[..label.len().min(63)];
            self.buf.push(label.len() as u8);
            self.buf.extend_from_slice(label);
            rest = tail;
        }
        self.buf.push(0);
    }

    fn record(&mut self, r: &Record) {
        self.name(&r.name, true);
        self.u16(r.rtype);
        self.u16(r.class);
        self.u32(r.ttl);
        let len_at = self.buf.len();
        self.u16(0);
        match &r.data {
            RData::A(ip) => self.buf.extend_from_slice(ip),
            RData::Ns(n) | RData::Cname(n) | RData::Ptr(n) => self.name(n, true),
            RData::Mx(p, n) => {
                self.u16(*p);
                self.name(n, true);
            }
            RData::Txt(parts) => {
                for p in parts {
                    let b = &p.as_bytes()[..p.len().min(255)];
                    self.buf.push(b.len() as u8);
                    self.buf.extend_from_slice(b);
                }
            }
            RData::Soa(s) => {
                self.name(&s.mname, true);
                self.name(&s.rname, true);
                for v in [s.serial, s.refresh, s.retry, s.expire, s.minimum] {
                    self.u32(v);
                }
            }
            RData::Other(b) => self.buf.extend_from_slice(b),
        }
        let len = (self.buf.len() - len_at - 2) as u16;
        self.buf[len_at..len_at + 2].copy_from_slice(&len.to_be_bytes());
    }
}

/// Read a (possibly compressed) name at `offset`: the canonical name and the offset just
/// past it where it started.
pub fn read_name(data: &[u8], offset: usize) -> Result<(String, usize), WireError> {
    let mut name = String::new();
    let mut pos = offset;
    let mut end = None;
    let mut hops = 0;
    let mut wire_len = 1usize;
    loop {
        let len = *data.get(pos).ok_or(WireError::Truncated)? as usize;
        if len == 0 {
            end.get_or_insert(pos + 1);
            break;
        }
        match len & 0xC0 {
            0xC0 => {
                let lo = *data.get(pos + 1).ok_or(WireError::Truncated)? as usize;
                end.get_or_insert(pos + 2);
                hops += 1;
                if hops > MAX_POINTER_HOPS {
                    return Err(WireError::PointerLoop);
                }
                pos = ((len & 0x3F) << 8) | lo;
            }
            0x00 => {
                let label = data.get(pos + 1..pos + 1 + len).ok_or(WireError::Truncated)?;
                wire_len += 1 + len;
                if wire_len > 255 {
                    return Err(WireError::BadName);
                }
                if label.iter().any(|&c| c == b'.' || !c.is_ascii_graphic()) {
                    return Err(WireError::BadName);
                }
                if !name.is_empty() {
                    name.push('.');
                }
                for &c in label {
                    name.push(c.to_ascii_lowercase() as char);
                }
                pos += 1 + len;
            }
            _ => return Err(WireError::BadName),
        }
    }
    Ok((name, end.unwrap_or(pos + 1)))
}

fn read_record(data: &[u8], off: usize) -> Result<(Record, usize), WireError> {
    let (name, o) = read_name(data, off)?;
    let f = data.get(o..o + 10).ok_or(WireError::Truncated)?;
    let rtype = u16::from_be_bytes([f[0], f[1]]);
    let class = u16::from_be_bytes([f[2], f[3]]);
    let ttl = u32::from_be_bytes([f[4], f[5], f[6], f[7]]);
    let rdlen = u16::from_be_bytes([f[8], f[9]]) as usize;
    let start = o + 10;
    let end = start + rdlen;
    let rd = data.get(start..end).ok_or(WireError::Truncated)?;
    // A name inside RDATA must end inside it.
    let name_in = |at: usize| -> Result<(String, usize), WireError> {
        let (n, e) = read_name(data, at)?;
        if e > end {
            return Err(WireError::BadRdata);
        }
        Ok((n, e))
    };
    let exact = |e: usize| if e == end { Ok(()) } else { Err(WireError::BadRdata) };
    let rdata = match rtype {
        A if rdlen == 4 => RData::A([rd[0], rd[1], rd[2], rd[3]]),
        A => return Err(WireError::BadRdata),
        NS | CNAME | PTR => {
            let (n, e) = name_in(start)?;
            exact(e)?;
            match rtype {
                NS => RData::Ns(n),
                CNAME => RData::Cname(n),
                _ => RData::Ptr(n),
            }
        }
        MX => {
            if rdlen < 3 {
                return Err(WireError::BadRdata);
            }
            let (n, e) = name_in(start + 2)?;
            exact(e)?;
            RData::Mx(u16::from_be_bytes([rd[0], rd[1]]), n)
        }
        SOA => {
            let (mname, e1) = name_in(start)?;
            let (rname, e2) = name_in(e1)?;
            let nums = data.get(e2..e2 + 20).ok_or(WireError::BadRdata)?;
            exact(e2 + 20)?;
            let n = |i: usize| u32::from_be_bytes([nums[i], nums[i + 1], nums[i + 2], nums[i + 3]]);
            RData::Soa(Soa { mname, rname, serial: n(0), refresh: n(4), retry: n(8), expire: n(12), minimum: n(16) })
        }
        TXT => {
            let mut parts = Vec::new();
            let mut i = 0;
            while i < rd.len() {
                let l = rd[i] as usize;
                let s = rd.get(i + 1..i + 1 + l).ok_or(WireError::BadRdata)?;
                parts.push(String::from_utf8_lossy(s).into_owned());
                i += 1 + l;
            }
            RData::Txt(parts)
        }
        _ => RData::Other(rd.to_vec()),
    };
    Ok((Record { name, rtype, class, ttl, data: rdata }, end))
}

/// Parse a record's data from master-file tokens (`10.0.0.3`, `ns1 ` + origin, ...).
pub fn rdata_from_text(rtype: u16, tokens: &[String], origin: &str) -> Result<RData, String> {
    let name = |i: usize| -> Result<String, String> {
        let t = tokens.get(i).ok_or("missing name")?;
        crate::name::absolute(t, origin).ok_or_else(|| format!("bad name '{t}'"))
    };
    let num = |i: usize| -> Result<u32, String> {
        let t = tokens.get(i).ok_or("missing number")?;
        crate::zone::parse_ttl(t).ok_or_else(|| format!("bad number '{t}'"))
    };
    let want = |n: usize| -> Result<(), String> {
        if tokens.len() == n {
            Ok(())
        } else {
            Err(format!("{} takes {} value(s), got {}", type_name(rtype), n, tokens.len()))
        }
    };
    Ok(match rtype {
        A => {
            want(1)?;
            RData::A(crate::name::parse_ip(&tokens[0]).ok_or_else(|| format!("bad address '{}'", tokens[0]))?)
        }
        NS => {
            want(1)?;
            RData::Ns(name(0)?)
        }
        CNAME => {
            want(1)?;
            RData::Cname(name(0)?)
        }
        PTR => {
            want(1)?;
            RData::Ptr(name(0)?)
        }
        MX => {
            want(2)?;
            let p = tokens[0].parse().map_err(|_| format!("bad preference '{}'", tokens[0]))?;
            RData::Mx(p, name(1)?)
        }
        TXT => {
            if tokens.is_empty() {
                return Err("TXT needs a string".into());
            }
            RData::Txt(tokens.to_vec())
        }
        SOA => {
            want(7)?;
            RData::Soa(Soa {
                mname: name(0)?,
                rname: name(1)?,
                serial: num(2)?,
                refresh: num(3)?,
                retry: num(4)?,
                expire: num(5)?,
                minimum: num(6)?,
            })
        }
        t => return Err(format!("record type {} is not supported in zone files", type_name(t))),
    })
}

/// Turn a free-form name typed by a user into the canonical form or an error message.
pub fn user_name(s: &str) -> Result<String, String> {
    normalize(s).ok_or_else(|| format!("'{s}' is not a valid domain name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Message {
        let q = Message::query(0x1234, "www.lab.test", A, true);
        let mut r = Message::response_to(&q, NOERROR);
        r.header.aa = true;
        r.answers.push(Record::new("www.lab.test", 300, RData::Cname("web.lab.test".into())));
        r.answers.push(Record::new("web.lab.test", 300, RData::A([10, 0, 0, 3])));
        r.authority.push(Record::new("lab.test", 3600, RData::Ns("ns1.lab.test".into())));
        r.additional.push(Record::new("ns1.lab.test", 3600, RData::A([10, 0, 0, 2])));
        r.additional.push(Record::new("lab.test", 60, RData::Txt(vec!["hello \"world\"".into(), "x".into()])));
        r.additional.push(Record::new("lab.test", 60, RData::Mx(10, "mail.lab.test".into())));
        r.additional.push(Record::new(
            "lab.test",
            60,
            RData::Soa(Soa {
                mname: "ns1.lab.test".into(),
                rname: "admin.lab.test".into(),
                serial: 7,
                refresh: 60,
                retry: 30,
                expire: 600,
                minimum: 30,
            }),
        ));
        r
    }

    #[test]
    fn roundtrip_and_compression() {
        let m = sample();
        let b = m.encode();
        let d = Message::decode(&b).unwrap();
        assert_eq!(d, m);
        // compression: "lab.test" appears once in full
        assert_eq!(b.windows(4).filter(|w| w == b"\x03lab").count(), 1);
        assert_eq!(d.answers[1].to_text(), "web.lab.test.\t300\tIN\tA\t10.0.0.3");
        assert_eq!(d.additional[1].data.to_text(), "\"hello \\\"world\\\"\" \"x\"");
    }

    #[test]
    fn truncation_sets_tc() {
        let mut m = sample();
        for i in 0..60 {
            m.answers.push(Record::new(&format!("h{i}.lab.test"), 60, RData::A([10, 0, 1, i as u8])));
        }
        let b = m.encode_limited(UDP_MAX);
        assert!(b.len() <= UDP_MAX);
        let d = Message::decode(&b).unwrap();
        assert!(d.header.tc);
        assert!(d.additional.is_empty());
        // without the additional section it fits: no TC
        let mut small = sample();
        small.additional.push(Record::new("x.lab.test", 1, RData::Other(vec![0; 600])));
        let d = Message::decode(&small.encode_limited(UDP_MAX)).unwrap();
        assert!(!d.header.tc && d.additional.is_empty() && d.answers.len() == 2);
    }

    #[test]
    fn garbage_is_rejected_not_panicking() {
        let b = sample().encode();
        for n in 0..b.len() {
            let _ = Message::decode(&b[..n]);
        }
        let mut rng = 0x1234_5678u32;
        for _ in 0..20000 {
            let mut x = b.clone();
            for _ in 0..3 {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let i = rng as usize % x.len();
                x[i] = (rng >> 8) as u8;
            }
            if let Ok(m) = Message::decode(&x) {
                let _ = Message::decode(&m.encode()).unwrap();
            }
        }
        // a pointer to itself
        let mut d = vec![0u8; 12];
        d[5] = 1;
        d.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
        assert_eq!(Message::decode(&d), Err(WireError::PointerLoop));
        // counts claiming more records than bytes
        let mut d = vec![0u8; 12];
        d[6] = 0xff;
        assert_eq!(Message::decode(&d), Err(WireError::Truncated));
    }

    #[test]
    fn types() {
        assert_eq!(type_from_name("mx"), Some(MX));
        assert_eq!(type_from_name("TYPE99"), Some(99));
        assert_eq!(type_name(99), "TYPE99");
        assert_eq!(type_from_name("nope"), None);
    }
}
