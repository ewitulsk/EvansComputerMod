//! SDP (RFC 4566 subset) and VoLTE-style offer/answer (RFC 3264).
//!
//! A session description is text, one `x=value` per line, CRLF ends:
//!
//! ```text
//! v=0
//! o=- 4711 1 IN IP4 10.45.0.7          username sess-id sess-version, address
//! s=-
//! c=IN IP4 10.45.0.7                   where to send media (session or media level)
//! t=0 0
//! m=audio 40000 RTP/AVP 96 97 98 101   kind, RTP port, profile, payload types
//! a=rtpmap:96 ADPCM-WB/16000           dynamic PT -> codec/clock
//! a=rtpmap:97 ADPCM-NB/8000
//! a=rtpmap:98 telephone-event/16000    DTMF (RFC 4733) at the WB clock
//! a=rtpmap:101 telephone-event/8000    ... and at the NB clock
//! a=fmtp:98 0-15
//! a=fmtp:101 0-15
//! a=ptime:20
//! a=sendrecv
//! ```
//!
//! The mod's codecs ([`Codec`]): **ADPCM-WB** (HD voice, IMA ADPCM at
//! 16 kHz, PT 96) and **ADPCM-NB** (IMA ADPCM at 8 kHz, PT 97), with
//! telephone-event at the matching clock (98 / 101, events `0-15`). Payload
//! types are dynamic, so negotiation goes by the `rtpmap` names, never the
//! numbers: an answer reuses the offerer's numbers.
//!
//! [`answer`] picks WB when both sides support it, else NB, else fails with
//! [`SdpError::NotAcceptable`] (SIP 488). Other media lines are rejected
//! (port 0) as RFC 3264 requires. Hold is a re-offer with
//! [`SessionDescription::with_direction`] (`sendonly`), answered `recvonly`.
//!
//! Parsing is strict about the lines it understands (`v o s c t m a` and the
//! `rtpmap`/`fmtp`/`ptime`/direction attributes) and returns errors, never
//! panics; other line types (`b= i= u= e= p= k= r= z=`) are skipped and other
//! attributes are kept raw in `attrs`. A session-level direction applies to
//! media without their own.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdpError {
    /// Not UTF-8.
    Utf8,
    /// A line that isn't `x=value`, or `v=` isn't first/`0`.
    Line(&'static str),
    /// A required line is missing.
    Missing(&'static str),
    /// A malformed value (which line or attribute).
    Value(&'static str),
    /// No common codec (SIP 488 Not Acceptable Here).
    NotAcceptable,
}

impl SdpError {
    /// The SIP status to answer an INVITE with.
    pub fn sip_code(&self) -> u16 {
        match self {
            SdpError::NotAcceptable => 488,
            _ => 400,
        }
    }
}

impl fmt::Display for SdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdpError::Utf8 => write!(f, "SDP is not UTF-8"),
            SdpError::Line(w) => write!(f, "bad SDP line ({w})"),
            SdpError::Missing(w) => write!(f, "SDP missing {w}"),
            SdpError::Value(w) => write!(f, "bad SDP {w}"),
            SdpError::NotAcceptable => write!(f, "no common codec"),
        }
    }
}

impl std::error::Error for SdpError {}

type Res<T> = Result<T, SdpError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::SendRecv => "sendrecv",
            Direction::SendOnly => "sendonly",
            Direction::RecvOnly => "recvonly",
            Direction::Inactive => "inactive",
        }
    }

    fn from_attr(s: &str) -> Option<Direction> {
        Some(match s {
            "sendrecv" => Direction::SendRecv,
            "sendonly" => Direction::SendOnly,
            "recvonly" => Direction::RecvOnly,
            "inactive" => Direction::Inactive,
            _ => return None,
        })
    }

    /// The direction the other side states for the same stream.
    pub fn reverse(self) -> Direction {
        match self {
            Direction::SendOnly => Direction::RecvOnly,
            Direction::RecvOnly => Direction::SendOnly,
            d => d,
        }
    }

    pub fn sends(self) -> bool {
        matches!(self, Direction::SendRecv | Direction::SendOnly)
    }

    pub fn receives(self) -> bool {
        matches!(self, Direction::SendRecv | Direction::RecvOnly)
    }
}

/// `o=<username> <sess-id> <sess-version> IN IP4|IP6 <addr>` (the address type
/// follows from the address: IP6 when it has a `:`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub username: String,
    pub sess_id: u64,
    pub sess_version: u64,
    pub addr: String,
}

