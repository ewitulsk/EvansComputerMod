//! `telephonyd`'s control socket: how Phone, Messages, Contacts, the `tel` and
//! `sms` commands and the Python / Lua `phone` module talk to the phone's IMS
//! stack (apps never touch the network themselves).
//!
//! TCP on `127.0.0.1:7720`, framed like the cellular daemons' admin sockets
//! (`ecm_cell::admin`): one request line, `ok` / `err <why>`, body lines, `.`.
//! A connection that sends `watch` gets `ok` then event lines, one per event,
//! until it closes. Text fields are percent-encoded ([`enc`]).
//!
//! | Request | Reply body |
//! |---|---|
//! | `status` | `key value`: `registration msisdn domain carrier bars volte hd voicemail unread missed` |
//! | `calls` | `call <id> <state> <number> <hidden 0/1> <outgoing 0/1> <seconds> <muted> <hd> <codec> <loss%> <merged>` |
//! | `dial <number> [hide]` | `call <id>` |
//! | `answer <id>`, `hangup <id>`, `decline <id>`, `hold <id> on\|off`, `swap`, `merge`, `mute <id> on\|off` | |
//! | `dtmf <id> <digits>` | keypad tones (RFC 4733) |
//! | `play <id> <wav path>` | the file goes to the far end instead of the microphone (IVRs, scripts) |
//! | `record <id> <seconds> <wav path>` | the far end's audio into a file |
//! | `speaker on\|off` | loudspeaker (echo guard on) or earpiece |
//! | `sms <to> <text>` | `sms <id>` |
//! | `mms <to,to,...> <text\|-> <file,file\|->` | `mms <id>`; several numbers make a group |
//! | `threads` | `thread <key> <unread> <last time> <last text>` newest first |
//! | `messages <key> [n]` | `msg <id> <dir in\|out> <from> <time> <status> <kind sms\|mms> <text> <files>` |
//! | `read <key>`, `delete <id>`, `delete-thread <key>` | |
//! | `log [n]` | the call log: `call <id> <dir in\|out\|missed> <number> <start> <seconds> <reason>` |
//! | `voicemail` | `vm <id> <from> <time> <ms> <new\|heard>` (visual voicemail) |
//! | `voicemail-fetch <id> <path>` | saves the message as a WAV |
//! | `voicemail-heard <id>`, `voicemail-delete <id>` | |
//! | `forward all <number>` / `forward off` | (sends `*72` / `*73`) |
//! | `watch` | events (below) |
//!
//! Events: `incoming call=<id> from=<number> hidden=0|1 waiting=0|1`,
//! `call call=<id> state=<state> reason=<text>`, `sms id=<id> from=<number> thread=<key> text=<text>`,
//! `mms id=<id> from=<number> thread=<key> text=<text> files=<a,b>`, `sent id=<id>`, `delivered id=<id>`,
//! `failed id=<id> why=<text>`, `dtmf call=<id> digit=<d>`, `voicemail waiting=<n>`, `registration state=<s> msisdn=<n>`.

/// Everything (an app needs the `phone` permission to connect).
pub const PORT: u16 = 7720;
/// Texts only (`sms`): `status`, `sms`, `mms`, `threads`, `messages`, `read`, `delete`,
/// `delete-thread`, and `watch` with the message events.
pub const SMS_PORT: u16 = 7726;

/// Requests the texts-only port takes.
pub fn sms_port_allows(line: &str) -> bool {
    matches!(line.split_whitespace().next().unwrap_or(""), "status" | "sms" | "mms" | "threads" | "messages" | "read" | "delete" | "delete-thread" | "watch")
}

/// Events the texts-only port's watchers get.
pub fn sms_event(name: &str) -> bool {
    matches!(name, "sms" | "mms" | "sent" | "delivered" | "failed" | "registration")
}

/// Percent-encode a text field (space, `%`, controls and non-ASCII).
pub fn enc(s: &str) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_graphic() && b != b'%' && b != b',' && b != b'=' {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    if o == "-" {
        return "%2D".into();
    }
    o
}

pub fn dec(s: &str) -> String {
    if s == "-" {
        return String::new();
    }
    let b = s.as_bytes();
    let mut o = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                o.push(x);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

/// One event line: its name and `key=value` fields (values decoded).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub name: String,
    pub fields: Vec<(String, String)>,
}

impl Event {
    pub fn new(name: &str, fields: &[(&str, &str)]) -> Event {
        Event { name: name.into(), fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }
    pub fn get(&self, k: &str) -> &str {
        self.fields.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str()).unwrap_or("")
    }
    pub fn line(&self) -> String {
        let mut s = self.name.clone();
        for (k, v) in &self.fields {
            s.push(' ');
            s.push_str(k);
            s.push('=');
            s.push_str(&enc(v));
        }
        s
    }
    pub fn parse(l: &str) -> Option<Event> {
        let mut w = l.split_whitespace();
        let name = w.next()?.to_string();
        let fields = w.filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), dec(v)))).collect();
        Some(Event { name, fields })
    }
}

/// A call as `calls` lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallLine {
    pub id: u32,
    pub state: String,
    pub number: String,
    pub hidden: bool,
    pub outgoing: bool,
    pub seconds: u64,
    pub muted: bool,
    pub hd: bool,
    pub codec: String,
    pub loss: f32,
    pub merged: bool,
}