/// `a=rtpmap:<pt> <name>/<rate>[/<channels>]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtpMap {
    pub pt: u8,
    pub name: String,
    pub rate: u32,
    pub channels: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
    /// `audio`, `video`...
    pub kind: String,
    /// 0 = rejected / disabled.
    pub port: u16,
    /// `RTP/AVP`.
    pub proto: String,
    pub formats: Vec<u8>,
    /// Media-level `c=` (overrides the session's).
    pub connection: Option<String>,
    pub rtpmap: Vec<RtpMap>,
    pub fmtp: Vec<(u8, String)>,
    pub direction: Direction,
    pub ptime: Option<u32>,
    /// Other attributes, raw (`maxptime:40`).
    pub attrs: Vec<String>,
}

impl Media {
    /// The first listed format whose rtpmap is `name/rate` (name case-insensitive).
    pub fn find_format(&self, name: &str, rate: u32) -> Option<u8> {
        self.formats.iter().copied().find(|pt| self.rtpmap.iter().any(|m| m.pt == *pt && m.rate == rate && m.name.eq_ignore_ascii_case(name)))
    }

    pub fn rtpmap_for(&self, pt: u8) -> Option<&RtpMap> {
        self.rtpmap.iter().find(|m| m.pt == pt)
    }

    pub fn fmtp_for(&self, pt: u8) -> Option<&str> {
        self.fmtp.iter().find(|(p, _)| *p == pt).map(|(_, s)| s.as_str())
    }

    fn rejected(kind: &str, proto: &str, formats: &[u8]) -> Media {
        Media {
            kind: kind.to_string(),
            port: 0,
            proto: proto.to_string(),
            formats: formats.to_vec(),
            connection: None,
            rtpmap: Vec::new(),
            fmtp: Vec::new(),
            direction: Direction::Inactive,
            ptime: None,
            attrs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDescription {
    pub origin: Origin,
    pub session_name: String,
    /// Session-level `c=` address.
    pub connection: Option<String>,
    /// Other session-level attributes, raw.
    pub attrs: Vec<String>,
    pub media: Vec<Media>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// IMA ADPCM, 16 kHz (HD voice).
    AdpcmWb,
    /// IMA ADPCM, 8 kHz.
    AdpcmNb,
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::AdpcmWb => "ADPCM-WB",
            Codec::AdpcmNb => "ADPCM-NB",
        }
    }

    pub fn rate(self) -> u32 {
        match self {
            Codec::AdpcmWb => 16_000,
            Codec::AdpcmNb => 8_000,
        }
    }

    /// The payload type this mod offers it as.
    pub fn default_pt(self) -> u8 {
        match self {
            Codec::AdpcmWb => PT_WB,
            Codec::AdpcmNb => PT_NB,
        }
    }

    /// The telephone-event PT this mod offers at the codec's clock.
    pub fn default_dtmf_pt(self) -> u8 {
        match self {
            Codec::AdpcmWb => PT_DTMF_WB,
            Codec::AdpcmNb => PT_DTMF_NB,
        }
    }
}

pub const PT_WB: u8 = 96;
pub const PT_NB: u8 = 97;
pub const PT_DTMF_WB: u8 = 98;
pub const PT_DTMF_NB: u8 = 101;
pub const TELEPHONE_EVENT: &str = "telephone-event";
pub const DTMF_EVENTS: &str = "0-15";
pub const PTIME: u32 = 20;

/// What the answering side supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub wideband: bool,
    pub narrowband: bool,
    pub dtmf: bool,
}

impl Caps {
    pub const ALL: Caps = Caps { wideband: true, narrowband: true, dtmf: true };
    pub const NARROWBAND: Caps = Caps { wideband: false, narrowband: true, dtmf: true };
}

impl Default for Caps {
    fn default() -> Caps {
        Caps::ALL
    }
}

/// The outcome of offer/answer, from this side's point of view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negotiated {
    pub pt: u8,
    pub codec: Codec,
    pub rate: u32,
    pub dtmf_pt: Option<u8>,
    pub remote_addr: String,
    pub remote_port: u16,
    /// This side's direction (`recvonly` while the far end holds us).
    pub direction: Direction,
    pub ptime: u32,
}

fn rtpmap(pt: u8, name: &str, rate: u32) -> RtpMap {
    RtpMap { pt, name: name.to_string(), rate, channels: None }
}

/// This mod's offer: WB + NB + both telephone-events when `wideband`, else NB only.
pub fn offer(addr: &str, port: u16, sess_id: u64, version: u64, wideband: bool) -> SessionDescription {
    let mut m = Media {
        kind: "audio".into(),
        port,
        proto: "RTP/AVP".into(),
        formats: Vec::new(),
        connection: None,
        rtpmap: Vec::new(),
        fmtp: Vec::new(),
        direction: Direction::SendRecv,
        ptime: Some(PTIME),
        attrs: Vec::new(),
    };
    if wideband {
        m.formats = vec![PT_WB, PT_NB, PT_DTMF_WB, PT_DTMF_NB];
        m.rtpmap = vec![
            rtpmap(PT_WB, Codec::AdpcmWb.name(), 16_000),
            rtpmap(PT_NB, Codec::AdpcmNb.name(), 8_000),
            rtpmap(PT_DTMF_WB, TELEPHONE_EVENT, 16_000),
            rtpmap(PT_DTMF_NB, TELEPHONE_EVENT, 8_000),
        ];
        m.fmtp = vec![(PT_DTMF_WB, DTMF_EVENTS.into()), (PT_DTMF_NB, DTMF_EVENTS.into())];
    } else {
        m.formats = vec![PT_NB, PT_DTMF_NB];
        m.rtpmap = vec![rtpmap(PT_NB, Codec::AdpcmNb.name(), 8_000), rtpmap(PT_DTMF_NB, TELEPHONE_EVENT, 8_000)];
        m.fmtp = vec![(PT_DTMF_NB, DTMF_EVENTS.into())];
    }
    SessionDescription {
        origin: Origin { username: "-".into(), sess_id, sess_version: version, addr: addr.to_string() },
        session_name: "-".into(),
        connection: Some(addr.to_string()),
        attrs: Vec::new(),
        media: vec![m],
    }
}

fn pick(m: &Media, caps: &Caps) -> Option<(Codec, u8)> {
    if m.kind != "audio" || m.port == 0 || !m.proto.starts_with("RTP/") {
        return None;
    }
    [(Codec::AdpcmWb, caps.wideband), (Codec::AdpcmNb, caps.narrowband)]
        .into_iter()
        .filter(|(_, ok)| *ok)
        .find_map(|(c, _)| m.find_format(c.name(), c.rate()).map(|pt| (c, pt)))
}

/// Answers `offer` (RFC 3264): the first usable audio stream gets WB if both
/// sides have it, else NB; every other stream is rejected with port 0.
pub fn answer(offer: &SessionDescription, addr: &str, port: u16, sess_id: u64, version: u64, caps: &Caps) -> Res<(SessionDescription, Negotiated)> {
    let mut chosen = None;
    let mut media = Vec::new();
    for m in &offer.media {
        let p = if chosen.is_none() { pick(m, caps) } else { None };
        let Some((codec, pt)) = p else {
            media.push(Media::rejected(&m.kind, &m.proto, &m.formats));
            continue;
        };
        let remote_addr = m.connection.clone().or_else(|| offer.connection.clone()).ok_or(SdpError::Missing("c="))?;
        let dtmf_pt = if caps.dtmf { m.find_format(TELEPHONE_EVENT, codec.rate()) } else { None };
        let direction = m.direction.reverse();
        let ptime = m.ptime.unwrap_or(PTIME);
        let mut a = Media {
            kind: m.kind.clone(),
            port,
            proto: m.proto.clone(),
            formats: vec![pt],
            connection: None,
            rtpmap: vec![rtpmap(pt, codec.name(), codec.rate())],
            fmtp: Vec::new(),
            direction,
            ptime: Some(ptime),
            attrs: Vec::new(),
        };
        if let Some(d) = dtmf_pt {
            a.formats.push(d);
            a.rtpmap.push(rtpmap(d, TELEPHONE_EVENT, codec.rate()));
            a.fmtp.push((d, m.fmtp_for(d).unwrap_or(DTMF_EVENTS).to_string()));
        }
        media.push(a);
        chosen = Some(Negotiated { pt, codec, rate: codec.rate(), dtmf_pt, remote_addr, remote_port: m.port, direction, ptime });
    }
    let neg = chosen.ok_or(SdpError::NotAcceptable)?;
    let sd = SessionDescription {
        origin: Origin { username: "-".into(), sess_id, sess_version: version, addr: addr.to_string() },
        session_name: "-".into(),
        connection: Some(addr.to_string()),
        attrs: Vec::new(),
        media,
    };
    Ok((sd, neg))
}