impl CallLine {
    pub fn line(&self) -> String {
        format!(
            "call {} {} {} {} {} {} {} {} {} {:.1} {}",
            self.id,
            self.state,
            enc(&self.number),
            self.hidden as u8,
            self.outgoing as u8,
            self.seconds,
            self.muted as u8,
            self.hd as u8,
            enc(&self.codec),
            self.loss,
            self.merged as u8
        )
    }
    pub fn parse(l: &str) -> Option<CallLine> {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() != 12 || w[0] != "call" {
            return None;
        }
        Some(CallLine {
            id: w[1].parse().ok()?,
            state: w[2].into(),
            number: dec(w[3]),
            hidden: w[4] == "1",
            outgoing: w[5] == "1",
            seconds: w[6].parse().unwrap_or(0),
            muted: w[7] == "1",
            hd: w[8] == "1",
            codec: dec(w[9]),
            loss: w[10].parse().unwrap_or(0.0),
            merged: w[11] == "1",
        })
    }
}

/// A message as `messages` lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MsgLine {
    pub id: u64,
    pub incoming: bool,
    pub from: String,
    pub time_ms: u64,
    /// `sending`, `sent`, `delivered`, `failed:<why>`, `received`, `read`.
    pub status: String,
    pub mms: bool,
    pub text: String,
    pub files: Vec<String>,
}

impl MsgLine {
    pub fn line(&self) -> String {
        format!(
            "msg {} {} {} {} {} {} {} {}",
            self.id,
            if self.incoming { "in" } else { "out" },
            enc(&self.from),
            self.time_ms,
            enc(&self.status),
            if self.mms { "mms" } else { "sms" },
            enc(&self.text),
            if self.files.is_empty() { "-".into() } else { self.files.iter().map(|f| enc(f)).collect::<Vec<_>>().join(",") }
        )
    }
    pub fn parse(l: &str) -> Option<MsgLine> {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() != 9 || w[0] != "msg" {
            return None;
        }
        Some(MsgLine {
            id: w[1].parse().ok()?,
            incoming: w[2] == "in",
            from: dec(w[3]),
            time_ms: w[4].parse().unwrap_or(0),
            status: dec(w[5]),
            mms: w[6] == "mms",
            text: dec(w[7]),
            files: if w[8] == "-" { Vec::new() } else { w[8].split(',').map(dec).collect() },
        })
    }
}

/// A conversation as `threads` lists it. The key is the other numbers, sorted, joined by `,`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreadLine {
    pub key: String,
    pub unread: u32,
    pub time_ms: u64,
    pub preview: String,
}

impl ThreadLine {
    pub fn line(&self) -> String {
        format!("thread {} {} {} {}", self.key, self.unread, self.time_ms, enc(&self.preview))
    }
    pub fn parse(l: &str) -> Option<ThreadLine> {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() != 5 || w[0] != "thread" {
            return None;
        }
        Some(ThreadLine { key: w[1].into(), unread: w[2].parse().unwrap_or(0), time_ms: w[3].parse().unwrap_or(0), preview: dec(w[4]) })
    }
    pub fn members(&self) -> Vec<String> {
        self.key.split(',').map(String::from).collect()
    }
}

/// A call log entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogLine {
    pub id: u64,
    /// `in`, `out`, `missed`.
    pub dir: String,
    pub number: String,
    pub start_ms: u64,
    pub seconds: u64,
    pub reason: String,
}

impl LogLine {
    pub fn line(&self) -> String {
        format!("call {} {} {} {} {} {}", self.id, self.dir, enc(&self.number), self.start_ms, self.seconds, enc(&self.reason))
    }
    pub fn parse(l: &str) -> Option<LogLine> {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() != 7 || w[0] != "call" {
            return None;
        }
        Some(LogLine { id: w[1].parse().ok()?, dir: w[2].into(), number: dec(w[3]), start_ms: w[4].parse().unwrap_or(0), seconds: w[5].parse().unwrap_or(0), reason: dec(w[6]) })
    }
}

/// `key value` from a status body.
pub fn value<'a>(body: &'a [String], k: &str) -> &'a str {
    body.iter().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(' '))).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        for t in ["", "hi bo, it's alex", "50% = half\nok", "-", "héllo, wörld"] {
            assert_eq!(dec(&enc(t)), t, "{:?}", t);
            assert!(!enc(t).contains(' '));
        }
        let e = Event::new("sms", &[("from", "+12145550142"), ("text", "hi there")]);
        assert_eq!(Event::parse(&e.line()), Some(e.clone()));
        assert_eq!(e.get("text"), "hi there");
        let c = CallLine { id: 3, state: "active".into(), number: "+12145550143".into(), hd: true, codec: "ADPCM-WB".into(), seconds: 12, ..Default::default() };
        assert_eq!(CallLine::parse(&c.line()), Some(c));
        let m = MsgLine { id: 9, incoming: true, from: "+1".into(), time_ms: 5, status: "failed:the phone is off".into(), mms: true, text: "a b".into(), files: vec!["/home/Messages/9/p.png".into()] };
        assert_eq!(MsgLine::parse(&m.line()), Some(m));
        let t = ThreadLine { key: "+12145550143,+12145550144".into(), unread: 2, time_ms: 7, preview: "pizza?".into() };
        assert_eq!(ThreadLine::parse(&t.line()).unwrap().members().len(), 2);
        let l = LogLine { id: 1, dir: "missed".into(), number: "+1".into(), start_ms: 3, seconds: 0, reason: "no answer".into() };
        assert_eq!(LogLine::parse(&l.line()), Some(l));
    }
}