/// The offerer's view of an answer to its `offer`.
pub fn accept_answer(offer: &SessionDescription, answer: &SessionDescription) -> Res<Negotiated> {
    for (i, a) in answer.media.iter().enumerate() {
        let Some(o) = offer.media.get(i) else { break };
        if a.port == 0 || o.kind != "audio" || a.kind != o.kind {
            continue;
        }
        for &pt in &a.formats {
            let Some(map) = a.rtpmap_for(pt) else { continue };
            let Some(codec) = [Codec::AdpcmWb, Codec::AdpcmNb].into_iter().find(|c| map.name.eq_ignore_ascii_case(c.name()) && map.rate == c.rate()) else {
                continue;
            };
            if o.find_format(codec.name(), codec.rate()).is_none() {
                return Err(SdpError::NotAcceptable);
            }
            let remote_addr = a.connection.clone().or_else(|| answer.connection.clone()).ok_or(SdpError::Missing("c="))?;
            let dtmf_pt = a.find_format(TELEPHONE_EVENT, codec.rate());
            return Ok(Negotiated {
                pt,
                codec,
                rate: codec.rate(),
                dtmf_pt,
                remote_addr,
                remote_port: a.port,
                direction: a.direction.reverse(),
                ptime: a.ptime.or(o.ptime).unwrap_or(PTIME),
            });
        }
    }
    Err(SdpError::NotAcceptable)
}

fn graphic(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic())
}

fn parse_addr(fields: &[&str], what: &'static str) -> Res<String> {
    let e = SdpError::Value(what);
    let [net, ty, addr] = fields else { return Err(e) };
    if *net != "IN" || !graphic(addr) {
        return Err(e);
    }
    match *ty {
        "IP4" if !addr.contains(':') => Ok(addr.to_string()),
        "IP6" if addr.contains(':') => Ok(addr.to_string()),
        _ => Err(e),
    }
}

fn addr_type(addr: &str) -> &'static str {
    if addr.contains(':') {
        "IP6"
    } else {
        "IP4"
    }
}

fn num<T: std::str::FromStr>(s: &str, what: &'static str) -> Res<T> {
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SdpError::Value(what));
    }
    s.parse().map_err(|_| SdpError::Value(what))
}

fn pt(s: &str) -> Res<u8> {
    let v: u8 = num(s, "payload type")?;
    if v > 127 {
        return Err(SdpError::Value("payload type"));
    }
    Ok(v)
}

fn parse_rtpmap(v: &str) -> Res<RtpMap> {
    let e = SdpError::Value("rtpmap");
    let (p, enc) = v.split_once(char::is_whitespace).ok_or(e.clone())?;
    let parts: Vec<&str> = enc.trim().split('/').collect();
    if !(2..=3).contains(&parts.len()) || !graphic(parts[0]) {
        return Err(e);
    }
    let rate: u32 = num(parts[1], "rtpmap")?;
    let channels = match parts.get(2) {
        Some(c) => Some(num::<u8>(c, "rtpmap")?).filter(|c| *c > 0).ok_or(e.clone()).map(Some)?,
        None => None,
    };
    if rate == 0 {
        return Err(e);
    }
    Ok(RtpMap { pt: pt(p)?, name: parts[0].to_string(), rate, channels })
}

impl SessionDescription {
    pub fn parse(data: &[u8]) -> Res<SessionDescription> {
        let text = std::str::from_utf8(data).map_err(|_| SdpError::Utf8)?;
        let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| !l.is_empty());
        if lines.next() != Some("v=0") {
            return Err(SdpError::Line("v=0 first"));
        }
        let (mut origin, mut name, mut conn) = (None, None, None);
        let mut attrs = Vec::new();
        let mut session_dir = None;
        let mut media: Vec<(Media, Option<Direction>)> = Vec::new();
        for line in lines {
            let b = line.as_bytes();
            if b.len() < 2 || b[1] != b'=' || !b[0].is_ascii_lowercase() {
                return Err(SdpError::Line("not x=value"));
            }
            let v = &line[2..];
            let fields: Vec<&str> = v.split_whitespace().collect();
            match (b[0], media.last_mut()) {
                (b'v', _) => return Err(SdpError::Line("second v=")),
                (b'o', None) => {
                    let [user, id, ver, rest @ ..] = fields.as_slice() else { return Err(SdpError::Value("o=")) };
                    if origin.is_some() {
                        return Err(SdpError::Line("second o="));
                    }
                    origin = Some(Origin { username: user.to_string(), sess_id: num(id, "o=")?, sess_version: num(ver, "o=")?, addr: parse_addr(rest, "o=")? });
                }
                (b's', None) => {
                    if name.replace(v.to_string()).is_some() {
                        return Err(SdpError::Line("second s="));
                    }
                }
                (b'c', None) => conn = Some(parse_addr(&fields, "c=")?),
                (b'c', Some((m, _))) => m.connection = Some(parse_addr(&fields, "c=")?),
                (b'm', _) => {
                    let [kind, port, proto, fmts @ ..] = fields.as_slice() else { return Err(SdpError::Value("m=")) };
                    if fmts.is_empty() || !kind.chars().all(|c| c.is_ascii_alphanumeric()) || !graphic(proto) {
                        return Err(SdpError::Value("m="));
                    }
                    let port = num(port.split_once('/').map_or(*port, |(p, _)| p), "m= port")?;
                    let formats = fmts.iter().map(|f| pt(f)).collect::<Res<Vec<u8>>>()?;
                    media.push((
                        Media {
                            kind: kind.to_string(),
                            port,
                            proto: proto.to_string(),
                            formats,
                            connection: None,
                            rtpmap: Vec::new(),
                            fmtp: Vec::new(),
                            direction: Direction::SendRecv,
                            ptime: None,
                            attrs: Vec::new(),
                        },
                        None,
                    ));
                }
                (b'a', cur) => {
                    let (an, av) = match v.split_once(':') {
                        Some((n, a)) => (n, Some(a)),
                        None => (v, None),
                    };
                    let dir = Direction::from_attr(an);
                    match cur {
                        None => match dir {
                            Some(d) => session_dir = Some(d),
                            None => attrs.push(v.to_string()),
                        },
                        Some((m, mdir)) => match (an, av) {
                            _ if dir.is_some() => *mdir = dir,
                            ("rtpmap", Some(a)) => m.rtpmap.push(parse_rtpmap(a)?),
                            ("fmtp", Some(a)) => {
                                let (p, rest) = a.split_once(char::is_whitespace).ok_or(SdpError::Value("fmtp"))?;
                                let rest = rest.trim();
                                if rest.is_empty() {
                                    return Err(SdpError::Value("fmtp"));
                                }
                                m.fmtp.push((pt(p)?, rest.to_string()));
                            }
                            ("ptime", Some(a)) => {
                                let t: u32 = num(a.trim(), "ptime")?;
                                if !(1..=1000).contains(&t) {
                                    return Err(SdpError::Value("ptime"));
                                }
                                m.ptime = Some(t);
                            }
                            ("rtpmap" | "fmtp" | "ptime", None) => return Err(SdpError::Value("attribute")),
                            _ => m.attrs.push(v.to_string()),
                        },
                    }
                }
                (b'o' | b's', Some(_)) => return Err(SdpError::Line("session line after m=")),
                _ => {} // t= b= i= u= e= p= k= r= z= and unknown types
            }
        }
        let media = media.into_iter().map(|(mut m, d)| {
            m.direction = d.or(session_dir).unwrap_or_default();
            m
        });
        Ok(SessionDescription {
            origin: origin.ok_or(SdpError::Missing("o="))?,
            session_name: name.ok_or(SdpError::Missing("s="))?,
            connection: conn,
            attrs,
            media: media.collect(),
        })
    }

    /// The wire bytes (CRLF line ends).
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }

    /// A re-offer with every active stream set to `direction` and the
    /// session version bumped (hold = `SendOnly`, resume = `SendRecv`).
    pub fn with_direction(&self, direction: Direction) -> SessionDescription {
        let mut sd = self.clone();
        sd.origin.sess_version = sd.origin.sess_version.wrapping_add(1);
        for m in sd.media.iter_mut().filter(|m| m.port != 0) {
            m.direction = direction;
        }
        sd
    }
}

impl fmt::Display for SessionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let o = &self.origin;
        write!(f, "v=0\r\no={} {} {} IN {} {}\r\ns={}\r\n", o.username, o.sess_id, o.sess_version, addr_type(&o.addr), o.addr, self.session_name)?;
        if let Some(c) = &self.connection {
            write!(f, "c=IN {} {c}\r\n", addr_type(c))?;
        }
        f.write_str("t=0 0\r\n")?;
        for a in &self.attrs {
            write!(f, "a={a}\r\n")?;
        }
        for m in &self.media {
            write!(f, "m={} {} {}", m.kind, m.port, m.proto)?;
            for p in &m.formats {
                write!(f, " {p}")?;
            }
            f.write_str("\r\n")?;
            if let Some(c) = &m.connection {
                write!(f, "c=IN {} {c}\r\n", addr_type(c))?;
            }
            for r in &m.rtpmap {
                write!(f, "a=rtpmap:{} {}/{}", r.pt, r.name, r.rate)?;
                if let Some(ch) = r.channels {
                    write!(f, "/{ch}")?;
                }
                f.write_str("\r\n")?;
            }
            for (p, s) in &m.fmtp {
                write!(f, "a=fmtp:{p} {s}\r\n")?;
            }
            if let Some(t) = m.ptime {
                write!(f, "a=ptime:{t}\r\n")?;
            }
            write!(f, "a={}\r\n", m.direction.as_str())?;
            for a in &m.attrs {
                write!(f, "a={a}\r\n")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(sd: &SessionDescription) {
        let wire = sd.to_bytes();
        assert_eq!(&SessionDescription::parse(&wire).unwrap(), sd);
    }

    #[test]
    fn offer_wire_format() {
        let o = offer("10.45.0.7", 40000, 4711, 1, true);
        let s = o.to_string();
        assert_eq!(
            s,
            "v=0\r\no=- 4711 1 IN IP4 10.45.0.7\r\ns=-\r\nc=IN IP4 10.45.0.7\r\nt=0 0\r\nm=audio 40000 RTP/AVP 96 97 98 101\r\n\
a=rtpmap:96 ADPCM-WB/16000\r\na=rtpmap:97 ADPCM-NB/8000\r\na=rtpmap:98 telephone-event/16000\r\na=rtpmap:101 telephone-event/8000\r\n\
a=fmtp:98 0-15\r\na=fmtp:101 0-15\r\na=ptime:20\r\na=sendrecv\r\n"
        );
        rt(&o);
        rt(&offer("fd00::7", 40002, 1, 2, false));
    }

    #[test]
    fn wideband_both_sides() {
        let o = offer("10.45.0.7", 40000, 1, 1, true);
        let (a, n) = answer(&o, "10.45.0.9", 41000, 9, 1, &Caps::ALL).unwrap();
        assert_eq!(n, Negotiated { pt: 96, codec: Codec::AdpcmWb, rate: 16_000, dtmf_pt: Some(98), remote_addr: "10.45.0.7".into(), remote_port: 40000, direction: Direction::SendRecv, ptime: 20 });
        assert_eq!(a.media[0].formats, vec![96, 98]);
        assert_eq!(a.media[0].fmtp_for(98), Some("0-15"));
        rt(&a);
        let back = SessionDescription::parse(&a.to_bytes()).unwrap();
        let mine = accept_answer(&o, &back).unwrap();
        assert_eq!((mine.codec, mine.pt, mine.dtmf_pt, mine.remote_addr.as_str(), mine.remote_port), (Codec::AdpcmWb, 96, Some(98), "10.45.0.9", 41000));
    }

    #[test]
    fn narrowband_fallback() {
        // The far end only offers NB.
        let o = offer("10.45.0.7", 40000, 1, 1, false);
        let (a, n) = answer(&o, "10.45.0.9", 41000, 9, 1, &Caps::ALL).unwrap();
        assert_eq!((n.codec, n.pt, n.rate, n.dtmf_pt), (Codec::AdpcmNb, 97, 8000, Some(101)));
        assert_eq!(accept_answer(&o, &a).unwrap().codec, Codec::AdpcmNb);
        // We only do NB, they offer both.
        let o = offer("10.45.0.7", 40000, 1, 1, true);
        let (_, n) = answer(&o, "10.45.0.9", 41000, 9, 1, &Caps::NARROWBAND).unwrap();
        assert_eq!((n.codec, n.dtmf_pt), (Codec::AdpcmNb, Some(101)));
        // Foreign dynamic numbers and a non-matching DTMF clock.
        let foreign = b"v=0\r\no=carrier 5 5 IN IP4 192.0.2.1\r\ns=call\r\nt=0 0\r\nb=AS:41\r\na=recvonly\r\n\
m=audio 5004 RTP/AVP 0 111 110\r\nc=IN IP4 192.0.2.2\r\na=rtpmap:111 adpcm-nb/8000/1\r\na=rtpmap:110 telephone-event/16000\r\na=maxptime:40\r\n";
        let o = SessionDescription::parse(foreign).unwrap();
        assert_eq!(o.media[0].direction, Direction::RecvOnly);
        assert_eq!(o.media[0].attrs, vec!["maxptime:40".to_string()]);
        let (a, n) = answer(&o, "10.45.0.9", 41000, 9, 1, &Caps::ALL).unwrap();
        assert_eq!((n.pt, n.dtmf_pt, n.remote_addr.as_str(), n.remote_port, n.direction), (111, None, "192.0.2.2", 5004, Direction::SendOnly));
        assert_eq!(a.media[0].ptime, Some(20));
        rt(&o);
        rt(&a);
    }

    #[test]
    fn not_acceptable() {
        let o = offer("10.45.0.7", 40000, 1, 1, false);
        let none = Caps { wideband: true, narrowband: false, dtmf: true };
        let err = answer(&o, "10.45.0.9", 41000, 9, 1, &none).unwrap_err();
        assert_eq!((err.clone(), err.sip_code()), (SdpError::NotAcceptable, 488));
        let pcmu = SessionDescription::parse(b"v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nc=IN IP4 1.2.3.4\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0 8\r\n").unwrap();
        assert_eq!(answer(&pcmu, "a", 1, 1, 1, &Caps::ALL).unwrap_err(), SdpError::NotAcceptable);
        let mut held = offer("1.2.3.4", 0, 1, 1, true);
        held.media[0].port = 0;
        assert_eq!(answer(&held, "a", 1, 1, 1, &Caps::ALL).unwrap_err(), SdpError::NotAcceptable);
        // An answer with a codec we never offered.
        let (mut a, _) = answer(&offer("1.2.3.4", 5, 1, 1, true), "5.6.7.8", 6, 1, 1, &Caps::ALL).unwrap();
        assert_eq!(accept_answer(&offer("1.2.3.4", 5, 1, 1, false), &a).unwrap_err(), SdpError::NotAcceptable);
        a.media[0].port = 0;
        assert_eq!(accept_answer(&offer("1.2.3.4", 5, 1, 1, true), &a).unwrap_err(), SdpError::NotAcceptable);
        let mut no_c = offer("1.2.3.4", 5, 1, 1, true);
        no_c.connection = None;
        assert_eq!(answer(&no_c, "a", 1, 1, 1, &Caps::ALL).unwrap_err(), SdpError::Missing("c="));
    }

    #[test]
    fn other_streams_rejected() {
        let mut o = offer("10.45.0.7", 40000, 1, 1, true);
        let mut video = o.media[0].clone();
        video.kind = "video".into();
        video.formats = vec![99];
        o.media.insert(0, video);
        o.media.push(o.media[1].clone());
        let (a, n) = answer(&o, "10.45.0.9", 41000, 9, 1, &Caps::ALL).unwrap();
        assert_eq!(a.media.len(), 3);
        assert_eq!((a.media[0].port, a.media[1].port, a.media[2].port), (0, 41000, 0));
        assert_eq!(a.media[0].formats, vec![99]);
        assert_eq!(n.codec, Codec::AdpcmWb);
        assert_eq!(accept_answer(&o, &a).unwrap().pt, 96);
        rt(&a);
    }

    #[test]
    fn hold_directions() {
        let o = offer("10.45.0.7", 40000, 1, 1, true);
        let (a, _) = answer(&o, "10.45.0.9", 41000, 9, 1, &Caps::ALL).unwrap();
        let hold = o.with_direction(Direction::SendOnly);
        assert_eq!(hold.origin.sess_version, 2);
        let (ha, hn) = answer(&hold, "10.45.0.9", 41000, 9, 2, &Caps::ALL).unwrap();
        assert_eq!(hn.direction, Direction::RecvOnly);
        assert_eq!(ha.media[0].direction, Direction::RecvOnly);
        assert!(!hn.direction.sends() && hn.direction.receives());
        assert_eq!(accept_answer(&hold, &ha).unwrap().direction, Direction::SendOnly);
        let (_, n) = answer(&o.with_direction(Direction::Inactive), "x", 1, 1, 1, &Caps::ALL).unwrap();
        assert_eq!(n.direction, Direction::Inactive);
        let (_, n) = answer(&o.with_direction(Direction::RecvOnly), "x", 1, 1, 1, &Caps::ALL).unwrap();
        assert_eq!(n.direction, Direction::SendOnly);
        let resume = hold.with_direction(Direction::SendRecv);
        assert_eq!(resume.origin.sess_version, 3);
        assert_eq!(answer(&resume, "x", 1, 1, 1, &Caps::ALL).unwrap().1.direction, Direction::SendRecv);
        rt(&hold);
        // The answerer can hold too: its re-offer from its answer.
        let ahold = a.with_direction(Direction::SendOnly);
        assert_eq!(answer(&ahold, "y", 2, 1, 1, &Caps::ALL).unwrap().1.direction, Direction::RecvOnly);
    }

    #[test]
    fn parse_errors() {
        let ok = "v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nt=0 0\r\nm=audio 5 RTP/AVP 96\r\n";
        assert!(SessionDescription::parse(ok.as_bytes()).is_ok());
        let cases: &[(&str, SdpError)] = &[
            ("", SdpError::Line("v=0 first")),
            ("v=1\r\n", SdpError::Line("v=0 first")),
            ("v=0\r\ns=-\r\n", SdpError::Missing("o=")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\n", SdpError::Missing("s=")),
            ("v=0\r\no=- x 1 IN IP4 1.2.3.4\r\ns=-\r\n", SdpError::Value("o=")),
            ("v=0\r\no=- 1 1 IN IP6 1.2.3.4\r\ns=-\r\n", SdpError::Value("o=")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nc=IN IP4\r\n", SdpError::Value("c=")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP\r\n", SdpError::Value("m=")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 70000 RTP/AVP 0\r\n", SdpError::Value("m= port")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 200\r\n", SdpError::Value("payload type")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 0\r\na=rtpmap:96 X\r\n", SdpError::Value("rtpmap")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 0\r\na=rtpmap:96 X/0\r\n", SdpError::Value("rtpmap")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 0\r\na=fmtp:96\r\n", SdpError::Value("fmtp")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 0\r\na=ptime:0\r\n", SdpError::Value("ptime")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 0\r\na=ptime\r\n", SdpError::Value("attribute")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nm=audio 5 RTP/AVP 0\r\ns=x\r\n", SdpError::Line("session line after m=")),
            ("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=-\r\nbad\r\n", SdpError::Line("not x=value")),
            ("v=0\r\nv=0\r\n", SdpError::Line("second v=")),
        ];
        for (text, err) in cases {
            assert_eq!(SessionDescription::parse(text.as_bytes()).unwrap_err(), *err, "{text:?}");
        }
        assert_eq!(SessionDescription::parse(b"v=0\r\n\xff").unwrap_err(), SdpError::Utf8);
        // Bare LF and port/count are accepted.
        let lf = SessionDescription::parse(b"v=0\no=- 1 1 IN IP4 1.2.3.4\ns=-\nm=audio 5/2 RTP/AVP 0\n").unwrap();
        assert_eq!(lf.media[0].port, 5);
    }

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn garbage_never_panics() {
        let samples = [offer("10.45.0.7", 40000, 1, 1, true).to_bytes(), offer("fd00::1", 1, 2, 3, false).to_bytes()];
        let mut seed = 0xdead_beef_cafe_f00du64;
        let mut ok = 0;
        for i in 0..20_000 {
            let r = xorshift(&mut seed);
            let mut data = if i % 4 == 0 {
                (0..r % 200).map(|_| xorshift(&mut seed) as u8).collect::<Vec<u8>>()
            } else {
                samples[(r % 2) as usize].clone()
            };
            if i % 4 != 0 {
                for _ in 0..1 + r % 3 {
                    let k = xorshift(&mut seed);
                    if data.is_empty() {
                        break;
                    }
                    let pos = (k as usize >> 8) % data.len();
                    match k % 4 {
                        0 => data[pos] ^= 1 << (k >> 3 & 7),
                        1 => data[pos] = b" :/=\r\nam0-"[(k >> 5) as usize % 10],
                        2 => data.truncate(pos),
                        _ => data.insert(pos, (k >> 9) as u8),
                    }
                }
            }
            if let Ok(sd) = SessionDescription::parse(&data) {
                ok += 1;
                rt(&sd);
                if let Ok((a, _)) = answer(&sd, "10.45.0.9", 41000, 1, 1, &Caps::ALL) {
                    rt(&a);
                    accept_answer(&sd, &a).unwrap();
                }
                let _ = accept_answer(&sd, &sd);
                let _ = sd.with_direction(Direction::SendOnly);
            }
        }
        assert!(ok > 1000, "only {ok} mutated inputs parsed");
    }
}
